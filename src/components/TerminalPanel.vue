<script setup lang="ts">
import { ref, onBeforeUnmount, nextTick, watch } from "vue";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { X, Trash2, Plus, TerminalIcon } from "lucide-vue-next";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useToast } from "../composables/useToast";

const props = defineProps<{
  cwd?: string;
  /** Whether the terminal panel is currently visible. When false, the panel is
   *  kept mounted (v-show) but does NOT spawn a backend shell — spawning is
   *  deferred until the terminal is first shown. This avoids auto-launching an
   *  interactive shell for every session on app start. */
  active?: boolean;
}>();

const emit = defineEmits<{
  (e: "close"): void;
}>();

// ---- Tab state ----
interface TabState {
  id: number;
  title: string;
  cwd: string;
  term: Terminal | null;
  fitAddon: FitAddon | null;
  unlisten: UnlistenFn | null;
  resizeObserver: ResizeObserver | null;
  /** Buffer text captured when the directory was switched away. Written into
   *  the new Terminal on mount before the event listener is attached. */
  pendingBuffer?: string;
}

const tabs = ref<TabState[]>([]);
const activeTabIndex = ref(-1);
let tabCounter = 0;

// Container refs — keyed by terminal id
const containerEls = ref<Record<number, HTMLElement | null>>({});
const tabsScrollEl = ref<HTMLElement | null>(null);

// ═══════════════════════════════════════════════════
// Module-level terminal persistence (per directory)
// ═══════════════════════════════════════════════════

interface SavedTab {
  id: number;
  title: string;
  cwd: string;
  /** Captured xterm buffer text at save time. Restored into the new Terminal
   *  when this directory is shown again — without this, switching sessions
   *  destroyed the xterm buffer (term.dispose()) and the historical output
   *  was lost even though the backend shell process stayed alive. */
  bufferText?: string;
}

interface SavedDirectoryState {
  tabs: SavedTab[];
  activeIndex: number;
  counter: number;
}

const directoryStates = new Map<string, SavedDirectoryState>();

/** Write a large string to a terminal in chunks, letting xterm process each
 *  chunk on its own frame. A single synchronous write of a large scrollback
 *  would block the main thread for the whole write; chunking keeps the UI
 *  responsive while restoring history. Calls `onDone` when all chunks have been
 *  written (or immediately if the text is empty). */
function writeChunked(term: Terminal, text: string, onDone?: () => void, chunkSize = 8192) {
  let offset = 0;
  const writeNext = () => {
    if (offset >= text.length) {
      onDone?.();
      return;
    }
    const chunk = text.slice(offset, offset + chunkSize);
    offset += chunkSize;
    term.write(chunk, writeNext);
  };
  writeNext();
}

/** Export a terminal's visible buffer (scrollback + current screen) as text.
 *  xterm has no direct serialization API, so we walk the buffer lines and
 *  join them. Wrapped lines are joined without a newline to preserve the
 *  original line structure. */
function captureBufferText(term: Terminal): string {
  const buffer = term.buffer.active;
  const lines: string[] = [];
  // Walk from the top of the buffer (including scrollback) to the bottom.
  for (let y = 0; y < buffer.length; y++) {
    const line = buffer.getLine(y);
    if (!line) continue;
    // A wrapped line continues the previous logical line. For the FIRST segment
    // of a wrapped logical line (isWrapped === false but the next line wraps),
    // trimRight would drop a legitimate trailing space ("foo " + "bar" → "foobar").
    // So keep trailing whitespace on any line that is itself wrapped OR is
    // followed by a wrapped continuation; trim only standalone lines.
    const next = buffer.getLine(y + 1);
    const isPartOfWrapped = line.isWrapped || (next?.isWrapped === true);
    const text = line.translateToString(!isPartOfWrapped);
    if (line.isWrapped && lines.length > 0) {
      lines[lines.length - 1] += text;
    } else {
      lines.push(text);
    }
  }
  // Join with \r\n (carriage return + line feed), not just \n — terminals
  // require CR to return to column 0 before the LF. Writing back with only \n
  // makes xterm advance lines without returning the cursor, so the restored
  // history appears misaligned/jumbled.
  return lines.join("\r\n").replace(/\r\n+$/, "");
}

/** Save current tabs metadata (incl. buffer text) for a directory WITHOUT
 *  killing backend processes */
