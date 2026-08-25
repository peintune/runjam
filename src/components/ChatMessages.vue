<script setup lang="ts">
import { ref, watch, nextTick, onBeforeUnmount, reactive, computed } from "vue";
import {
  ChevronDown, ChevronUp, ChevronRight, Clock, Check, Copy,
  MousePointerClick, FolderOpen,
} from "lucide-vue-next";
import { respondInteraction, respondPermission } from "../api/sessions";
import { useMarkdown, renderCached, clearStreamingCache, containsCodeFence } from "../composables/useMarkdown";
import { useThemeStore } from "../stores/useThemeStore";
import AgentIcon from "./AgentIcon.vue";
import MessageContent from "./MessageContent.vue";
import { invoke } from "@tauri-apps/api/core";
import { recordRender, recordMdParse } from "../lib/diag";
import { t } from "../i18n";

const { safeSliceForStreaming, renderMermaidBlocks, hasMermaid } = useMarkdown();
const themeStore = useThemeStore();

// ═══ Types ═══
export interface InteractionOption { key: string; label: string; is_default: boolean; }
export interface ToolCall {
  toolName: string;
  input: string;
  output?: string;
  status: string;
  startTime?: number;
  durationMs?: number;
  title?: string;
}
export interface PermissionPrompt {
  requestId: string;
  prompt: string;
  options: InteractionOption[];
  sessionId: string;
}
export interface Message {
  role: "user" | "agent";
  content: string;
  thinking?: string;
  thoughtDuration?: string;
  interaction?: { prompt: string; options: InteractionOption[]; sessionId: string };
  permission?: PermissionPrompt;
  isProcessing?: boolean;
  startTime?: number;
  toolCalls?: ToolCall[];
  totalTokens?: number;
  totalDurationMs?: number;
  inputTokens?: number;
  outputTokens?: number;
  cachedTokens?: number;
}

// ═══ Props ═══
const props = defineProps<{ messages: Message[]; agentId?: string; active?: boolean }>();
const emit = defineEmits<{ (e: "contentUpdated"): void }>();

// ═══ Message Groups: consecutive agent messages merge into one bubble ═══
const messageGroups = computed(() => {
  const groups: { type: "user" | "agent"; items: { msg: Message; oi: number }[] }[] = [];
  for (let i = 0; i < props.messages.length; i++) {
    const msg = props.messages[i];
    const item = { msg, oi: i };
    if (msg.role === "user") {
      groups.push({ type: "user", items: [item] });
    } else {
      const last = groups[groups.length - 1];
      if (last && last.type === "agent") {
        last.items.push(item);
      } else {
        groups.push({ type: "agent", items: [item] });
      }
    }
  }
  return groups;
});

// ═══ Lazy rendering of off-viewport message groups ═══
// Rendering every historical message's full DOM (markdown + hljs + DOMPurify)
// in one tick is the dominant cost when opening a large session. We render only
// the groups near the viewport; everything else gets a cheap fixed-height
// placeholder. An IntersectionObserver (rootMargin 400px so rendering happens
// before the user actually scrolls there) flips a group from placeholder → fully
// rendered. Active/live groups always render fully. Small histories render
// everything (no observer overhead). The module-level markdown cache stays —
// revisits are instant.
const visibleGroups = ref<Set<number>>(new Set());
let visibilityObserver: IntersectionObserver | null = null;
const GHOST_HEIGHT = 140; // px estimate for placeholder groups

// Lazy rendering triggers on ESTIMATED RENDER COST, not just group count. A
// session with 19 huge messages (many KB each, dozens of code blocks) is far more
// expensive to render than one with 100 one-line messages — so we must key on the
// total content size, not the number of groups. Thresholds:
//   GROUP_COUNT    — many small messages (e.g. 100+ turns)
//   TOTAL_CHARS    — fewer but large messages (e.g. 60KB+ of markdown/code)
// Either condition flips the session to lazy rendering.
const LAZY_GROUP_COUNT = 50;
const LAZY_TOTAL_CHARS = 60_000; // ~60KB of content → render on demand
/** Estimated render cost of the whole history: sum of all message content length.
 * Cheap O(n) string-length scan, re-evaluated only when messages change. */
const totalRenderChars = computed(() => {
  let n = 0;
  for (let i = 0; i < props.messages.length; i++) {
    const m = props.messages[i];
    n += (m.content?.length || 0) + (m.thinking?.length || 0);
  }
  return n;
});
/** True when this history is large enough to warrant lazy rendering */
const lazyMode = ref(false);
function shouldLazyRender(): boolean {
  // Sticky: once a session is deemed large, it stays lazy for its whole life.
  // This prevents a mid-stream flip (content growing past the threshold) from
  // suddenly turning already-rendered groups into placeholders. Reset on switch.
  if (lazyMode.value) return true;
  const large =
    messageGroups.value.length > LAZY_GROUP_COUNT ||
    totalRenderChars.value > LAZY_TOTAL_CHARS;
  if (large) lazyMode.value = true;
  return large;
}
/** When true, every group renders fully (lazy rendering disabled). Used by
 * SessionView's scrollToMessage fallback: jumping to an arbitrary historical
 * message needs that group's DOM present, so we render everything once. */
const forceRender = ref(false);

function ensureGroup(el: HTMLElement, gIdx: number) {
  // Skip observer entirely for small histories — render everything (no overhead)
  if (!shouldLazyRender()) return;
  if (visibleGroups.value.has(gIdx)) return;
  if (!visibilityObserver) {
    // Root the observer at the scroll container (the message list's scrolling
    // ancestor), NOT the viewport — otherwise a container taller than the window
    // or a split layout would report groups as "not intersecting" and they'd
    // never render. chatEl is the list content; its scrolling ancestor is the
    // container that scrolls.
    let root: HTMLElement | null = null;
    let node = chatEl.value?.parentElement ?? el.parentElement;
    while (node) {
      const style = window.getComputedStyle(node);
      if (/(auto|scroll|overlay)/.test(style.overflowY)) { root = node; break; }
      node = node.parentElement;
    }
    visibilityObserver = new IntersectionObserver(
      (entries) => {
        const obs = visibilityObserver;
        for (const en of entries) {
          const idx = Number((en.target as HTMLElement).dataset.gIdx);
          if (Number.isNaN(idx)) continue;
          if (en.isIntersecting) {
            visibleGroups.value.add(idx);
            if (obs) obs.unobserve(en.target); // render once, keep rendered
          }
        }
      },
      { root, rootMargin: "400px 0px 400px 0px" }, // render 400px before entering viewport
    );
  }
  visibilityObserver.observe(el);
}

