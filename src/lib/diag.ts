/**
 * Lightweight performance diagnostics for the streaming UI.
 *
 * Why this exists: with several sessions streaming at once, the JS main thread
 * can saturate (markdown parsing + full re-render on every chunk), making the
 * window look frozen — scrolling still works (compositor-driven) but buttons
 * never receive events. Instead of guessing, this module records concrete
 * numbers: event rate, per-event handler cost, render/markdown cost, and long
 * tasks (tasks ≥ 50ms that block the main thread and eat click events).
 *
 * Usage:
 *   - Toggle the on-screen overlay: Ctrl+Shift+D (Cmd+Shift+D on macOS)
 *   - From the console: window.__runjamDiag.enable() / .disable() / .reset()
 *   - While enabled, a one-line summary is printed to the console every 5s.
 */

interface DiagState {
  enabled: boolean;
  /** Total ACP events handled (per session). */
  events: number;
  /** Total streamed payload bytes observed. */
  eventBytes: number;
  /** Cumulative time spent inside handleAcpEvent (ms). */
  handlerMs: number;
  maxHandlerMs: number;
  /** Sampled renderContent() calls and cumulative cost. */
  renderCalls: number;
  renderMs: number;
  /** Markdown parses (cache misses) and cumulative cost. */
  mdParses: number;
  mdParseMs: number;
  /** Long tasks (main-thread blocks ≥ 50ms) counted by PerformanceObserver. */
  longTasks: number;
  maxLongTaskMs: number;
  /** Rolling window for events-per-second. */
  windowStart: number;
  windowEvents: number;
  lastSummaryAt: number;
}

const s: DiagState = {
  enabled: false,
  events: 0,
  eventBytes: 0,
  handlerMs: 0,
  maxHandlerMs: 0,
  renderCalls: 0,
  renderMs: 0,
  mdParses: 0,
  mdParseMs: 0,
  longTasks: 0,
  maxLongTaskMs: 0,
  windowStart: performance.now(),
  windowEvents: 0,
  lastSummaryAt: performance.now(),
};

let overlayEl: HTMLElement | null = null;
let overlayTimer: number | null = null;
let consoleTimer: number | null = null;
let renderSampleCounter = 0;
let longTaskObs: PerformanceObserver | null = null;
let lastLongTaskWarnAt = 0;

const OVERLAY_ID = "__runjam_diag_overlay__";

export function initDiag(): void {
  if (typeof window === "undefined") return;
  if ((window as any).__runjamDiag) return;

  if (typeof PerformanceObserver !== "undefined") {
    try {
      longTaskObs = new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) {
          s.longTasks++;
          const d = Math.round(entry.duration);
          if (d > s.maxLongTaskMs) s.maxLongTaskMs = d;
          // A task of 100ms+ is a freeze the user would actually feel while
          // streaming — surface it immediately instead of waiting for the 5s summary.
          const now = performance.now();
          if (d >= 100 && now - lastLongTaskWarnAt > 3000) {
            lastLongTaskWarnAt = now;
            console.warn(
              `[DIAG] Long task detected: ${d}ms — main thread blocked (this is what makes the UI unresponsive while sessions stream).`,
            );
          }
        }
      });
      longTaskObs.observe({ entryTypes: ["longtask"] });
    } catch {
      longTaskObs = null;
    }
  }

  window.addEventListener("keydown", onKeydown);
  (window as any).__runjamDiag = {
    enable: enableDiag,
    disable: disableDiag,
    toggle: toggleDiag,
    reset: resetDiag,
    stats: () => ({ ...s }),
  };
}

function onKeydown(e: KeyboardEvent): void {
  if ((e.ctrlKey || e.metaKey) && e.shiftKey && (e.key === "D" || e.key === "d")) {
    e.preventDefault();
    toggleDiag();
  }
}

export function toggleDiag(): void {
  if (s.enabled) disableDiag();
  else enableDiag();
}

export function enableDiag(): void {
  if (s.enabled) return;
  s.enabled = true;
  console.log("[DIAG] Enabled — toggle with Ctrl/Cmd+Shift+D or window.__runjamDiag.disable()");
  renderOverlay();
  overlayTimer = window.setInterval(renderOverlay, 500);
  consoleTimer = window.setInterval(printSummary, 5000);
}