function saveDirectoryState(cwd: string) {
  if (!cwd || tabs.value.length === 0) return;
  directoryStates.set(cwd, {
    tabs: tabs.value.map((t) => ({
      id: t.id,
      title: t.title,
      cwd: t.cwd,
      // Capture the live term's buffer if it's mounted; otherwise fall back to
      // any pendingBuffer that hasn't been written yet (mountTerminal may not
      // have run). This keeps history across a rapid switch-before-mount.
      bufferText: t.term ? captureBufferText(t.term) : t.pendingBuffer,
    })),
    activeIndex: activeTabIndex.value,
    counter: tabCounter,
  });
}

/** Restore tabs for a directory. Returns null if no saved state. Event
 *  listeners are NOT established here — they're set up in mountTerminal after
 *  the buffer text is written, so restored history arrives before any new
 *  backend output. */
async function restoreDirectoryState(cwd: string): Promise<{
  restoredTabs: TabState[];
  restoredIndex: number;
  restoredCounter: number;
} | null> {
  const saved = directoryStates.get(cwd);
  if (!saved || saved.tabs.length === 0) return null;

  const restoredTabs: TabState[] = [];
  for (const st of saved.tabs) {
    const tab: TabState = {
      id: st.id,
      title: st.title,
      cwd: st.cwd,
      term: null,
      fitAddon: null,
      unlisten: null,
      resizeObserver: null,
      pendingBuffer: st.bufferText,
    };
    restoredTabs.push(tab);
  }

  return {
    restoredTabs,
    restoredIndex: saved.activeIndex,
    restoredCounter: saved.counter,
  };
}

/** Dispose xterm DOM resources for a tab (NOT the backend process, NOT the event listener) */
function disposeTabDOM(tab: TabState) {
  tab.resizeObserver?.disconnect();
  tab.resizeObserver = null;
  tab.term?.dispose();
  tab.term = null;
  tab.fitAddon = null;
}

/** Full dispose (including event listener) — used when tab is explicitly closed */
function disposeTabFull(tab: TabState) {
  tab.unlisten?.();
  tab.unlisten = null;
  disposeTabDOM(tab);
}

/** Save current tabs, then dispose all DOM + listeners */
function teardownCurrentTabs() {
  for (const tab of tabs.value) {
    disposeTabFull(tab);
  }
}

// ═══════════════════════════════════════════════════
// Terminal operations
// ═══════════════════════════════════════════════════

function handleClear() {
  activeTab()?.term?.clear();
}

function activeTab(): TabState | undefined {
  return tabs.value[activeTabIndex.value];
}

async function createTab(): Promise<TabState> {
  const workDir = props.cwd || null;
  const termId = await invoke<number>("spawn_terminal", { cwd: workDir });
  tabCounter++;

  // Event listener is attached in mountTerminal (after any pending buffer is
  // written), so a fresh shell's first output isn't dropped before the term exists.
  return {
    id: termId,
    title: `sh-${tabCounter}`,
    cwd: workDir || "",
    term: null,
    fitAddon: null,
    unlisten: null,
    resizeObserver: null,
  };
}

/** Create a new tab and mount it. Accepts the init generation (from
 *  `initForCwd`) so a spawn that completes AFTER a newer session switch can
 *  abandon itself: it kills the freshly-spawned shell instead of pushing a
 *  zombie tab for the wrong directory (which previously made the terminal show
 *  a fresh shell / lost history after a rapid session switch). Called without
 *  an argument from the "+" button, where the user action is always valid. */
async function addTab(myGen?: number): Promise<TabState | null> {
  const tab = await createTab();
  if (myGen !== undefined && myGen !== initGeneration) {
    // Superseded while the shell was spawning — kill it, don't mount it.
    invoke("kill_terminal", { terminalId: tab.id }).catch(() => {});
    return null;
  }
  tabs.value.push(tab);
  activeTabIndex.value = tabs.value.length - 1;
  spawnedCwds.add(tab.cwd);
  await nextTick();
  if (myGen !== undefined && myGen !== initGeneration) {
    // Superseded during the frame — undo the push so no zombie tab survives.
    tabs.value = tabs.value.filter((t) => t.id !== tab.id);
    activeTabIndex.value = tabs.value.length - 1;
    spawnedCwds.delete(tab.cwd);
    invoke("kill_terminal", { terminalId: tab.id }).catch(() => {});
    return null;
  }
  mountTerminal(tab, myGen).catch((err) => {
    console.error("Failed to mount terminal:", err);
  });
  return tab;
}