// A group must render fully when it's visible OR live (streaming/processing/typing).
// Also always render the LAST few groups: a chat opens scrolled to the bottom
// (latest messages), so those must be real content immediately — otherwise the
// initial scroll target is computed against placeholder heights and lands in the
// middle of history. The old history above stays lazy.
const ALWAYS_RENDER_TAIL = 8; // last N groups render fully on open
function shouldRenderGroup(g: { items: { oi: number; msg: Message }[] }, gIdx: number): boolean {
  if (forceRender.value) return true; // scrollToMessage fallback: render everything
  if (!shouldLazyRender()) return true; // small/cheap session: render all
  if (gIdx >= messageGroups.value.length - ALWAYS_RENDER_TAIL) return true; // bottom of chat
  if (visibleGroups.value.has(gIdx)) return true;
  return isGroupActive(g.items); // never placeholder a live group
}

// Exposed to SessionView so scrollToMessage can force every group to render
// (its jump target may be an arbitrary historical group that is still a
// placeholder). Once forced, lazy rendering stays off for this session — the
// user explicitly asked to browse the full history, so full render is fine.
// ═══ 历史折叠：超长会话只渲染尾部，头部收进一个"查看更早"按钮 ═══
// 惰性渲染（IntersectionObserver + placeholder）决定"组是否渲染真实内容"；
// 折叠更进一步，让头部组根本不进入 v-for —— DOM 里只有 1 个按钮而不是
// 上千个 placeholder div。展开时锚定回原位置，视口不跳。
const HISTORY_FOLD_THRESHOLD = 40; // 组数超过该值就折叠头部
const showFullHistory = ref(false);
const foldedHeadCount = computed(() => {
  if (showFullHistory.value) return 0;
  return Math.max(0, messageGroups.value.length - HISTORY_FOLD_THRESHOLD);
});

function expandHistory() {
  const anchorIdx = foldedHeadCount.value;
  showFullHistory.value = true;
  nextTick(() => {
    // 锚定：展开后滚动到原第一组的位置，避免视口跳到历史开头
    const el = chatEl.value?.querySelector(`[data-gIdx="${anchorIdx}"]`);
    if (el) el.scrollIntoView({ block: "start" });
  });
}

function forceRenderAll() {
  forceRender.value = true;
  showFullHistory.value = true; // scrollToMessage 需要全部组真实渲染
}

defineExpose({ forceRenderAll });

// Reset visibility when the message array is swapped to a DIFFERENT session.
// Fires only on reference change, so it's separate from the deep watchers below.
// Streaming appends replace props.messages with a new array too, but the first
// message's reference is unchanged (same session) — we must NOT reset then, or
// the visible set would be wiped on every chunk and groups would flicker between
// placeholder and rendered. Only a real session switch (different first message)
// resets the set. We key on the FIRST message's reference only (not length):
// an in-session rollback (error path pops messages) shrinks the array but stays
// the same session, so it must not reset either.
watch(
  () => props.messages,
  (newMsgs, oldMsgs) => {
    const sameSession = !!oldMsgs && oldMsgs.length > 0 && newMsgs[0] === oldMsgs[0];
    if (!sameSession) {
      if (visibilityObserver) { visibilityObserver.disconnect(); visibilityObserver = null; }
      visibleGroups.value = new Set();
      forceRender.value = false; // a fresh session starts lazy again
      lazyMode.value = false; // re-evaluate size for the new session
    }
  },
);

// 会话隐藏（active=false）时列表整个被 v-if 移除，observer 若还持着旧
// 的（已卸载）placeholder 元素就会泄漏节点引用。断开后由 ensureGroup
// 在重新可见时惰性重建。
watch(
  () => props.active,
  (isActive) => {
    if (isActive === false && visibilityObserver) {
      visibilityObserver.disconnect();
      visibilityObserver = null;
    }
  },
);

// ═══ Collapsing state — track EXPANDED, not collapsed (default: hidden) ═══
const thinkingExpanded = ref<Set<number>>(new Set());
const toolExpanded = ref<Set<string>>(new Set());
const respondedPermissions = ref<Set<string>>(new Set());
const respondedInteractions = ref<Set<string>>(new Set());

// 用户手动切换过的 thinking 索引。一旦用户主动点过（展开或收起），自动
// 展开/折叠逻辑就不再干预该条 thought——否则流式 chunk 触发的 deep watch
// 会反复把用户刚展开的历史 thought 折叠回去，表现为"没法展开，一直折叠"。
const userToggledThinking = ref<Set<number>>(new Set());

// Auto-expand thinking if its message has no content yet (still thinking phase)
function shouldAutoExpandThinking(msg: Message): boolean {
  return !msg.content && !!msg.thinking;
}

watch(
  () => props.messages,
  (msgs) => {
    let hasActiveThinking = false;
    for (let i = 0; i < msgs.length; i++) {
      // Auto-expand thinking that hasn't reached content phase yet
      // (skip thoughts the user explicitly toggled)
      if (!userToggledThinking.value.has(i) && shouldAutoExpandThinking(msgs[i])) {
        thinkingExpanded.value.add(i);
        hasActiveThinking = true;
      }
    }

    // When a new thought is active, collapse previously completed thoughts —
    // but never ones the user explicitly toggled.
    if (hasActiveThinking) {
      for (let i = 0; i < msgs.length; i++) {
        const m = msgs[i];
        // Collapse completed thoughts (those that have both thinking and content)
        if (
          m.content &&
          m.thinking &&
          !userToggledThinking.value.has(i) &&
          thinkingExpanded.value.has(i)
        ) {
          thinkingExpanded.value.delete(i);
        }
      }
    }
  },
  { deep: true, immediate: true },
);

function toggleThinking(idx: number) {
  userToggledThinking.value.add(idx);
  if (thinkingExpanded.value.has(idx)) thinkingExpanded.value.delete(idx);
  else thinkingExpanded.value.add(idx);
}
function toggleToolCall(msgIdx: number, toolIdx: number) {
  const key = `${msgIdx}-${toolIdx}`;
  if (toolExpanded.value.has(key)) toolExpanded.value.delete(key);
  else toolExpanded.value.add(key);
}

// ═══ Tool detail log expansion — show FULL input/output on demand ═══
// tc.input / tc.output can be multi-thousand-line logs (Bash output, errors).
// Collapsed shows a 3-line preview; expanding renders the whole text inside an
// internally scrollable region so huge logs don't blow up the message list.
const logExpanded = ref<Set<string>>(new Set());
function toggleLogDetail(key: string) {
  if (logExpanded.value.has(key)) logExpanded.value.delete(key);
  else logExpanded.value.add(key);
}
/** Treat text as "long" when it exceeds ~3 lines or 160 chars. */
function isLongText(text?: string): boolean {
  if (!text) return false;
  return text.length > 160 || text.split("\n").length > 3;
}
function logLineCount(text?: string): number {
  return text ? text.split("\n").length : 0;
}