export function disableDiag(): void {
  s.enabled = false;
  if (overlayTimer !== null) { clearInterval(overlayTimer); overlayTimer = null; }
  if (consoleTimer !== null) { clearInterval(consoleTimer); consoleTimer = null; }
  if (overlayEl) { overlayEl.remove(); overlayEl = null; }
  console.log("[DIAG] Disabled");
}

export function resetDiag(): void {
  s.events = 0; s.eventBytes = 0; s.handlerMs = 0; s.maxHandlerMs = 0;
  s.renderCalls = 0; s.renderMs = 0; s.mdParses = 0; s.mdParseMs = 0;
  s.longTasks = 0; s.maxLongTaskMs = 0;
  s.windowStart = performance.now(); s.windowEvents = 0;
}

/** Called once per ACP event handled (see SessionView.handleAcpEvent). */
export function recordEvent(ms: number, bytes = 0): void {
  s.events++;
  s.windowEvents++;
  s.eventBytes += bytes;
  s.handlerMs += ms;
  if (ms > s.maxHandlerMs) s.maxHandlerMs = ms;
}

/** Called from renderContent — sampled so the measurement itself stays cheap. */
export function recordRender(ms: number): void {
  if ((++renderSampleCounter & 15) === 0) {
    s.renderCalls++;
    s.renderMs += ms;
  }
}

/** Called on markdown cache misses (actual parses). */
export function recordMdParse(ms: number): void {
  s.mdParses++;
  s.mdParseMs += ms;
}

function renderOverlay(): void {
  const now = performance.now();
  const elapsed = Math.max(1, now - s.windowStart);
  const eps = ((s.windowEvents / elapsed) * 1000).toFixed(1);
  // Reset the rate window every refresh so EPS is current, not cumulative.
  s.windowStart = now;
  s.windowEvents = 0;

  const kb = (s.eventBytes / 1024).toFixed(0);
  const avgHandler = s.events ? (s.handlerMs / s.events).toFixed(2) : "0";
  const avgRender = s.renderCalls ? (s.renderMs / s.renderCalls).toFixed(2) : "0";
  const avgMd = s.mdParses ? (s.mdParseMs / s.mdParses).toFixed(2) : "0";
  const rows = [
    `events: ${s.events}  (${eps}/s, ${kb} KB)`,
    `handler: avg ${avgHandler}ms / max ${s.maxHandlerMs.toFixed(1)}ms`,
    `renderContent (sampled): ${s.renderCalls} calls, avg ${avgRender}ms`,
    `markdown parses: ${s.mdParses}, avg ${avgMd}ms`,
    `long tasks: ${s.longTasks}  (max ${Math.round(s.maxLongTaskMs)}ms)`,
    `session: ${document.title}`,
  ];
  const html = rows.map((r) => `<div>${escapeHtml(r)}</div>`).join("");

  if (!overlayEl || !document.body.contains(overlayEl)) {
    overlayEl = document.createElement("div");
    overlayEl.id = OVERLAY_ID;
    overlayEl.style.cssText = [
      "position:fixed", "top:8px", "right:8px", "z-index:99999",
      "background:rgba(0,0,0,0.82)", "color:#7ee787", "font:11px/1.5 ui-monospace,Menlo,Consolas,monospace",
      "padding:8px 10px", "border-radius:6px", "pointer-events:none",
      "white-space:pre", "max-width:360px",
    ].join(";");
    document.body.appendChild(overlayEl);
  }
  overlayEl.innerHTML = `<div style="font-weight:bold;color:#ffd33d">RUNJAM DIAG (Ctrl/Cmd+Shift+D to hide)</div>${html}`;
}

function printSummary(): void {
  const now = performance.now();
  const dt = (now - s.lastSummaryAt) / 1000;
  s.lastSummaryAt = now;
  const eps = dt > 0 ? (s.windowEvents / dt).toFixed(1) : "0";
  s.windowStart = now;
  s.windowEvents = 0;
  const avgHandler = s.events ? (s.handlerMs / s.events).toFixed(2) : "0";
  const avgRender = s.renderCalls ? (s.renderMs / s.renderCalls).toFixed(2) : "0";
  console.log(
    `[DIAG] events=${s.events} (${eps}/s) handlerAvg=${avgHandler}ms ` +
      `renderAvg=${avgRender}ms mdParses=${s.mdParses} longTasks=${s.longTasks} (max ${Math.round(s.maxLongTaskMs)}ms)`,
  );
}

function escapeHtml(str: string): string {
  return str.replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!,
  );
}