/** Kill every backend terminal process and clear all local state for the
 *  current directory. Used when the user confirms closing the whole terminal
 *  panel (top-right toggle) — the processes are actually terminated, and the
 *  saved state is dropped so the next open spawns a fresh shell. */
async function killAll() {
  for (const tab of tabs.value) {
    invoke("kill_terminal", { terminalId: tab.id }).catch(() => {});
    disposeTabFull(tab);
  }
  tabs.value = [];
  activeTabIndex.value = -1;
  if (props.cwd) {
    directoryStates.delete(props.cwd);
    spawnedCwds.delete(props.cwd);
  }
}

function closeTab(index: number) {
  const tab = tabs.value[index];
  if (!tab) return;

  // Kill the backend process — this is an explicit user action
  invoke("kill_terminal", { terminalId: tab.id }).catch(() => {});
  disposeTabFull(tab);

  tabs.value.splice(index, 1);

  // Drop this tab from the directory's saved state so a later restore can't
  // resurrect a tab whose backend shell was just killed.
  const saved = directoryStates.get(tab.cwd);
  if (saved) {
    saved.tabs = saved.tabs.filter((t) => t.id !== tab.id);
    if (saved.tabs.length === 0) directoryStates.delete(tab.cwd);
  }

  if (tabs.value.length === 0) {
    activeTabIndex.value = -1;
  } else if (activeTabIndex.value >= tabs.value.length) {
    activeTabIndex.value = tabs.value.length - 1;
  }

  if (activeTabIndex.value >= 0) {
    nextTick(() => tabs.value[activeTabIndex.value]?.fitAddon?.fit());
  }
}

function switchTab(index: number) {
  if (index === activeTabIndex.value) return;
  activeTabIndex.value = index;
  nextTick(() => {
    const tab = tabs.value[index];
    if (!tab) return;
    if (tab.term) {
      setTimeout(() => tab.fitAddon?.fit(), 50);
    } else if (!tab.unlisten) {
      // A restored tab that hasn't been mounted yet (on restore only the active
      // tab is mounted). Mount it lazily now that it's the visible tab — this
      // writes its captured history back into a fresh xterm. Passing the
      // current generation lets mountTerminal abandon itself (and unlisten) if
      // a session switch happens while the listener is being attached.
      mountTerminal(tab, initGeneration).catch((err) => {
        console.error("Failed to mount terminal:", err);
      });
    }
  });
}