// ═══ Typewriter / streaming state ═══
const displayMap = reactive<Record<number, { thinking: string; content: string }>>({});
const startTimes = reactive<Record<number, { thinking: number; content: number }>>({});
const frozenDurations = reactive<Record<number, { thinking: number; content: number }>>({});
const now = ref(Date.now());
const timers: ReturnType<typeof setInterval>[] = [];
/** Track one timer per message index so we can avoid duplicate typewriters on streaming updates. */
const timerMap = new Map<number, { thinking?: ReturnType<typeof setInterval>; content?: ReturnType<typeof setInterval> }>();
const thinkingRefs = ref<Record<number, HTMLElement>>({});

// ── now tick: only run while something is live ──
// `now` is read in the template (running-tool durations, "working Xs", live
// thinking label). A naive `setInterval` that bumps `now` every 500ms forces the
// WHOLE message list to re-render on every tick — every message's
// renderContent() re-runs even when nothing changed. That is the dominant cost
// when viewing a long, already-finished conversation: the list re-renders twice
// a second for no reason. So we only tick while there is genuinely live content
// (a streaming message, a running tool call, an in-progress thinking block).
const hasLiveActivity = computed(() => {
  const msgs = props.messages;
  for (let i = 0; i < msgs.length; i++) {
    const m = msgs[i];
    // Message is actively being generated / processed — this is the ONLY live
    // signal that matters. A message still being generated needs the `now`
    // tick to show live durations ("working Xs", "Thinking • Xs").
    if (m.isProcessing === true) return true;
    // Typewriter still revealing content or thinking (content is growing)
    const d = displayMap[i];
    if (d) {
      if (m.content && d.content.length < m.content.length) return true;
      if (m.thinking && d.thinking.length < m.thinking.length) return true;
    }
    // A tool call is currently executing (running duration shown)
    if (m.toolCalls && m.toolCalls.some((tc) => tc.status === "started" || tc.status === "running")) return true;
  }
  return false;
});

let tickTimer: ReturnType<typeof setInterval> | null = null;
function stopTick() {
  if (tickTimer) { clearInterval(tickTimer); tickTimer = null; }
}
function startTick() {
  if (tickTimer) return;
  // 1s 一拍（原来 500ms）：所有展示都是"秒"粒度（working Xs / Thinking • Xs），
  // 1s 更新完全够用，还能少一半的全列表重渲染。
  tickTimer = setInterval(() => { now.value = Date.now(); }, 1000);
}
// 仅当会话可见（active）且有实时内容时才 tick——后台隐藏会话不需要任何重渲染。
watch(
  [hasLiveActivity, () => props.active],
  ([live, isActive]) => {
    if (live && isActive !== false) startTick(); else stopTick();
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  stopTick();
  timers.forEach(clearInterval);
  timerMap.forEach((t) => {
    if (t.thinking) clearInterval(t.thinking);
    if (t.content) clearInterval(t.content);
  });
  timerMap.clear();
  if (msgsRafHandle !== null) {
    cancelAnimationFrame(msgsRafHandle);
    msgsRafHandle = null;
  }
  pendingMsgs = null;
  if (visibilityObserver) {
    visibilityObserver.disconnect();
    visibilityObserver = null;
  }
  if (contentObserver) {
    contentObserver.disconnect();
    contentObserver = null;
  }
});

/** 智能打字机判定：仅当消息较短（≤1500 字符）且不含代码围栏时才逐字揭示。
 * 长内容/含代码块的消息直接一次性写入 displayMap 完整显示——即使步长自适应，
 * 超长文本仍会产生大量 tick 与整列表 diff，且代码围栏在流式半截状态下反复
 * 触发高开销的 markdown 解析。 */
const TYPING_MAX_CHARS = 1500;

function shouldTypewriter(msg: Message): boolean {
  const content = msg.content || "";
  const thinking = msg.thinking || "";
  if (content.length + thinking.length > TYPING_MAX_CHARS) return false;
  return !containsCodeFence(content) && !containsCodeFence(thinking);
}

function startTypewriter(
  idx: number,
  fullText: string,
  field: "thinking" | "content",
  speed = 8,
) {
  if (!displayMap[idx]) displayMap[idx] = { thinking: "", content: "" };
  if (!startTimes[idx]) startTimes[idx] = { thinking: 0, content: 0 };
  if (!frozenDurations[idx]) frozenDurations[idx] = { thinking: 0, content: 0 };
  if (startTimes[idx][field] === 0) startTimes[idx][field] = Date.now();

  const current = displayMap[idx][field];
  if (current.length >= fullText.length) return;

  // Clear any existing timer for this field to avoid duplicate timers
  if (!timerMap.has(idx)) timerMap.set(idx, {});
  const existing = timerMap.get(idx)!;
  if (existing[field]) {
    clearInterval(existing[field]!);
  }

  const timer = setInterval(() => {
    if (!displayMap[idx]) {
      clearInterval(timer);
      return;
    }
    const cur = displayMap[idx][field];
    if (cur.length < fullText.length) {
      // 自适应步长：剩余越长，每步揭示越多，总步数收敛到 ~40 步左右。
      // 原先固定 8~16 字符/步，一条 10KB 的长消息要 tick 上千次，每次 tick
      // 都触发整列表重渲染 + 对增长的切片做一次 markdown 解析——多会话并行
      // 流式时这就是卡顿的主因。步长自适应后长消息只需 ~40 步，渲染与解析
      // 开销下降约 20 倍，视觉上仍是从上到下流畅填充。
      const remaining = fullText.length - cur.length;
      const chunk = Math.max(8, Math.min(600, Math.ceil(remaining / 40)));
      displayMap[idx][field] = fullText.substring(
        0,
        Math.min(cur.length + chunk, fullText.length),
      );
      nextTick(() => {
        const el = thinkingRefs.value[idx];
        if (el) el.scrollTop = el.scrollHeight;
      });
    } else {
      clearInterval(timer);
      if (timerMap.has(idx)) {
        delete timerMap.get(idx)![field];
      }
      frozenDurations[idx][field] = Date.now() - startTimes[idx][field];
      // content 揭示完成时直接触发 mermaid 渲染——取代原 deep watcher
      // 轮询（旧 watcher 每个 typewriter tick 都跑一遍 .map() 生成新数组
      // 再做 deep 比较，是流式期间每 tick 的额外 O(n) 开销）。isTyping
      // 检查会挡住"内容仍在流式增长"的中间态，最终完成时只渲染一次。
      if (field === "content" && props.messages[idx]) {
        void handleMermaidInContent(idx, props.messages[idx]!);
      }
    }
  }, speed);
  existing[field] = timer;
  timers.push(timer);
}

function elapsed(ms: number): string {
  const s = Math.floor(ms / 1000);
  if (s < 60) return s + "s";
  return Math.floor(s / 60) + "m " + (s % 60) + "s";
}

/** Detect if a tool call is a file-system operation (mkdir, write, create, edit, etc.) */
function isFileOperation(tc: ToolCall): boolean {
  const name = (tc.toolName || "").toLowerCase();
  const input = (tc.input || "").toLowerCase();
  // Match by tool name keywords
  const fileKeywords = ["mkdir", "write_file", "create_file", "edit_file", "file_write", "file_create",
    "rename_file", "delete_file", "move_file", "copy_file", "read_file", "write_file_to"];
  if (fileKeywords.some(k => name.includes(k))) return true;
  // Also match generic keywords when tool name is short (e.g. "Write", "Create", "Edit", "Read")
  if (name.length <= 8 && ["write", "create", "edit", "read", "file", "rename", "delete", "copy", "move"].some(k => name.includes(k))) return true;
  // Bash / shell commands that touch the filesystem
  if (name === "bash" || name === "execute_command" || name === "run_command") {
    if (input.includes("mkdir") || input.includes("touch ") || input.includes("write_file") ||
        input.includes("create_file") || input.includes("cat >") || input.includes("echo ") ||
        input.includes(">") || input.includes("mv ") || input.includes("cp ") || input.includes("rm ")) return true;
  }
  // Check input JSON for file_path / path fields
  try {
    const parsed = JSON.parse(tc.input);
    if (parsed.file_path || parsed.path || parsed.file || parsed.filename) return true;
  } catch { /* not JSON */ }
  return false;
}

/** Extract the first file/directory path from a tool call's input */
function extractFilePath(tc: ToolCall): string | null {
  try {
    const parsed = JSON.parse(tc.input);
    // Direct field: file_path, path, file, filename
    const pathField = parsed.file_path || parsed.path || parsed.file || parsed.filename;
    if (pathField && typeof pathField === "string") return pathField;
    // Command field: extract path from shell command
    if (parsed.command && typeof parsed.command === "string") {
      return extractPathFromCommand(parsed.command);
    }
    return null;
  } catch {
    // Not JSON — try plain text extraction
    return extractPathFromString(tc.input);
  }
}

/** Extract file path from a shell command string */
function extractPathFromCommand(cmd: string): string | null {
  // Handle output redirection: echo 'content' > /path/to/file
  const redirectMatch = cmd.match(/[>]+\s*(\S+)/);
  if (redirectMatch) return redirectMatch[1];
  // Handle mkdir -p /path/to/dir
  const mkdirMatch = cmd.match(/mkdir\s+(-p\s+)?(\S+)/);
  if (mkdirMatch) return mkdirMatch[2] || mkdirMatch[1];
  // Handle touch /path/to/file
  const touchMatch = cmd.match(/touch\s+(\S+)/);
  if (touchMatch) return touchMatch[1];
  // Handle mv /from /to — take the last path
  const mvMatch = cmd.match(/mv\s+(\S+)\s+(\S+)/);
  if (mvMatch) return mvMatch[2];
  // Handle cp /from /to — take the last path
  const cpMatch = cmd.match(/cp\s+(\S+)\s+(\S+)/);
  if (cpMatch) return cpMatch[2];
  // Fallback: last argument that looks like a path
  const parts = cmd.split(/\s+/);
  for (let i = parts.length - 1; i >= 0; i--) {
    const p = parts[i];
    if (p.startsWith("/") || p.startsWith("./") || p.startsWith("~") || p.startsWith(".") || p.includes("/")) {
      return p;
    }
  }
  return null;
}