async function mountTerminal(tab: TabState, myGen?: number) {
  const el = containerEls.value[tab.id];
  if (!el) return;

  const term = new Terminal({
    fontSize: 13,
    fontFamily:
      "JetBrains Mono, Fira Code, Cascadia Code, SF Mono, Menlo, monospace",
    theme: {
      background: "#0d1117",
      foreground: "#c9d1d9",
      cursor: "#58a6ff",
      cursorAccent: "#0d1117",
      selectionBackground: "#264f78",
      black: "#484f58",
      red: "#ff7b72",
      green: "#3fb950",
      yellow: "#d29922",
      blue: "#58a6ff",
      magenta: "#bc8cff",
      cyan: "#39c5cf",
      white: "#b1bac4",
      brightBlack: "#6e7681",
      brightRed: "#ffa198",
      brightGreen: "#56d364",
      brightYellow: "#e3b341",
      brightBlue: "#79c0ff",
      brightMagenta: "#d2a8ff",
      brightCyan: "#56d4dd",
      brightWhite: "#f0f6fc",
    },
    cursorBlink: true,
    cursorStyle: "bar",
    cursorWidth: 2,
    scrollback: 5000,
    allowProposedApi: true,
    smoothScrollDuration: 0,
    drawBoldTextInBrightColors: true,
    macOptionIsMeta: true,
  });

  const fitAddon = new FitAddon();
  term.loadAddon(fitAddon);
  term.open(el);

  term.onData((data) => {
    const bytes = new TextEncoder().encode(data);
    invoke("write_terminal", {
      terminalId: tab.id,
      data: Array.from(bytes),
    }).catch(() => {});
  });

  // Keep the backend PTY's size in sync with the rendered xterm. Without this
  // the shell stays at 80x24 and long lines wrap at 80 columns, causing
  // re-wrap/re-render jank on every fit.
  term.onResize(({ cols, rows }) => {
    invoke("resize_terminal", { terminalId: tab.id, rows, cols }).catch(() => {});
  });

  tab.term = term;
  tab.fitAddon = fitAddon;

  // ── History restore + listener ordering ────────────────────────────
  // We must write the captured history BEFORE any new backend output, or the
  // restored text would be interleaved with fresh output. But we also want the
  // listener attached as early as possible so a freshly spawned shell's prompt
  // isn't dropped. Solution: attach the listener immediately, but buffer any
  // incoming output until the history has finished writing (historyDone), then
  // flush the buffer. This preserves ordering AND doesn't drop early output.
  let historyDone = true; // recomputed after the pending drain below
  const pendingOutput: (string | Uint8Array)[] = [];

  if (!tab.unlisten) {
    tab.unlisten = await listen<number[] | string>(
      `terminal-data-${tab.id}`,
      (event) => {
        if (!tab.term) return;
        const payload = event.payload;
        const data = typeof payload === "string" ? payload : new Uint8Array(payload);
        if (!historyDone) {
          // History still being written — hold this output until it's flushed.
          pendingOutput.push(data);
          return;
        }
        tab.term.write(data);
      }
    );
  }

  // Drain PTY output the backend buffered BEFORE the listener attached. Called
  // after listen() so nothing can fall between the drain and the listener: any
  // output produced from here on flows through the event channel above. Fixes
  // lost history when a session switch races the initial xterm mount (the tab
  // had no term yet, so saveDirectoryState couldn't capture its buffer).
  let pendingData: Uint8Array | undefined;
  try {
    const raw = await invoke<number[] | null>("take_terminal_pending", {
      terminalId: tab.id,
    });
    if (raw && raw.length) pendingData = new Uint8Array(raw);
  } catch {
    // Terminal already killed on the backend — nothing to restore.
  }

  // If this tab was superseded by a newer session switch while we awaited the
  // listener (or the pending drain), tear down what we just set up (the tab is
  // no longer current).
  if (myGen !== undefined && myGen !== initGeneration) {
    // The tab may already have been disposed by teardownCurrentTabs. If term is
    // still alive, dispose it; always unlisten to avoid leaking the listener.
    if (tab.term === term) {
      tab.term = null;
      tab.fitAddon = null;
      term.dispose();
    }
    tab.unlisten?.();
    tab.unlisten = null;
    return;
  }

  // Write captured history (chunked so a large scrollback doesn't block the
  // main thread): first the saved buffer (if any), then the pre-mount PTY
  // output drained above, then flush any output that arrived during the write.
  const savedHistory = tab.pendingBuffer;
  tab.pendingBuffer = undefined;
  const hasHistory = !!savedHistory || !!pendingData;
  historyDone = !hasHistory;
  const flushPendingOutput = () => {
    historyDone = true;
    if (tab.term !== term) return;
    for (const d of pendingOutput) term.write(d);
    pendingOutput.length = 0;
  };
  if (hasHistory) {
    // Guard: the tab may have been disposed by teardownCurrentTabs while we
    // awaited the listener (lazy-mount via switchTab has no caller generation
    // bump) — writing into a disposed xterm would throw.
    if (tab.term !== term) return;
    if (savedHistory) {
      writeChunked(term, savedHistory, () => {
        if (tab.term !== term) return;
        if (pendingData) term.write(pendingData, flushPendingOutput);
        else flushPendingOutput();
      });
    } else if (pendingData) {
      term.write(pendingData, flushPendingOutput);
    } else {
      flushPendingOutput();
    }
  }

  // fit() must only run while this tab's term is still the live one — after a
  // session switch the tab may be disposed, and fit() on a disposed terminal
  // throws. Guard every fit call against tab.term having been swapped/disposed.
  const safelyFit = () => {
    if (tab.term === term) fitAddon.fit();
  };

  setTimeout(safelyFit, 150);

  let fitTimer: ReturnType<typeof setTimeout> | null = null;
  tab.resizeObserver = new ResizeObserver(() => {
    // Debounce: xterm.fit() is synchronous and expensive.
    // During sidebar/panel resize animations, the observer fires on every
    // frame — calling fit() each time blocks the main thread and causes jank.
    if (fitTimer) clearTimeout(fitTimer);
    fitTimer = setTimeout(() => { safelyFit(); fitTimer = null; }, 80);
  });
  tab.resizeObserver.observe(el);
}