/** Extract file path from a non-JSON string */
function extractPathFromString(input: string): string | null {
  if (!input) return null;
  const trimmed = input.trim();
  // If it looks like a file path (starts with /, ~, ., or contains /)
  if (trimmed.startsWith("/") || trimmed.startsWith("~") || trimmed.startsWith("./") || trimmed.includes("/")) {
    return trimmed;
  }
  // Try to find a path pattern in the string
  const pathMatch = trimmed.match(/(\/[^\s"'<>|;:]+)/);
  if (pathMatch) return pathMatch[1];
  return null;
}

function openInExplorer(path: string) {
  // Resolve relative paths relative to the current working directory
  invoke("open_in_finder", { path }).catch((e: Error) => console.error("Failed to open path:", e));
}

function thinkingLabel(msg: Message, idx: number): string {
  if (msg.thoughtDuration) return `Thought • ${msg.thoughtDuration}`;
  // Frozen thinking duration takes priority over content check
  if (frozenDurations[idx]?.thinking)
    return `Thought • ${elapsed(frozenDurations[idx].thinking)}`;
  if (msg.content) return "Thought";
  // A finished thinking-only message (thinking shown, no content — an agent
  // that thought and then ended) should read as "Thought", not a live
  // "Thinking..." that implies it's still running. Only show a live timer
  // while the message is actually being generated.
  if (msg.isProcessing === true) {
    const st = startTimes[idx]?.thinking || 0;
    return st ? `Thinking • ${elapsed(now.value - st)}` : "Thinking...";
  }
  return "Thought";
}

function formatDuration(ms: number): string {
  const s = Math.floor(ms / 1000);
  if (s < 60) return s + "s";
  return Math.floor(s / 60) + "m " + (s % 60) + "s";
}

function formatTokens(n: number): string {
  if (n >= 1000) return (n / 1000).toFixed(1) + "k";
  return n.toString();
}

// ═══ Render content with safe streaming slice ═══
// Markdown parsing (marked + DOMPurify + hljs) is the most expensive step in the
// streaming hot path — it used to re-run for EVERY message on EVERY chunk and on
// the `now` tick. renderCached keeps a MODULE-level cache keyed by source string,
// so unchanged messages (the vast majority of a long conversation) are never
// re-parsed, and the cache survives session switches.
//
// safeSliceCache avoids redundant regex matching in safeSliceForStreaming when
// the same displayed content is requested multiple times in the same render cycle.
const safeSliceCache = new Map<string, string>();
function renderContent(idx: number, msg: Message): string {
  const fullContent = msg.content;
  const displayed = displayMap[idx]?.content;
  // Reading the theme here makes the whole list re-render (and re-key the
  // render cache) when the user switches light/dark.
  const theme = themeStore.theme;
  const t0 = performance.now();
  let html: string;
  if (displayed === undefined || displayed.length >= fullContent.length) {
    // 完成/历史内容：可缓存
    html = renderCached(fullContent, { sanitize: true, theme }, (ms) => recordMdParse(ms));
  } else {
    // 流式部分内容：使用独立流式缓存（不与历史缓存冲突）
    let safeSlice = safeSliceCache.get(displayed);
    if (safeSlice === undefined) {
      safeSlice = safeSliceForStreaming(displayed);
      safeSliceCache.set(displayed, safeSlice);
      // Keep the cache small — only need the most recent few slices
      if (safeSliceCache.size > 50) {
        const oldest = safeSliceCache.keys().next().value;
        if (oldest !== undefined) safeSliceCache.delete(oldest);
      }
    }
    html = renderCached(safeSlice, { sanitize: true, theme }, (ms) => recordMdParse(ms), false);
  }
  recordRender(performance.now() - t0);
  return html;
}

function isTyping(idx: number, msg: Message): boolean {
  const displayed = displayMap[idx]?.content;
  return displayed !== undefined && displayed.length < msg.content.length;
}

// Whether any message in the group is still active (has typing or is processing)
function isGroupActive(items: { msg: Message; oi: number }[]): boolean {
  return items.some(
    (it) => isTyping(it.oi, it.msg) || it.msg.isProcessing === true,
  );
}

// ═══ Mermaid post-processing ═══
const mermaidRenderedMessages = ref<Set<number>>(new Set());

async function handleMermaidInContent(idx: number, msg: Message) {
  if (isTyping(idx, msg)) return;
  if (!hasMermaid(msg.content)) return;
  if (mermaidRenderedMessages.value.has(idx)) return;

  mermaidRenderedMessages.value.add(idx);
  await nextTick();

  const containers = (chatEl.value as HTMLElement | null)?.querySelectorAll(
    `[data-msg-content="${idx}"]`,
  );
  if (containers && containers.length > 0) {
    await renderMermaidBlocks(containers[0] as HTMLElement, themeStore.theme);
  }
}

// When the app theme changes, re-render mermaid diagrams with the new palette.
watch(
  () => themeStore.theme,
  () => {
    mermaidRenderedMessages.value = new Set();
    clearStreamingCache();
    // Re-run mermaid post-processing once the DOM has the new-theme HTML.
    props.messages.forEach((m, i) => {
      if (m.content && hasMermaid(m.content)) void handleMermaidInContent(i, m);
    });
  },
);

// ═══ Message list reactivity ═══
let lastMsgsRef: Message[] | null = null;
let msgsRafHandle: number | null = null;
let pendingMsgs: Message[] | null = null;
/** 已初始化且与 displayMap 完全同步的消息索引（非 live）。deep watch 每
 * chunk 触发时，用该集合跳过历史消息的重复初始化热点，只完整处理真正有
 * 变化的 live 消息。live 消息不进入集合，会话切换时整体清空。 */
const knownIndices = new Set<number>();

function processMessages(msgs: Message[]) {
  if (msgs !== lastMsgsRef) {
    lastMsgsRef = msgs;
    knownIndices.clear();
    // Remove entries for messages that no longer exist (e.g. session switch)
    for (const k of Object.keys(displayMap)) {
      if (Number(k) >= msgs.length) {
        delete displayMap[Number(k)];
        delete startTimes[Number(k)];
        delete frozenDurations[Number(k)];
      }
    }
    mermaidRenderedMessages.value.clear();
    // Clear streaming caches on session switch — stale streaming slices
    // from the previous session are never going to be requested again.
    clearStreamingCache();
    safeSliceCache.clear();
    // Don't clear timers — they will naturally complete when content matches
    // 会话切换恢复加速：预填尾部 ALWAYS_RENDER_TAIL 组为可见——首屏
    // （视口+尾部）立即渲染真实内容而非 placeholder，IntersectionObserver
    // 只负责后续历史的滚动懒加载，减少 placeholder→真实内容的首帧翻转。
    const tailStart = Math.max(0, messageGroups.value.length - ALWAYS_RENDER_TAIL);
    const prefill = new Set<number>();
    for (let g = tailStart; g < messageGroups.value.length; g++) prefill.add(g);
    visibleGroups.value = prefill;
  }
  for (let i = 0; i < msgs.length; i++) {
    const m = msgs[i];

    // Only messages explicitly marked as processing get the typewriter effect.
    // Completed/historical messages (isProcessing: false or undefined) render instantly.
    // 后台隐藏会话（active=false）不做打字机揭示：直接同步完整内容，既不跑
    // interval 也不触发重渲染；切回可见时内容已就绪，无需回放。
    const isLive = m.isProcessing === true && props.active !== false;

    // knownIndices 快路径：已完成/历史消息且 displayMap 已同步时跳过重复
    // 的初始化块（startTimes/frozenDurations/折叠/mermaid 判定）。live 消息
    // 内容持续增长，必须完整处理，不进入该集合。
    if (!isLive && knownIndices.has(i) &&
        displayMap[i]?.content === (m.content || "") &&
        displayMap[i]?.thinking === (m.thinking || "")) {
      continue;
    }
    if (!displayMap[i]) displayMap[i] = { thinking: "", content: "" };
    if (!startTimes[i]) startTimes[i] = { thinking: 0, content: 0 };
    if (!frozenDurations[i]) frozenDurations[i] = { thinking: 0, content: 0 };

    if (isLive) {
      knownIndices.delete(i);
      // 智能打字机分流：短消息保留逐字效果；长消息/含代码块的消息直接
      // 一次性写入 displayMap 完整显示，跳过逐字打印，消除长文本的 tick
      // 风暴与整列表 diff。渲染管线统一走 displayMap，后续 wasLive→
      // clearStreamingCache 逻辑保持不变。
      const typewriter = shouldTypewriter(m);
      if (m.thinking && displayMap[i].thinking.length < m.thinking.length) {
        if (typewriter) startTypewriter(i, m.thinking, "thinking", 16);
        else displayMap[i].thinking = m.thinking;
      }
      if (m.content && displayMap[i].content.length < m.content.length) {
        if (typewriter) startTypewriter(i, m.content, "content", 16);
        else displayMap[i].content = m.content;
      }
    } else {
      const wasLive = displayMap[i].content.length < (m.content?.length || 0) ||
                      displayMap[i].thinking.length < (m.thinking?.length || 0);
      // Message is no longer live — freeze any running durations
      if (startTimes[i].thinking && !frozenDurations[i].thinking) {
        frozenDurations[i].thinking = Date.now() - startTimes[i].thinking;
      }
      if (startTimes[i].content && !frozenDurations[i].content) {
        frozenDurations[i].content = Date.now() - startTimes[i].content;
      }
      displayMap[i].thinking = m.thinking || "";
      displayMap[i].content = m.content || "";
      // Auto-collapse thinking that now has content (done phase) — unless the
      // user explicitly toggled it, in which case their choice wins.
      if (m.content && !userToggledThinking.value.has(i) && thinkingExpanded.value.has(i)) {
        thinkingExpanded.value.delete(i);
      }
      if (m.content && hasMermaid(m.content)) {
        handleMermaidInContent(i, m);
      }
      // When a message finishes streaming, clear transient caches so stale
      // streaming slices don't accumulate. The final content is cached in the
      // shared module-level cache.
      if (wasLive) {
        clearStreamingCache();
        safeSliceCache.clear();
      }
      // 内容已完整同步到 displayMap，标记为已知——后续无变化的 deep watch
      // 对该消息走快路径跳过，只处理 live 消息。
      knownIndices.add(i);
    }
  }
}

watch(
  () => props.messages,
  (msgs) => {
    const sessionChanged = msgs !== lastMsgsRef;
    // 流式过程中高频 chunk 会触发大量 deep watch。若存在正在流式的消息，
    // 把全列表处理合并到 rAF：每帧至多一次，代替每 chunk 一次全列表遍历。
    // 会话切换（引用变化）与"无任何 live 消息"（流式结束/历史消息）必须
    // 立即处理：切换需即时清索引，结束态内容需即时落定，不被 rAF 延迟。
    let hasLive = false;
    for (let i = 0; i < msgs.length; i++) {
      if (msgs[i].isProcessing === true) { hasLive = true; break; }
    }
    if (sessionChanged || !hasLive) {
      if (msgsRafHandle !== null) {
        cancelAnimationFrame(msgsRafHandle);
        msgsRafHandle = null;
      }
      pendingMsgs = null;
      processMessages(msgs);
      return;
    }
    pendingMsgs = msgs;
    if (msgsRafHandle !== null) return; // 已有一帧待处理，丢弃本次重复调度
    msgsRafHandle = requestAnimationFrame(() => {
      msgsRafHandle = null;
      if (pendingMsgs) {
        const batch = pendingMsgs;
        pendingMsgs = null;
        processMessages(batch);
      }
    });
  },
  { deep: true },
);

// ═══ DOM refs ═══
const chatEl = ref<HTMLElement | null>(null);
const copiedIndex = ref<number | null>(null);

// ═══ 内容高度变化 → 通知 SessionView 跟随滚动 ═══
// 流式生成中 SessionView 依赖本信号：仅当用户停留在底部附近时滚动到最新
// 一行，保证"定位在最后一行就持续看到新内容"。用 ResizeObserver 而非轮询
// /节流——它只在内容尺寸真实变化时回调，天然覆盖 typewriter 逐字揭示、
// 长消息直写、mermaid/懒渲染渲染完成、图片加载等所有高度变化源。
let contentObserver: ResizeObserver | null = null;
watch(chatEl, (el) => {
  contentObserver?.disconnect();
  contentObserver = null;
  if (el) {
    contentObserver = new ResizeObserver(() => emit("contentUpdated"));
    contentObserver.observe(el);
  }
});

// ═══ Code-block copy via event delegation ═══
function handleContentClick(e: MouseEvent) {
  const btn = (e.target as HTMLElement).closest("[data-copy]") as HTMLElement | null;
  if (!btn) return;

  const wrap = btn.closest(".cb-wrap");
  if (!wrap) return;
  const code = wrap.querySelector("code");
  if (!code) return;

  const txt = code.textContent || "";
  navigator.clipboard.writeText(txt).then(() => {
    btn.innerHTML = `<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="20 6 9 17 4 12"/></svg><span>${t("chat.copied")}</span>`;
    setTimeout(() => {
      btn.innerHTML = `<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg><span>${t("chat.copy")}</span>`;
    }, 2000);
  });
}

// ═══ Interaction / Permission handlers ═══
async function handleInteractionClick(sessionId: string, key: string, msgIdx: number) {
  try {
    await respondInteraction(sessionId, key);
    respondedInteractions.value.add(String(msgIdx));
  } catch (err) {
    console.error(err);
  }
}
async function handlePermissionClick(
  sessionId: string,
  requestId: string,
  key: string,
  msgIdx: number,
) {
  try {
    await respondPermission(sessionId, requestId, key);
    respondedPermissions.value.add(String(msgIdx));
  } catch (err) {
    console.error(err);
  }
}

// ═══ Message action bar ═══
async function copyMessage(content: string, idx: number) {
  try {
    await navigator.clipboard.writeText(content);
    copiedIndex.value = idx;
    setTimeout(() => (copiedIndex.value = null), 2000);
  } catch (err) {
    console.error("Copy failed:", err);
  }
}

// ═══ Auto-scroll ═══
// NOTE: scrolling is owned by SessionView's smart stick-to-bottom logic (it
// knows whether the user scrolled up to read history). This component must NOT
// force scrollTop on every new message, or the user can never scroll up while a
// session streams.

// Truncate a long label for display, keep full text in title
function truncateLabel(label: string, maxLen = 32): string {
  return label.length > maxLen ? label.slice(0, maxLen) + "…" : label;
}
</script>

<template>
  <!-- 后台隐藏会话（active=false）渲染轻量占位：不建 vnode 树、不调用
       renderContent，ACP chunk 只触发上面的 watcher 同步 displayMap。
       否则 KeepAlive 保活的每个后台会话都会在收到自己的 chunk 时对整份
       消息列表做一次全量重渲染（vnode 重建在分离的 DOM 上进行），多个
       会话并行流式时主线程就被这些隐藏渲染吃满。切回可见时 list 立即
       渲染，markdown 有模块级缓存，秒出。 -->
  <div v-if="props.active !== false" ref="chatEl" class="space-y-6 py-2">
    <!-- ── 历史折叠：头部组收进一个按钮，点击展开并锚定回原位 ── -->
    <button
      v-if="foldedHeadCount > 0"
      @click="expandHistory"
      class="msg-row w-full flex items-center justify-center gap-2 py-3 text-[13px] text-indigo-500 hover:text-indigo-600 hover:bg-indigo-50/40 rounded-xl border border-dashed border-indigo-200 transition-colors cursor-pointer"
    >
      <ChevronDown :size="14" />
      查看更早的 {{ foldedHeadCount }} 条消息
    </button>

    <template v-for="(group, gIdx) in messageGroups" :key="gIdx">
      <template v-if="gIdx >= foldedHeadCount">
      <!-- ── Placeholder: cheap fixed-height, flips to full render when near viewport ── -->
      <div
        v-if="!shouldRenderGroup(group, gIdx)"
        :data-gIdx="gIdx"
        :ref="(el) => { if (el) ensureGroup(el as HTMLElement, gIdx); }"
        class="msg-row msg-row-ghost"
        :style="{ height: GHOST_HEIGHT + 'px' }"
      >
        <span class="text-gray-300 text-[13px]">…</span>
      </div>

      <!-- ── Fully rendered group (visible or live) ── -->
      <template v-else>
      <!-- ── User message group (always single) ── -->
      <div
        v-if="group.type === 'user'"
        class="msg-row flex gap-3 justify-end"
        :data-user-msg-index="gIdx"
        :data-gIdx="gIdx"
      >
        <div class="msg-user-bubble max-w-[75%] px-4 py-2.5 text-[15px] leading-relaxed">
          {{ group.items[0].msg.content }}
        </div>
      </div>

      <!-- ── Agent message group (1+ messages in one bubble) ── -->
      <div v-else class="msg-row flex gap-3 justify-start" :data-gIdx="gIdx">
        <div class="msg-agent-avatar shrink-0 mt-0.5">
          <AgentIcon v-if="agentId" :agent-id="agentId" :size="28" />
          <span v-else class="text-[13px] font-bold text-gray-400">A</span>
        </div>
        <div class="msg-agent-bubble max-w-[85%] px-5 py-4">
          <!-- Iterate each message in the group -->
          <template v-for="(item, ii) in group.items" :key="item.oi">
            <!-- Thinking -->
            <div
              v-if="item.msg.thinking"
              :class="ii > 0 ? 'mt-3 pt-3 border-t border-gray-100' : ''"
              class="mb-2.5"
            >
              <button
                @click="toggleThinking(item.oi)"
                class="flex items-center gap-1 px-1 py-0.5 text-[11px] font-normal text-gray-400 hover:text-gray-600 transition-colors cursor-pointer"
              >
                <ChevronDown v-if="thinkingExpanded.has(item.oi)" :size="10" />
                <ChevronRight v-else :size="10" />
                <Clock :size="11" />
                {{ thinkingLabel(item.msg, item.oi) }}
              </button>
              <div
                v-if="thinkingExpanded.has(item.oi)"
                :ref="
                  (el) => {
                    if (el) thinkingRefs[item.oi] = el as HTMLElement;
                  }
                "
                class="px-1 py-0.5 text-[11px] text-gray-400 font-mono whitespace-pre-wrap break-words max-h-40 overflow-y-auto"
              >
                {{ displayMap[item.oi]?.thinking || item.msg.thinking }}
                <span
                  v-if="
                    (displayMap[item.oi]?.thinking?.length || 0) <
                    (item.msg.thinking?.length || 0)
                  "
                  class="animate-pulse text-gray-300"
                  >▌</span
                >
              </div>
            </div>

            <!-- Tool calls for this message -->
            <div
              v-if="item.msg.toolCalls && item.msg.toolCalls.length > 0"
              class="mb-2.5 space-y-1.5"
            >
              <div
                v-for="(tc, ti) in item.msg.toolCalls"
                :key="ti"
              >
                <button
                  @click="toggleToolCall(item.oi, ti)"
                  class="w-full flex items-center gap-1 px-1 py-0.5 text-[11px] font-normal text-gray-400 hover:text-gray-600 transition-colors cursor-pointer text-left"
                >
                  <ChevronDown
                    v-if="toolExpanded.has(`${item.oi}-${ti}`)"
                    :size="10"
                  />
                  <ChevronRight v-else :size="10" />
                  <span class="flex-1 min-w-0 truncate">{{
  tc.title ? (tc.title + (tc.toolName && tc.title !== tc.toolName ? ' · ' + tc.toolName : '')) : (tc.toolName || "Tool")
}}</span>
                  <span
                    v-if="tc.status === 'started' || tc.status === 'running'"
                    class="text-gray-400 animate-pulse"
                  >
                    running{{
                      tc.startTime ? " " + elapsed(now - tc.startTime) : "..."
                    }}
                  </span>
                  <Check
                    v-else-if="tc.status === 'completed'"
                    :size="11"
                    class="text-green-500"
                  />
                  <span
                    v-else-if="tc.status === 'failed' || tc.status === 'error'"
                    class="text-red-500 text-[11px]"
                    >{{ $t("chat.toolFailed") }}</span
                  >
                  <span
                    v-if="tc.durationMs"
                    class="text-[11px] text-gray-400 ml-auto"
                    >{{ formatDuration(tc.durationMs) }}</span
                  >
                  <span
                    v-else-if="tc.status === 'completed' && tc.startTime"
                    class="text-[11px] text-gray-400 ml-auto"
                    >{{ elapsed(now - tc.startTime) }}</span
                  >
                  <!-- File explorer button for completed file operations -->
                  <button
                    v-if="
                      tc.status === 'completed' &&
                      isFileOperation(tc) &&
                      extractFilePath(tc)
                    "
                    @click.stop="openInExplorer(extractFilePath(tc)!)"
                    class="ml-1 p-1 rounded hover:bg-gray-200 text-gray-400 hover:text-gray-600 transition-colors"
                    :title="$t('chat.openInExplorer')"
                  >
                    <FolderOpen :size="11" />
                  </button>
                </button>
                <!-- Tool call details (only when expanded) -->
                <template v-if="toolExpanded.has(`${item.oi}-${ti}`)">
                  <div v-if="tc.input">
                    <div
                      class="px-1 py-0.5 text-[11px] text-gray-400 font-mono whitespace-pre-wrap break-all"
                      :class="
                        logExpanded.has(`${item.oi}-${ti}-in`)
                          ? 'max-h-[40vh] overflow-y-auto'
                          : 'line-clamp-3'
                      "
                    >
                      {{ tc.input }}
                    </div>
                    <button
                      v-if="isLongText(tc.input)"
                      @click="toggleLogDetail(`${item.oi}-${ti}-in`)"
                      class="w-full px-1 py-0.5 text-[11px] text-gray-400 hover:text-gray-500 transition-colors cursor-pointer flex items-center gap-1"
                    >
                      <ChevronUp v-if="logExpanded.has(`${item.oi}-${ti}-in`)" :size="11" />
                      <ChevronDown v-else :size="11" />
                      {{
                        logExpanded.has(`${item.oi}-${ti}-in`)
                          ? $t("chat.toolCollapse")
                          : $t("chat.toolExpand", { count: logLineCount(tc.input) })
                      }}
                    </button>
                  </div>
                  <div v-if="tc.output">
                    <div
                      class="px-1 py-0.5 text-[11px] text-gray-400 font-mono whitespace-pre-wrap break-all"
                      :class="
                        logExpanded.has(`${item.oi}-${ti}-out`)
                          ? 'max-h-[50vh] overflow-y-auto'
                          : 'line-clamp-3'
                      "
                    >
                      {{ tc.output }}
                    </div>
                    <button
                      v-if="isLongText(tc.output)"
                      @click="toggleLogDetail(`${item.oi}-${ti}-out`)"
                      class="w-full px-1 py-0.5 text-[11px] text-gray-400 hover:text-gray-500 transition-colors cursor-pointer flex items-center gap-1"
                    >
                      <ChevronUp v-if="logExpanded.has(`${item.oi}-${ti}-out`)" :size="11" />
                      <ChevronDown v-else :size="11" />
                      {{
                        logExpanded.has(`${item.oi}-${ti}-out`)
                          ? $t("chat.toolCollapse")
                          : $t("chat.toolExpand", { count: logLineCount(tc.output) })
                      }}
                    </button>
                  </div>
                </template>
              </div>
            </div>

            <!-- Interaction options -->
            <div
              v-if="
                item.msg.interaction &&
                item.msg.interaction.options.length > 0 &&
                !respondedInteractions.has(String(item.oi))
              "
              class="mb-2.5 rounded-xl border border-indigo-200 bg-indigo-50/30 overflow-hidden"
            >
              <div
                class="flex items-center gap-1.5 px-3 py-1.5 text-[12px] font-medium text-indigo-700"
              >
                <MousePointerClick :size="12" />
                <span>{{ item.msg.interaction.prompt || "Select an option" }}</span>
              </div>
              <div
                class="flex flex-wrap gap-1.5 px-3 py-2 bg-white/50 border-t border-indigo-100"
              >
                <button
                  v-for="opt in item.msg.interaction.options"
                  :key="opt.key"
                  @click="
                    handleInteractionClick(
                      item.msg.interaction!.sessionId,
                      opt.key,
                      item.oi,
                    )
                  "
                  :title="opt.label"
                  class="px-3 py-1.5 rounded-lg text-[12px] font-medium transition-all duration-150 border cursor-pointer max-w-[200px] truncate"
                  :class="
                    opt.is_default
                      ? 'bg-indigo-600 text-white border-indigo-600 hover:bg-indigo-500'
                      : 'bg-white text-gray-700 border-gray-200 hover:bg-gray-50'
                  "
                >
                  {{ truncateLabel(opt.label) }}
                </button>
              </div>
            </div>

            <!-- Permission request -->
            <div
              v-if="
                item.msg.permission &&
                item.msg.permission.options.length > 0 &&
                !respondedPermissions.has(String(item.oi))
              "
              class="mb-2.5 rounded-xl border border-gray-200 bg-gray-50/50 overflow-hidden"
            >
              <div
                class="flex items-center gap-1.5 px-3 py-1.5 text-[12px] font-medium text-gray-600"
              >
                <MousePointerClick :size="12" />
                <span>{{ item.msg.permission.prompt || "Choose an option" }}</span>
              </div>
              <div
                class="flex flex-wrap gap-1.5 px-3 py-2 bg-white/50 border-t border-gray-100"
              >
                <button
                  v-for="opt in item.msg.permission.options"
                  :key="opt.key"
                  @click="
                    handlePermissionClick(
                      item.msg.permission!.sessionId,
                      item.msg.permission!.requestId,
                      opt.key,
                      item.oi,
                    )
                  "
                  :title="opt.label"
                  class="px-3 py-1.5 rounded-lg text-[12px] font-medium transition-all duration-150 border cursor-pointer max-w-[200px] truncate"
                  :class="
                    opt.is_default
                      ? 'bg-gray-900 text-white border-gray-900 hover:bg-gray-800 dark:bg-zinc-800 dark:border-zinc-800 dark:hover:bg-zinc-700'
                      : 'bg-white text-gray-700 border-gray-200 hover:bg-gray-50'
                  "
                >
                  {{ truncateLabel(opt.label) }}
                </button>
              </div>
            </div>

            <!-- Markdown Content for this message（Memoized 子组件：html prop
                 不变时不重设 innerHTML，避免每次重渲染都重新解析整段 HTML） -->
            <MessageContent
              v-if="item.msg.content"
              :html="renderContent(item.oi, item.msg)"
              :data-msg-content="item.oi"
              @click="handleContentClick"
            />
          </template>

          <!-- Typewriter cursor for group -->
          <span
            v-if="isGroupActive(group.items)"
            class="animate-pulse text-gray-300 text-[15px]"
            >▌</span
          >

          <!-- Unknown / loading for empty group -->
          <div
            v-if="
              group.items.every(
                (it) => !it.msg.content && !it.msg.thinking,
              )
            "
            class="flex items-center gap-1 text-gray-300 text-[14px]"
          >
            <span class="animate-pulse">●</span>
            <span class="animate-pulse" style="animation-delay: 0.2s">●</span>
            <span class="animate-pulse" style="animation-delay: 0.4s">●</span>
          </div>

          <!-- ── Action bar (hover-visible) ── -->
          <div
            v-if="group.items.some((it) => !!it.msg.content)"
            class="msg-action-bar"
          >
            <!-- Status: show working timer if any msg is processing, else checkmark -->
            <template
              v-if="group.items.some((it) => it.msg.isProcessing)"
            >
              <span
                v-for="(it, _ix) in group.items"
                :key="'status-' + it.oi"
                class="text-[12px] text-gray-400"
              >
                <template v-if="it.msg.isProcessing && it.msg.startTime">
                  working {{ Math.floor((now - it.msg.startTime) / 1000) }}s
                </template>
              </span>
            </template>
            <span v-else class="flex items-center gap-1 text-green-500">
              <Check :size="12" />
            </span>
            <!-- Total duration & tokens (on last completed message) -->
            <template v-if="!group.items.some((it) => it.msg.isProcessing)">
              <span
                v-if="group.items[group.items.length - 1].msg.totalDurationMs"
                class="text-[12px] text-gray-400"
              >
                {{ formatDuration(group.items[group.items.length - 1].msg.totalDurationMs!) }}
              </span>
              <span
                v-if="group.items[group.items.length - 1].msg.inputTokens"
                class="text-[12px] text-gray-500"
              >
                in {{ formatTokens(group.items[group.items.length - 1].msg.inputTokens!) }}
              </span>
              <span
                v-if="group.items[group.items.length - 1].msg.cachedTokens && group.items[group.items.length - 1].msg.cachedTokens! > 0"
                class="text-[12px] text-green-600 font-medium"
                :title="$t('chat.cacheHit')"
              >
                ⚡ cache {{ formatTokens(group.items[group.items.length - 1].msg.cachedTokens!) }}
              </span>
              <span
                v-if="group.items[group.items.length - 1].msg.outputTokens"
                class="text-[12px] text-gray-500"
              >
                out {{ formatTokens(group.items[group.items.length - 1].msg.outputTokens!) }}
              </span>
            </template>
            <!-- Copy - copies last content in group -->
            <button
              @click="
                copyMessage(
                  group.items[group.items.length - 1].msg.content,
                  group.items[group.items.length - 1].oi,
                )
              "
              class="msg-action-btn"
              :title="$t('chat.copyMessage')"
            >
              <Check
                v-if="
                  copiedIndex ===
                  group.items[group.items.length - 1].oi
                "
                :size="12"
              />
              <Copy v-else :size="12" />
              <span>{{
                copiedIndex === group.items[group.items.length - 1].oi
                  ? "Copied"
                  : "Copy"
              }}</span>
            </button>
          </div>
        </div>
      </div>
      </template>
      </template>
    </template>
  </div>
  <!-- 非活动会话的轻量占位（见上方注释） -->
  <div v-else class="py-2" />
</template>

<style scoped>
/* ── content-visibility：让浏览器原生跳过离屏消息组的布局/渲染 ──
   IntersectionObserver 决定"是否渲染真实内容"（placeholder ↔ 完整 DOM），
   这里决定"已渲染但离屏的组不再参与布局/绘制"。KeepAlive 恢复组件时，
   离屏组不再逐个 layout——这是"重新点击会话卡几秒"的主要来源之一。
   contain-intrinsic-size: auto 140px：离屏时用浏览器记忆的上次真实尺寸占位
   （auto 关键字），滚动条高度真实、无跳变。auto 不支持的旧内核（老
   WKWebView）会整体忽略这两条声明，退化为现状——无优化但不破坏功能。 */
.msg-row {
  content-visibility: auto;
  contain-intrinsic-size: auto 140px;
}
</style>