// ═══════════════════════════════════════════════════
// Directory-switch logic — persist per-directory
// ═══════════════════════════════════════════════════

// ═══════════════════════════════════════════════════
// Lazy initialization — spawn/restore only when visible
// ═══════════════════════════════════════════════════

// Directories that have had a shell spawned (via addTab). Used to avoid
// re-spawning a fresh interactive shell every time the panel is toggled open —
// spawning is one of the expensive steps that made toggling the terminal freeze.
// A directory's saved state (directoryStates) is restored first whenever we
// re-show it, so an already-spawned process is reused, not duplicated.
const spawnedCwds = new Set<string>();

// Generation token for async init. initForCwd awaits restoreDirectoryState
// (which awaits listen() per tab) and addTab (which awaits spawn_terminal).
// Under rapid session switching these in-flight awaits can complete out of
// order, letting a stale directory's restore overwrite the current directory's
// tabs. Bumping the token on every switch lets each init check whether it's
// still the latest before applying its result — stale inits abandon themselves.
let initGeneration = 0;

// One-time hint when the backend runs terminals in lightweight shell mode (the
// user's rc config is heavy, e.g. oh-my-zsh/p10k). Tell them why the terminal
// looks bare so it isn't mistaken for a bug.
let shellModeHintShown = false;
const { showWarning } = useToast();
async function maybeHintLightweightShell() {
  if (shellModeHintShown) return;
  shellModeHintShown = true;
  try {
    const mode = await invoke<string>("get_terminal_shell_mode");
    if (mode === "lightweight") {
      showWarning(
        "Your shell config is heavy, so the terminal runs in lightweight mode to reduce CPU usage (rc files are not loaded)."
      );
    }
  } catch {
    // Backend without the command (older build) — ignore.
  }
}

async function initForCwd(cwd: string) {
  const myGen = ++initGeneration;
  maybeHintLightweightShell();
  const restored = await restoreDirectoryState(cwd);
  if (myGen !== initGeneration) return; // superseded by a newer switch
  if (restored) {
    // State was saved (backend process still alive) — restore it, no new spawn.
    tabs.value = restored.restoredTabs;
    activeTabIndex.value = restored.restoredIndex;
    tabCounter = restored.restoredCounter;
    await nextTick();
    if (myGen !== initGeneration) return;
    const tab = tabs.value[activeTabIndex.value];
    if (tab) {
      // Fire-and-forget: don't block the session switch on terminal init.
      // mountTerminal restores history + attaches the listener asynchronously;
      // the UI stays responsive and the terminal fills in as it initializes.
      // Pass myGen so mountTerminal can abandon itself if superseded by a newer
      // switch (prevents a listener/term leak on the orphaned tab).
      mountTerminal(tab, myGen).catch((err) => {
        console.error("Failed to init terminal:", err);
      });
    }
  } else if (!spawnedCwds.has(cwd)) {
    // Never spawned a shell for this directory — create one. The directory is
    // only marked as "spawned" once the shell actually exists (inside addTab),
    // so an abandoned spawn (superseded by a newer switch while the shell was
    // starting) can't leave the directory marked with neither a tab nor a saved
    // state — which made later opens show an EMPTY terminal panel.
    await nextTick();
    if (myGen !== initGeneration) return;
    await addTab(myGen);
  }
}

watch(
  () => props.cwd,
  async (newCwd, oldCwd) => {
    // Invalidate any in-flight init from the previous directory so a stale
    // async restore can't overwrite the new directory's tabs.
    initGeneration++;
    // Save old directory's terminal state
    if (oldCwd) {
      saveDirectoryState(oldCwd);
    }
    // Tear down current DOM/listeners
    teardownCurrentTabs();
    tabs.value = [];
    activeTabIndex.value = -1;

    if (newCwd && props.active) {
      await initForCwd(newCwd);
    }
  }
);

// Lazy init: only spawn/restore terminals when the panel is actually visible.
// With the workspace kept mounted via v-show, this prevents auto-launching an
// interactive shell for every session on app start.
watch(
  () => props.active,
  async (active) => {
    if (active) {
      // Only init if not already initialized for the current cwd. A session
      // switch that also toggles the panel fires both watch(cwd) and this
      // watcher — without this guard, initForCwd would run twice for the same
      // directory (and could double-spawn).
      if (props.cwd && tabs.value.length === 0) await initForCwd(props.cwd);
    } else {
      // Panel hidden — persist tab metadata so re-opening restores it.
      // (Backend processes stay alive; only the DOM/listeners are torn down.)
      if (props.cwd) saveDirectoryState(props.cwd);
    }
  },
  { immediate: true }
);

onBeforeUnmount(() => {
  // Save state (keep backend processes alive)
  if (props.cwd) {
    saveDirectoryState(props.cwd);
  }
  // Tear down DOM + listeners
  teardownCurrentTabs();
  // Note: backend terminal processes are NOT killed here.
  // They persist until the user explicitly closes a tab or the app exits.
});

// Expose a way for the parent to terminate every terminal process (used by the
// "close terminal" confirmation flow).
defineExpose({ killAll });
</script>

<template>
  <div class="flex flex-col h-full bg-[#0d1117]">
    <!-- Header bar: single row with tabs inline -->
    <div
      class="flex items-center h-[36px] flex-shrink-0 select-none border-b border-white/[0.06]"
      style="background: linear-gradient(180deg, #161b22 0%, #0d1117 100%)"
    >
      <!-- Left: status indicator -->
      <div class="flex items-center gap-1.5 pl-3 pr-1.5 shrink-0">
        <span class="w-[5px] h-[5px] rounded-full bg-[#3fb950] ring-1 ring-[#3fb950]/30" />
        <span class="text-[10px] font-semibold text-[#8b949e] tracking-[0.04em] uppercase">TERMINAL</span>
      </div>

      <!-- Tabs: horizontal scroll -->
      <div
        class="flex items-center gap-0.5 min-w-0 max-w-[420px] overflow-x-auto [&::-webkit-scrollbar]:hidden"
        style="scrollbar-width: none;"
        ref="tabsScrollEl"
      >
        <button
          v-for="(tab, i) in tabs"
          :key="tab.id"
          @click="switchTab(i)"
          @click.middle.prevent="closeTab(i)"
          class="flex items-center gap-1 px-2 h-[22px] rounded text-[10px] whitespace-nowrap shrink-0 cursor-pointer transition-colors select-none"
          :class="
            i === activeTabIndex
              ? 'bg-[#0d1117] text-[#c9d1d9] border border-white/[0.08]'
              : 'text-[#484f58] hover:text-[#8b949e] hover:bg-white/[0.04]'
          "
        >
          <TerminalIcon :size="9" />
          <span>{{ tab.title }}</span>
          <button
            @click.stop="closeTab(i)"
            class="w-[14px] h-[14px] flex items-center justify-center rounded hover:bg-white/[0.1] text-[#484f58] hover:text-[#c9d1d9]"
            title="Close"
          >
            <X :size="8" />
          </button>
        </button>
        <button
          @click="addTab()"
          class="w-[22px] h-[22px] flex items-center justify-center rounded text-[#484f58] hover:text-[#8b949e] hover:bg-white/[0.06] transition-colors shrink-0"
          title="New Terminal"
        >
          <Plus :size="13" />
        </button>
      </div>

      <!-- Right: actions -->
      <div class="flex items-center gap-px pr-2 shrink-0 ml-auto">
        <button
          @click="handleClear"
          class="w-[22px] h-[22px] flex items-center justify-center rounded text-[#8b949e] hover:text-[#c9d1d9] hover:bg-white/[0.08] transition-colors cursor-pointer"
          title="Clear"
        >
          <Trash2 :size="11.5" />
        </button>
        <button
          @click="emit('close')"
          class="w-[22px] h-[22px] flex items-center justify-center rounded text-[#8b949e] hover:text-[#c9d1d9] hover:bg-white/[0.08] transition-colors cursor-pointer"
          title="Close"
        >
          <X :size="11.5" />
        </button>
      </div>
    </div>

    <!-- Terminal containers -->
    <div class="flex-1 relative overflow-hidden">
      <div
        v-for="(tab, i) in tabs"
        :key="tab.id"
        :ref="(el) => { if (el) containerEls[tab.id] = el as HTMLElement }"
        class="absolute inset-0"
        :class="{ 'hidden': i !== activeTabIndex }"
      />
      <!-- Empty state when no tabs -->
      <div
        v-if="tabs.length === 0"
        class="absolute inset-0 flex items-center justify-center text-[#30363d]"
      >
        <TerminalIcon :size="28" class="opacity-20" />
      </div>
    </div>
  </div>
</template>
