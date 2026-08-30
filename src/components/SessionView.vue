<script setup lang="ts">
import { ref, watch, onMounted, onBeforeUnmount, onActivated, computed, nextTick, reactive } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useRouter } from "vue-router";
import { useWorkspaceStore, type Session } from "../stores/useWorkspaceStore";
import { useMessageStore } from "../stores/useMessageStore";
import { useAgentStore } from "../stores/useAgentStore";
import { getModels, getLastAgent, setLastAgent, getAgentModels, setAgentModel, type ModelEntry, getProviderByName } from "../api/models";
import { getProviderLogo } from "../utils/providerIcons";
import { sendInput, startSession as apiStartSession, stopSession as tauriStopSession, sessionAlive, listSkills, listSessionSkills, deploySessionSkill, removeSessionSkill, type SkillInfo } from "../api/sessions";
import { saveConversationMessage, getConversationMessages, saveSession, updateSessionModel } from "../api/search";
import { getAgentStatuses, type AgentInfo } from "../api/agents";
import { recordEvent } from "../lib/diag";
import { open } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { homeDir } from "@tauri-apps/api/path";
import ChatMessages, { type Message } from "./ChatMessages.vue";
import AgentIcon from "./AgentIcon.vue";
import MentionPicker from "./MentionPicker.vue";
import { type FileEntry, parseFile } from "../api/fs";
import { submitFeedback, track } from "../api/telemetry";
import { Send, Square, Download, Shield, ChevronDown, ArrowDown, Folder, X, FolderPlus, Sparkles, HelpCircle, Plus, Package, Wand2, Paperclip, MessageCircle, Check } from "lucide-vue-next";
import { useToast } from "../composables/useToast";
import { useContextSize } from "../composables/useContextSize";
import { t, currentLocale, type TranslationKey } from "../i18n";

interface InteractionOption { key: string; label: string; is_default: boolean; }
interface AcpPayload {
  session_id: string; turn_id: string; msg_id: string;
  type: "start" | "thinking" | "text" | "tool_call" | "tool_result" | "interaction" | "permission_request" | "finish" | "error";
  content?: string; status?: string; duration?: string;
  stop_reason?: string; message?: string;
  prompt?: string; options?: InteractionOption[];
  tool_name?: string; input?: string; output?: string;
  start_time?: number; duration_ms?: number;
  request_id?: string;
  title?: string;
  input_tokens?: number;
  output_tokens?: number;
  cached_tokens?: number;
}

const props = defineProps<{
  /** Compact mode: hide text labels in permission/model selectors to save
   *  horizontal space when the chat panel is narrowed by the file tree. */
  compact?: boolean;
  /** 当前会话是否为可见会话。KeepAlive 保活的非活动 SessionView 仍在后台
   *  接收 ACP chunk，但它的 ChatMessages 需要渲染轻量占位而不是整份消息
   *  列表（避免多会话并行流式时主线程被隐藏重渲染吃满）。 */
  active?: boolean;
  /** Session ID when using KeepAlive cache. When provided, this component
   *  is dedicated to a single session and won't watch store.activeSessionId. */
  sessionId?: string;
}>();

const store = useWorkspaceStore();
const router = useRouter();
const msgStore = useMessageStore();
const agentStore = useAgentStore();
const messages = ref<Message[]>([]);

// 每个会话的 persistMessage 请求序号。连续保存（start 保存上一条 agent 消息、
// finish 保存最后一条、handleSend 保存用户消息）会并发发出多个请求，后端返回的
// 都是全量 SUM(context_chars)，但响应可能乱序到达——先发的请求后返回时会把较新
// 的 contextChars 覆盖回旧值，导致侧栏数字短暂回跳。用序号只采纳最后一次请求的
// 响应，丢弃迟到的旧响应。
let contextCharCounter = 0;
const contextCharSeq = new Map<string, number>();

/** Persist a message to the backend, then sync the session's `contextChars`
 *  so the sidebar shows an up-to-date context size for every session. */
function persistMessage(sessionId: string, role: string, content: string) {
  const seq = ++contextCharCounter;
  contextCharSeq.set(sessionId, seq);
  saveConversationMessage(sessionId, role, content)
    .then((chars) => {
      // 已有更新的请求发出（旧响应迟到）时直接丢弃，避免回写过期值
      if (contextCharSeq.get(sessionId) !== seq) return;
      const s = store.sessions.find(x => x.id === sessionId);
      if (s && typeof chars === "number") s.contextChars = chars;
    })
    .catch(() => {});
}
const unlisteners = new Map<string, UnlistenFn>();
// initSession 里注册的 msgStore 同步 watcher（用于接回旧实例写入的首条用户
// 消息），组件卸载时与 ACP 监听一并停止。
const msgSyncStops = new Map<string, () => void>();
const isSessionLoading = ref(false);

// Scroll-to-bottom state
const showScrollToBottom = ref(false);
// Floating message list state
const showMessageList = ref(false);
let hideMessageListTimer: ReturnType<typeof setTimeout> | null = null;

function handleMouseLeave() {
  hideMessageListTimer = setTimeout(() => {
    showMessageList.value = false;
  }, 150);
}

function handleMouseEnterTrigger() {
  if (hideMessageListTimer) {
    clearTimeout(hideMessageListTimer);
    hideMessageListTimer = null;
  }
  showMessageList.value = true;
}

function closeMessageList() {
  if (hideMessageListTimer) {
    clearTimeout(hideMessageListTimer);
    hideMessageListTimer = null;
  }
  showMessageList.value = false;
}

function toggleMessageList() {
  if (hideMessageListTimer) {
    clearTimeout(hideMessageListTimer);
    hideMessageListTimer = null;
  }
  showMessageList.value = !showMessageList.value;
}

// Compute user messages for the floating list
const userMessages = computed(() => {
  return messages.value
    .map((m, i) => ({ index: i, content: m.content, role: m.role }))
    .filter(m => m.role === 'user' && m.content);
});

// ═══ Smart auto-scroll (stick-to-bottom) ═══
// 只在用户位于底部附近时自动跟随滚动；用户向上滚动查看历史后暂停跟随，
// 滚回底部附近自动恢复。避免会话生成过程中被强制拉回底部、无法上翻。
const STICK_THRESHOLD = 100;
const stickToBottom = ref(true);

// scroll 事件高频触发，同步读 scrollHeight/clientHeight 会强制布局。
// 用 rAF 节流：同一帧内多次 scroll 只算一次，一次布局读取同时算出
// stickToBottom 与 showScrollToBottom。
let scrollCheckPending = false;
// 上一帧结算时的 scrollTop，用于判断用户滚动方向。
let lastScrollTop: number | null = null;
function onChatScroll() {
  if (scrollCheckPending) return;
  scrollCheckPending = true;
  // 用户一滚动就立即退出自动跟随，不等 rAF 结算：流式输出密集时，若仍保持
  // stickToBottom=true 直到下一帧结算，窗口期内到达的 contentUpdated 会把用户
  // 刚上翻的距离强制拉回底部（<100px 的滚动被吞掉、永远累积不起来），表现为
  // "生成中滚不上去，一直在最后"。同步置 false 后，后续 contentUpdated 直接
  // return，用户可自由上翻。
  stickToBottom.value = false;
  requestAnimationFrame(() => {
    scrollCheckPending = false;
    const el = messageContainer.value;
    if (!el) return;
    const distFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    const dir: "up" | "down" | null =
      lastScrollTop === null ? null : el.scrollTop < lastScrollTop ? "up" : "down";
    lastScrollTop = el.scrollTop;
    showScrollToBottom.value = distFromBottom > 100;
    // 仅当用户"向下滚回底部附近"时才恢复自动跟随。若只看距离（distFromBottom
    // <= 100），用户在底部刚上翻一点（滚动距离 < 100px）就会被判定为"回到底部"
    // 而恢复跟随，紧接着下一帧内容增长 onContentUpdated 又把用户强制拉回——
    // Windows 上表现为"底部上滑有很强的阻力、画面反复闪烁"。按方向恢复后，
    // 只要用户手势还在向上，就始终不跟随；滚回底部时才重新吸附。
    stickToBottom.value = dir === "down" && distFromBottom <= STICK_THRESHOLD;
  });
}

function scrollToMessage(msgIndex: number) {
  // 用户主动跳转到历史消息 → 暂停自动跟随（与上翻查看一致）
  stickToBottom.value = false;
  closeMessageList();
  nextTick(() => {
    const el = messageContainer.value;
    if (!el) return;
    // Find the user message element by data attribute
    // msgIndex is the index in messages array, but we need to find the group index
    el.querySelector(`[data-user-msg-index]`);
    // Since we have the message index, we need to find the corresponding group
    // The data-user-msg-index is the group index, not the message index
    // Let's find all user message elements and match by content
    const targetMsg = messages.value[msgIndex];
    if (!targetMsg) return;
    if (!scrollToUserMessage(el, targetMsg.content)) {
      // Lazy rendering: the target group may still be a placeholder (no
      // .msg-user-bubble). Force every group to render, then retry once.
      chatMessagesRef.value?.forceRenderAll();
      nextTick(() => {
        scrollToUserMessage(el, targetMsg.content);
      });
    }
  });
}

// Find the user-message element whose bubble text matches `content` and scroll to
// it. Returns true if found & scrolled, false if the element isn't rendered yet.
function scrollToUserMessage(el: HTMLElement, content: string): boolean {
  const allUserEls = el.querySelectorAll('[data-user-msg-index]');
  for (const userEl of allUserEls) {
    const textEl = userEl.querySelector('.msg-user-bubble');
    if (textEl && textEl.textContent === content) {
      userEl.scrollIntoView({ behavior: 'smooth', block: 'center' });
      return true;
    }
  }
  return false;
}

const agents = ref<AgentInfo[]>(agentStore.agents.length > 0 ? agentStore.agents : []);
const selectedAgentId = ref<string>("claude-code");

const enabledAgents = computed(() => agents.value.filter(a => a.enabled));
const selectedAgent = computed(() => agents.value.find(a => a.id === selectedAgentId.value));

// Backend agent records predating the display_name fix (or detected only via
// the DB cache) can carry an empty name — fall back to the built-in label so
// the picker never renders an icon with no text.
const builtinAgentNames: Record<string, string> = {
  "claude-code": "Claude Code",
  "codex-cli": "Codex CLI",
  "gemini-cli": "Gemini CLI",
};
function agentDisplayName(agent: { id: string; display_name: string }): string {
  return agent.display_name || builtinAgentNames[agent.id] || agent.id;
}

const modelList = ref<ModelEntry[]>(agentStore.models.length > 0 ? agentStore.models : []);
const selectedModel = ref("");
const assignedModels = ref<ModelEntry[]>([]);

// 记录每个会话的 agent 进程实际启动时使用的模型。切换模型后，运行中的
// agent 进程仍使用启动时的模型环境变量（例如 ~/.gemini/settings.json 在
// 进程启动时快照的 GEMINI_MODEL），必须重启进程新模型才会生效。
const lastUsedModel = new Map<string, string | null>();

// Onboarding state
const hasAnyAgentInstalled = computed(() => agents.value.some(a => a.installed));
const hasAnyModel = computed(() => modelList.value.length > 0);
const activeDirectory = computed(() => {
  const session = store.activeSession;
  if (!session?.directoryId) return null;
  return store.directories.find(d => d.id === session.directoryId)?.path ?? null;
});
// Cached home directory for computing default session paths. The backend uses
// ~/.runjam/session/{id} for sessions without a user-chosen directory; the
// frontend needs to know this path to manage per-session skills for those
// sessions (especially old sessions whose directory wasn't persisted).
const cachedHomeDir = ref("");

// The active session's actual working directory (cwd). For active sessions
// this comes from the backend's start_session response (session.directory).
// As a fallback for sessions where directory is null (e.g. old sessions),
// compute the default path ~/.runjam/session/{id} so skills still work.
const activeSessionCwd = computed(() => {
  const session = store.activeSession;
  if (session?.directory) return session.directory;
  if (session && cachedHomeDir.value) {
    return `${cachedHomeDir.value}/.runjam/session/${session.id}`;
  }
  return null;
});

// Cwd used by the @ mention picker. For project sessions, use session.directory.
// For plain sessions (no directory), prefer the user-selected project dir
// (dirPath via "work in a project") so @ mentions search the real project tree
// — only fall back to the session dir (agent-generated files) if nothing is
// selected. Without this, activeSessionCwd returns ~/.runjam/session/{id} for
// plain sessions, shadowing dirPath and making project files unsearchable.
const mentionCwd = computed(() => {
  const session = store.activeSession;
  if (session?.directory) return session.directory;
  return dirPath.value || activeSessionCwd.value || "";
});

const inputText = ref("");
const dirPath = ref("");
const showDirMenu = ref(false);
const showMoreAgents = ref(false);

// ── Attached files ─────────────────────────────────────────────
// Files selected via the + button; parsed to text right before send.
const attachedFiles = ref<AttachedFile[]>([]);
const showAttachList = ref(false);

interface AttachedFile {
  path: string;
  name: string;
  size: number;
  ext: string;
  parsedContent: string;
  truncated: boolean;
  error: string;
}

// File extensions accepted by the + file picker (mirrors backend parse_file).
const ATTACH_ACCEPTED_EXTS = [
  "txt", "md", "json", "csv", "log", "yaml", "yml", "xml",
  "py", "js", "jsx", "ts", "tsx", "java", "rs", "go",
  "html", "css", "scss", "less", "sh", "bash", "toml", "ini", "cfg", "conf",
  "sql", "vue", "rb", "php", "swift", "kt", "scala", "c", "cpp", "h", "hpp",
  "docx", "xlsx", "xls", "pptx", "pdf",
];

// Max total attached text sent to the LLM (100k chars) to protect context.
const MAX_ATTACH_TOTAL_CHARS = 100_000;

// ── Context size indicator ────────────────────────────────────
// Shared with the sidebar via useContextSize: the numerator is the char
// count of the conversation's message body text (content only — thinking
// blocks and tool inputs/outputs are display-only and not counted) plus the
// text currently typed in the input box; the denominator is the selected
// model's context_window × CHARS_PER_TOKEN, falling back to 200k chars when
// the model has no window configured. When the total exceeds the cap the
// send is blocked — the user has to start a new session to keep going.
const modelContextWindow = computed(() =>
  modelList.value.find(m => m.id === selectedModel.value)?.context_window ?? undefined,
);

const {
  totalChars: contextCharCount,
  maxChars: contextMaxChars,
  fillRatio: contextFillRatio,
  overLimit: contextOverLimit,
  ringColor: contextRingColor,
} = useContextSize(messages, inputText, modelContextWindow);

const contextRingLabel = computed(() => {
  const n = contextCharCount.value;
  if (n < 1_000) return `${n}`;
  if (n < 1_000_000) return `${(n / 1_000).toFixed(n < 10_000 ? 1 : 0)}k`;
  return `${(n / 1_000_000).toFixed(2)}M`;
});

const contextRingTitle = computed(() => {
  return `Context: ${contextCharCount.value.toLocaleString()} / ${contextMaxChars.value.toLocaleString()} chars`;
});

const showContextPopover = ref(false);

async function handleAttachFiles() {
  try {
    const selected = await open({
      multiple: true,
      filters: [{ name: "Supported Files", extensions: ATTACH_ACCEPTED_EXTS }],
    });
    if (!selected) return;
    const list = Array.isArray(selected) ? selected : [selected];
    const existing = new Set(attachedFiles.value.map(f => f.path));
    let addedCount = 0;
    for (const file of list) {
      if (existing.has(file)) continue;
      addedCount++;
      const name = file.split("/").pop() || file;
      const ext = name.includes(".") ? name.split(".").pop()!.toLowerCase() : "";
      let size = 0;
      try {
        size = await invoke<number>("get_file_size", { path: file });
      } catch { /* ignore */ }
      attachedFiles.value.push({
        path: file,
        name,
        size,
        ext,
        parsedContent: "",
        truncated: false,
        error: "",
      });
    }
    if (addedCount > 0) track("attach_files", { count: addedCount });
  } catch (err) {
    console.error("Failed to open file picker:", err);
  }
}

function removeAttachedFile(path: string) {
  attachedFiles.value = attachedFiles.value.filter(f => f.path !== path);
  if (attachedFiles.value.length === 0) showAttachList.value = false;
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

// Parse each attached file into plain text and build the final message.
// Failed files are skipped (with a warning); total length is capped so the
// LLM context isn't blown up by huge attachments.
async function buildAttachmentMessage(userText: string): Promise<string> {
  const parts: string[] = [];
  let failures: string[] = [];

  for (const f of attachedFiles.value) {
    try {
      const parsed = await parseFile(f.path);
      if (parsed.error) {
        f.error = parsed.error;
        failures.push(`${f.name}: ${parsed.error}`);
        continue;
      }
      f.parsedContent = parsed.content;
      f.truncated = parsed.truncated;
      parts.push(`[Attached File: ${f.name}]\n${parsed.content}`);
    } catch (err) {
      f.error = String(err);
      failures.push(`${f.name}: ${err}`);
    }
  }

  // Enforce a hard cap on the total attachment text sent to the LLM.
  let attachText = parts.join("\n\n");
  if (attachText.length > MAX_ATTACH_TOTAL_CHARS) {
    attachText = attachText.slice(0, MAX_ATTACH_TOTAL_CHARS)
      + `\n\n[Attachments truncated: content exceeds ${(MAX_ATTACH_TOTAL_CHARS / 1000).toFixed(0)}k characters]`;
  }

  attachedFiles.value = [];
  showAttachList.value = false;

  if (failures.length > 0) {
    showWarning(`Failed to parse: ${failures.join("; ")}`);
  }

  if (!attachText) return userText;
  return userText ? `${userText}\n\n${attachText}` : attachText;
}

// Skills: loaded from builtin-skills, user can toggle which ones to deploy
const availableSkills = ref<SkillInfo[]>([]);
const selectedSkills = ref<Set<string>>(new Set());
const showSkillsPopover = ref(false);
const selectedSkillNames = computed(() => Array.from(selectedSkills.value));
// While a new session is being created, the activeSessionCwd watcher would
// otherwise fire before the backend has deployed the selected skills to disk
// and clear the in-memory selection. Suppress the sync during createSession —
// the in-memory picks are authoritative at that moment.
let suppressSkillSync = false;

const otherAgents = [
  { id: "openclaw", name: "OpenClaw" },
  { id: "codebuddy", name: "CodeBuddy" },
  { id: "qoder", name: "Qoder" },
  { id: "augment", name: "Augment Code" },
  { id: "codium", name: "Codium" },
  { id: "windsurf", name: "Windsurf" },
  { id: "continue", name: "Continue" },
];

const RECENT_DIRS_KEY = "recent-project-dirs";
const MAX_RECENT = 5;

function loadRecentDirs(): string[] {
  try {
    const raw = localStorage.getItem(RECENT_DIRS_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch { return []; }
}

function saveRecentDir(p: string) {
  let dirs = loadRecentDirs();
  dirs = dirs.filter(d => d !== p);
  dirs.unshift(p);
  if (dirs.length > MAX_RECENT) dirs = dirs.slice(0, MAX_RECENT);
  localStorage.setItem(RECENT_DIRS_KEY, JSON.stringify(dirs));
}

const recentDirs = ref<string[]>(loadRecentDirs());

function selectRecentDir(p: string) {
  dirPath.value = p;
  saveRecentDir(p);
  showDirMenu.value = false;
}

function removeRecentDir(p: string) {
  const dirs = loadRecentDirs();
  const filtered = dirs.filter(d => d !== p);
  localStorage.setItem(RECENT_DIRS_KEY, JSON.stringify(filtered));
  recentDirs.value = filtered;
  if (dirPath.value === p) {
    dirPath.value = '';
  }
}

const selectedMode = ref("assistant");
const selectedPermissionMode = ref("ask_approval");
const showPermissionDropdown = ref(false);
// noThinking 提升到全局 store：WorkspaceLayout 用 activeSessionId 作 key，
// 新建会话 → 创建会话会销毁重建本组件，组件内状态会复位（新会话页开启的
// reasoning 进会话页后丢失）。store 是模块级单例，跨实例共享。
const noThinking = computed({
  get: () => store.noThinking,
  set: (v: boolean) => {
    store.noThinking = v;
  },
});

// Translated at each (re)start of the typewriter effect so switching language
// mid-session picks up the new placeholder.
const placeholderText = () => t("input.placeholderFull");
const typingPlaceholder = ref("");
let typingIndex = 0;
let typingInterval: ReturnType<typeof setInterval> | null = null;

function stopTyping() {
  if (typingInterval) {
    clearInterval(typingInterval);
    typingInterval = null;
  }
}

function startTyping() {
  stopTyping();
  typingIndex = 0;
  typingPlaceholder.value = "";
  typingInterval = setInterval(() => {
    const text = placeholderText();
    if (typingIndex < text.length) {
      typingPlaceholder.value += text[typingIndex];
      typingIndex++;
    } else {
      stopTyping();
    }
  }, 50);
}

watch(inputText, (newVal) => {
  if (newVal) {
    stopTyping();
  } else if (!typingPlaceholder.value) {
    startTyping();
  }
});

// 语言切换时，若输入框为空（新建会话页），用新语言重新播放占位符。
// 打字机播完后 typingPlaceholder 会固定为旧语言文本，仅靠上面的
// inputText watch 无法刷新，必须监听 locale 变化主动重启。
watch(
  () => currentLocale(),
  () => {
    if (!inputText.value) {
      startTyping();
    }
  }
);

// Keep the sidebar's context-size display in sync with the active session's
// input draft — the draft counts toward the context total in both places.
watch(inputText, (newVal) => {
  store.activeDraftChars = newVal.length;
});

const defaultPermissionModes: Record<string, string> = {
  "claude-code": "approve_for_me",
  "codex-cli": "approve_for_me",
  "gemini-cli": "approve_for_me",
};

async function loadPermissionMode(agentId: string): Promise<string> {
  try {
    const saved = await invoke<string>("get_agent_permission_mode", { agentId });
    if (saved) return saved;
  } catch {}
  return defaultPermissionModes[agentId] || "ask_approval";
}

async function savePermissionMode(agentId: string, mode: string) {
  try {
    await invoke("set_agent_permission_mode", { agentId, mode });
  } catch {}
}

async function loadSessionModel(agentId: string): Promise<string> {
  try {
    const saved = await invoke<string>("get_session_model", { agentId });
    if (saved) return saved;
  } catch {}
  return "";
}

async function saveSessionModel(agentId: string, modelId: string) {
  try {
    await invoke("set_session_model", { agentId, modelId });
  } catch {}
}

watch(selectedAgentId, async (newAgentId) => {
  selectedPermissionMode.value = await loadPermissionMode(newAgentId);
}, { immediate: true });

watch(selectedPermissionMode, (newMode) => {
  savePermissionMode(selectedAgentId.value, newMode);
  // Live-propagate the new mode to EVERY running session of this agent's ACP
  // client so it takes effect immediately — previously the mode was only
  // persisted and only applied on session restart, so switching out of Plan
  // Mode mid-conversation did nothing (and other sessions of the same agent
  // kept the stale snapshot too).
  for (const sess of store.sessions) {
    if (sess.cli === selectedAgentId.value && (sess.status === 'running' || sess.status === 'idle')) {
      invoke("set_session_permission_mode", { id: sess.id, mode: newMode }).catch(() => {});
    }
  }
});

const permissionModeOptions = [
  { id: "read_only" },
  { id: "ask_approval" },
  { id: "approve_for_me" },
  { id: "full_access" },
];

// Values are TranslationKey names; gemini-cli keeps its short CLI flag names
// as-is (they're not keys, so t() returns them unchanged).
const permissionLabels: Record<string, Record<string, string>> = {
  "claude-code": { read_only: "perm.plan", ask_approval: "perm.acceptEdits", approve_for_me: "perm.auto", full_access: "perm.bypass" },
  "codex-cli": { read_only: "perm.readOnlyLabel", ask_approval: "perm.askApprovalLabel", approve_for_me: "perm.approveForMeLabel", full_access: "perm.fullAccessLabel" },
  "gemini-cli": { read_only: "plan", ask_approval: "auto_edit", approve_for_me: "auto", full_access: "yolo" },
};

const permissionDescriptions: Record<string, TranslationKey> = {
  read_only: "perm.readOnly",
  ask_approval: "perm.askApproval",
  approve_for_me: "perm.approveForMe",
  full_access: "perm.fullAccess",
};

const permissionModeLabel = computed(() => {
  const raw = permissionLabels[selectedAgentId.value]?.[selectedPermissionMode.value] || selectedPermissionMode.value;
  return t(raw as TranslationKey);
});

const permissionDisplayLabels = computed(() => {
  return permissionModeOptions.map(o => ({
    ...o,
    label: t((permissionLabels[selectedAgentId.value]?.[o.id] || o.id) as TranslationKey),
    description: t(permissionDescriptions[o.id]),
  }));
});

const selectedModelInfo = computed(() => {
  return modelList.value.find(m => m.id === selectedModel.value);
});

const activeThinking = ref("");
const activeContent = ref("");
const thoughtDuration = ref("");
const isProcessing = ref(false);

const messageContainer = ref<HTMLElement | null>(null);
const chatMessagesRef = ref<{ forceRenderAll: () => void } | null>(null);
const newSessionTextarea = ref<HTMLTextAreaElement | null>(null);
const activeSessionTextarea = ref<HTMLTextAreaElement | null>(null);
const showModelDropdown = ref(false);

// ── @ Mention picker state ──────────────────────────────────
const showMentionPicker = ref(false);
const mentionAnchorIndex = ref(-1);
const mentionAnchorRect = ref<DOMRect | null>(null);
const mentionsMap = ref<Map<string, string>>(new Map()); // relativePath → absolutePath
const mentionPickerRef = ref<InstanceType<typeof MentionPicker> | null>(null);
// Search query synced from the textarea text after @. This lets the user
// type the search directly in the textarea (no focus-jump to the popup
// input needed) — the picker filters in real-time.
const mentionQuery = ref("");
const runningServerPort = ref(0);
const runningServerModel = ref<string | null>(null);
const { showWarning } = useToast();

// ── Feedback modal ──────────────────────────────────────────────
const showFeedbackModal = ref(false);
const feedbackSending = ref(false);
const feedbackDone = ref(false);
const feedbackError = ref("");
const feedbackType = ref("bug");
const feedbackContent = ref("");
const feedbackEmail = ref("");
const feedbackTypes: { id: string; labelKey: TranslationKey }[] = [
  { id: "bug", labelKey: "session.feedbackBug" },
  { id: "feature", labelKey: "session.feedbackFeature" },
  { id: "other", labelKey: "session.feedbackOther" },
];

function closeFeedback() {
  if (feedbackSending.value) return;
  showFeedbackModal.value = false;
  feedbackDone.value = false;
  feedbackError.value = "";
}

async function submitFeedbackForm() {
  const content = feedbackContent.value.trim();
  if (!content || feedbackSending.value) return;
  feedbackSending.value = true;
  feedbackError.value = "";
  try {
    await submitFeedback(
      feedbackType.value,
      content,
      feedbackEmail.value.trim() || undefined,
    );
    feedbackSending.value = false;
    feedbackDone.value = true;
    // Auto-close after showing the success state
    setTimeout(() => {
      showFeedbackModal.value = false;
      feedbackDone.value = false;
      feedbackContent.value = "";
    }, 1600);
  } catch (e) {
    feedbackSending.value = false;
    feedbackError.value = String(e);
  }
}

function getFilename(name: string): string {
  const parts = name.split('/');
  return parts[parts.length - 1];
}

async function checkServerRunning() {
  try {
    const result = await invoke<{ running: boolean; port: number; model: string | null }>("get_server_status");
    runningServerPort.value = result.running ? result.port : 0;
    runningServerModel.value = result.running ? result.model : null;
    console.log("[DEBUG] Server status:", result);
    
    // Auto-add running model to modelList if not already there
    if (result.running && result.model) {
      const modelFilename = getFilename(result.model);
      const existingModel = modelList.value.find(m => getFilename(m.name) === modelFilename && m.provider === 'llama');
      if (!existingModel) {
        console.log("[DEBUG] Auto-adding running model to modelList:", result.model);
        modelList.value.push({
          id: `llama-${result.model}-auto`,
          name: result.model,
          alias: result.model,
          provider: "llama",
          provider_name: "Llama",
          provider_icon: "llama",
          api_base: `http://localhost:${result.port}/v1`,
          api_key: "llama",
          protocol: "openai_chat",
          context_window: 0,
          support_reasoning: false,
          support_tools: true,
          tags: [],
          use_proxy: false,
        });
      }
    }
  } catch (e) {
    runningServerPort.value = 0;
    runningServerModel.value = null;
    console.log("[DEBUG] Server status check failed:", e);
  }
}

function isLocalModelRunning(model: any): boolean {
  const modelFilename = getFilename(model.name);
  const runningFilename = runningServerModel.value ? getFilename(runningServerModel.value) : '';
  return runningServerPort.value > 0 && runningFilename === modelFilename;
}

async function toggleModelDropdown() {
  showModelDropdown.value = !showModelDropdown.value;
  if (showModelDropdown.value) {
    await checkServerRunning();
    console.log("[DEBUG] Model dropdown opened:");
    console.log("  runningServerPort:", runningServerPort.value);
    console.log("  modelList:", modelList.value);
    console.log("  llama models:", modelList.value.filter(m => m.provider === 'llama'));
    console.log("  all providers:", modelList.value.map(m => m.provider));
  }
}

// session title rename
const sessionRename = ref(false);
const sessionRenameText = ref("");
function startSessionRename() { sessionRename.value = true; sessionRenameText.value = store.activeSession?.title || store.activeSession?.cliDisplayName || ""; }
function doSessionRename() {
  if (sessionRenameText.value.trim() && store.activeSession) { store.setSessionTitle(store.activeSession.id, sessionRenameText.value.trim()); }
  sessionRename.value = false;
}

function scrollToBottom() {
  // 用户手动点击"回到底部"（或切换会话）时恢复自动跟随
  stickToBottom.value = true;
  showScrollToBottom.value = false;
  // 重置方向记录：程序性回底（非用户手势）后，下一次用户滚动重新判定方向，
  // 避免用上一个会话的旧 scrollTop 比较而误判方向。
  lastScrollTop = null;
  // nextTick 确保响应式更新（如 ChatMessages 从轻量占位重建完整列表、懒渲染
  // 占位换真实内容）已 flush 到 DOM，再在下一帧读 scrollHeight —— 此时才是
  // 真实高度，否则 scrollTop=scrollHeight 会落在历史中间。仅在 rAF 回调内
  // 读一次布局，不会产生原"nextTick + rAF 双查"的双次强制布局。
  nextTick(() => {
    requestAnimationFrame(() => {
      if (messageContainer.value) {
        messageContainer.value.scrollTop = messageContainer.value.scrollHeight;
      }
    });
  });
}

// 流式生成中内容持续长高：ChatMessages 在内容尺寸变化时发 contentUpdated
// 信号，这里仅在用户停留在底部附近（stickToBottom）时跟随滚动，保证
// "定位在最后一行就持续看到最新内容"。用户上翻查看历史时不打扰（不实时
// 滚动，内容照常渲染，回到底部即见最新）。
// 注意：不能用 watch(messages) —— 流式 chunk 通过 syncMessagesToView 保持
// 同一数组引用、原地修改，浅监听数组引用变化在生成期间根本不会触发。
// ResizeObserver 回调触发时本轮内容已渲染到 DOM，直接读 scrollHeight 即可。
function onContentUpdated() {
  // 用户刚滚动、onChatScroll 的 rAF 还没结算 stickToBottom 时（scrollCheckPending），
  // 不强行拉回，避免打断用户正在进行的上翻操作；结算后若用户仍在底部会自然恢复跟随。
  if (!stickToBottom.value || scrollCheckPending) return;
  const el = messageContainer.value;
  if (el) el.scrollTop = el.scrollHeight;
}

// Effective session ID: use prop in KeepAlive mode, store otherwise
const effectiveSessionId = computed(() => props.sessionId || store.activeSessionId || '');

// 可见性：KeepAlive 模式下只有当前渲染的实例（props.sessionId 等于活动会话）
// 是可见的；无 props.sessionId（非 KeepAlive 旧路径）时本实例始终可见。
const isActiveView = computed(() => !props.sessionId || store.activeSessionId === props.sessionId);

// When using KeepAlive (props.sessionId provided), initialize on mount
// and restore messages on reactivation without re-loading from DB.
onMounted(async () => {
  if (props.sessionId) {
    await initSession(props.sessionId);
  }
});

onActivated(() => {
  if (props.sessionId) {
    const state = getSessionState(props.sessionId);
    syncMessagesToView(state);
    isProcessing.value = state.isProcessing;
    // 切换会话默认显示最新对话：始终滚动到底。原 firstTime
    // （messages.value !== state.messages）判断在"同引用"（syncMessagesToView
    // 建立，之后原地 push 不换引用）下永远为 false，导致 KeepAlive 缓存恢复
    // 时从不滚动到最新 —— 这是"切回大会话停在历史位置"的根因。
    scrollToBottom();
    // 已加载会话从缓存恢复时，ChatMessages 要从轻量占位（active=false 的
    // v-else）重建完整列表，是同步渲染，大会话有明显卡顿 —— 先给一帧 loading
    // 反馈，渲染就绪后关闭。首次挂载时 state.loaded 为 false，loading 由
    // initSession 的异步加载负责，这里不抢，避免 loading 提前消失。
    if (state.loaded) {
      isSessionLoading.value = true;
      nextTick(() => {
        requestAnimationFrame(() => {
          isSessionLoading.value = false;
        });
      });
    }
  }
});

async function initSession(sid: string) {
  activeThinking.value = ""; activeContent.value = ""; thoughtDuration.value = "";
  inputText.value = "";
  mentionsMap.value = new Map();
  closeMentionPicker();
  const state = getSessionState(sid);
  syncMessagesToView(state);
  isProcessing.value = state.isProcessing;
  isSessionLoading.value = true;
  // 防闪烁：与加载并行计时，loading 至少展示一小段；加载较慢时已 resolve，
  // 加载完成立即关闭，不会提前消失造成白屏（loading 现在覆盖整个消息区）。
  const minShow = new Promise<void>((res) => setTimeout(res, 150));
  try {
    if (!unlisteners.has(sid)) {
      // 事件监听不能阻塞会话初始化：后端 agent 进程未启动时（新建会话、
      // running:false）listen 可能一直挂起，若 await 会卡死整个 initSession，
      // finally 永不执行、isSessionLoading 永远为 true，Loading 遮罩盖住消息区，
      // 已入列的用户消息不可见（重启后 agent 已启动才能看到）。
      listen<AcpPayload>(`acp:${sid}`, (e) => handleAcpEvent(sid, e.payload))
        .then((un) => unlisteners.set(sid, un))
        .catch(() => {});
    }
    await loadSessionMessages(sid);
    // 新会话首条消息竞态：handleSend 运行在 KeepAlive 被替换前的旧实例
    //（__new__ 页或上一个会话实例）中，首条用户消息 push 进旧实例私有的
    // sessionStates（<script setup> 内声明 → 每个实例各一份），再写入跨实例
    // 共享的 msgStore。本实例的 getSessionState 在 createSession 的 key 切换
    // 瞬间创建（早于 handleSend 的写入），若只认自己那份状态，首条用户消息
    // 会永久丢失——正是"新建会话后第一条消息不显示"的根因。这里监听
    // msgStore 中本会话数组的替换并采用之；streaming 阶段 handleAcpEvent 的
    // setMessages 传入同一引用（原地 push），不会触发，无循环。
    const stopMsgSync = watch(
      () => msgStore.getMessages(sid),
      (msgs) => {
        if (!msgs || msgs.length === 0) return;
        const state = getSessionState(sid);
        if (msgs === state.messages) return;
        state.messages = msgs;
        syncMessagesToView(state);
      },
      { immediate: true }
    );
    msgSyncStops.set(sid, stopMsgSync);
    restorePendingSend(sid);
    scrollToBottom();
  } finally {
    await minShow;
    isSessionLoading.value = false;
  }
}

/**
 * After a webview reload interrupted a send (see handleSend's
 * "runjam-pending-send" marker), put the lost question back into the input box
 * so the user can re-send it. Only fires on an actual page reload AND when the
 * session has no messages yet — in the normal flow the marker is cleared by the
 * time this runs.
 */
function restorePendingSend(sid: string) {
  // Skip on the initial launch: this only applies after a reload.
  let navType = "";
  try {
    navType = (performance.getEntriesByType("navigation")[0] as PerformanceNavigationTiming | undefined)?.type || "";
  } catch {}
  if (navType !== "reload") return;
  // Fresh session only — a session with messages already survived.
  if (messages.value.length > 0) return;
  try {
    const raw = localStorage.getItem("runjam-pending-send");
    if (!raw) return;
    const pending = JSON.parse(raw);
    localStorage.removeItem("runjam-pending-send");
    // Stale marker (>60s old) or wrong target — drop it.
    if (!pending?.text || typeof pending.at !== "number") return;
    if (Date.now() - pending.at > 60_000) return;
    inputText.value = pending.text;
    console.log("[SEND] Restored pending question into input after reload (session", sid, ")");
    nextTick(() => activeSessionTextarea.value?.focus());
  } catch {}
}

// Legacy watch: used when sessionId prop is not provided (no KeepAlive)
watch(() => store.activeSessionId, async (newId) => {
  // KeepAlive 模式下 props.sessionId 恒为字符串（含 __new__ 占位 ''）。
  // 原守卫 `if (props.sessionId) return` 拦不住 ''：__new__ 实例在切换到
  // 新会话时也会注册 acp:{sid} 监听，与真实实例的 initSession 监听重复，
  // 导致同一 ACP 事件被处理两次（日志里 Received ACP session ID 出现两次）。
  // 改为 `!== undefined`：只要传入过 prop（哪怕为空串）就交给
  // onMounted/initSession 接管，本 watch 仅在真正的非 KeepAlive 用法下生效。
  if (props.sessionId !== undefined) return;
  activeThinking.value = ""; activeContent.value = ""; thoughtDuration.value = "";
  inputText.value = "";
  mentionsMap.value = new Map();
  closeMentionPicker();
  if (newId) {
    const state = getSessionState(newId);
    syncMessagesToView(state);
    isProcessing.value = state.isProcessing;
    isSessionLoading.value = true;
    if (!unlisteners.has(newId)) {
      // 与 initSession 同理：listen 可能因后端进程未就绪而挂起，
      // await 会卡住下面的 loadSessionMessages / loading 关闭。
      listen<AcpPayload>(`acp:${newId}`, (e) => handleAcpEvent(newId, e.payload))
        .then((un) => unlisteners.set(newId, un))
        .catch(() => {});
    }
    await loadSessionMessages(newId);
    scrollToBottom();
    setTimeout(() => { isSessionLoading.value = false; }, 200);
  } else {
    messages.value = [];
    isProcessing.value = false;
    isSessionLoading.value = false;
  }
}, { immediate: true });

interface SessionState {
  messages: Message[];
  activeThinking: string;
  activeContent: string;
  thoughtDuration: string;
  thinkingStartTime: number;
  isProcessing: boolean;
  loaded: boolean;
  turnStartTime: number;
  /** 发送轮次序号（单调递增）。每次新发送 +1。deadline / retry 定时器在回调里
   *  校验它：轮次已变说明定时器是上一轮的残留（该轮已 finish，或已被新一轮
   *  发送取代），其"无活动"判定失效，必须丢弃——否则会在会话早已结束后误报
   *  "Timed out ... (300s without activity)" 并把消息重发一遍。 */
  turnSeq: number;
  /** Send retry state. Set when a message is dispatched; drives the
   *  auto-retry loop (max 3 attempts) that shows "try x/3" on the chat. */
  retry: {
    attempts: number;
    lastText: string;
    timer: ReturnType<typeof setTimeout> | null;
    deadlineTimer: ReturnType<typeof setTimeout> | null;
    /** Timestamp of the original send (kept across auto-retries). Drives the
     *  2h total-session cap: retries extend the activity window but never the
     *  overall deadline. */
    startAt: number;
  } | null;
  /** True while a tool call is in flight (tool_call received, tool_result not
   *  yet). Bash/long commands emit no ACP events while running — that's normal,
   *  not a dead agent — so the no-activity timeout must not fire during this
   *  window. Cleared on tool_result / finish / error / stop / new send. */
   hasActiveTool: boolean;
   /** True once the agent has emitted ANY non-terminal event this turn
    *  (start / thinking / text / tool_call ...). Proves the agent is alive, so
    *  the deadline switches from the tight "first response" timeout to the
    *  generous active-phase timeout — a model digesting tool results and
    *  planning the next step can legitimately go silent for minutes. */
   hasStarted: boolean;
   /** True once this turn has produced REAL agent output (text / thinking /
    *  tool_call / tool_result ...). Note `hasStarted` cannot be used for this:
    *  the backend emits a synthetic Start event the moment a prompt is
    *  dispatched (session/runner.rs send_input), so it is true even when the
    *  agent never replied at all. */
   hasTurnOutput: boolean;
   }

// Max send attempts including the first try; failures auto-retry up to this.
const RETRY_MAX = 3;
const RETRY_DELAY_MS = 1000;
// 初始阶段超时：发送后 agent 还没发出任何事件（start/thinking/tool_call...）
// 达到该时长即视为失败（agent 没起来 / 连接异常）。60s 足够慢首 token 模型。
const RETRY_TIMEOUT_MS = 60_000;
// LLM 推理阶段超时：agent 已存活（收到过任意事件）且没有工具在途——此刻它在
// 调用 LLM API / 消化工具结果 / 规划下一步。单个 LLM API 调用的超时应短，
// 5 分钟无任何事件说明上游大概率卡死（慢模型通常几分钟内必出首 token）。
// 若本地慢模型仍偶发误杀，可调大此值。
const LLM_TIMEOUT_MS = 300_000;
// 整个回答过程（一次发送，含全部自动重试）的总时长硬上限：4 小时。任何阶段
// 都不得超过这个上限，到点即最终失败，不再自动重试。工具在途（长命令/构建）
// 的执行时长也由它兜底——见 startRetryDeadline。
const SESSION_TOTAL_TIMEOUT_MS = 14_400_000;

const sessionStates = new Map<string, SessionState>();
/** 组件已卸载标志。卸载后所有残留回调（deadline / retry / 迟到的 ACP 事件）
 *  都必须变成 no-op：监听已注销，它们再改动 store 或写库只会把已经完成的
 *  会话误标成失败、并把消息重发出去。 */
let unmounted = false;

function getSessionState(sessionId: string): SessionState {
  let state = sessionStates.get(sessionId);
  if (!state) {
    state = {
      // reactive()：状态数组本身是响应式 proxy。对它的 push / 原地属性
      // 修改会自动触发 messages 视图更新（Task 2 建立同引用后）。
      // reactive(proxy) 返回自身，不会二次包装。
      messages: reactive(msgStore.getMessages(sessionId) || []),
      activeThinking: "",
      activeContent: "",
      thoughtDuration: "",
      thinkingStartTime: 0,
      isProcessing: false,
      loaded: false,
      turnStartTime: 0,
      turnSeq: 0,
      retry: null,
      hasActiveTool: false,
      hasStarted: false,
      hasTurnOutput: false,
    };
    sessionStates.set(sessionId, state);
  }
  return state;
}

/**
 * 让视图消息数组与状态数组建立同一引用。状态数组是 reactive proxy，
 * 之后 handleAcpEvent 等对 state.messages 的原地修改/追加会自动触发视图
 * 更新（Vue 追踪 proxy 的数组变更），无需每次整体换数组——整体换数组会
 * 让 ChatMessages 的引用 watcher 误判为"换会话"，清空 mermaid 渲染缓存并
 * 重建 displayMap，是重新打开长会话卡顿的主因之一。
 * 引用已相同时为 no-op（O(1)），可安全地在每个事件分支调用。
 */
function syncMessagesToView(state: SessionState) {
  if (messages.value !== state.messages) {
    messages.value = state.messages as unknown as Message[];
  }
}

async function loadSessionMessages(sessionId: string) {
  const state = getSessionState(sessionId);
  if (state.loaded) return;
  try {
    const dbMessages = await getConversationMessages(sessionId);
    if (dbMessages.length > 0) {
      const loadedMessages: Message[] = dbMessages.map(m => ({
        role: m.role as "user" | "agent",
        content: m.content,
        isProcessing: false, // historical messages are never live
      }));
      state.messages = reactive(loadedMessages);
      state.loaded = true;
      msgStore.setMessages(sessionId, [...state.messages]);
      if (effectiveSessionId.value === sessionId) {
        syncMessagesToView(state);
      }
    } else {
      state.loaded = true;
    }
  } catch (err) {
    console.error("Failed to load session messages:", err);
    state.loaded = true;
  }
}

/**
 * When a new think/tool comes after text, we push a new message for linear display.
 */

/**
 * Find the last agent message that has toolCalls (for routing tool_result).
 * Falls back to the last agent message.
 */
function lastToolMsg(msgs: Message[]): Message | null {
  for (let i = msgs.length - 1; i >= 0; i--) {
    if (msgs[i].role === "agent" && msgs[i].toolCalls && msgs[i].toolCalls!.length > 0) {
      return msgs[i];
    }
  }
  return lastAgentMsg(msgs);
}

/**
 * Push a new agent message for the current phase (think or tool).
 * This ensures each think → tool sequence displays linearly rather than
 * all accumulating in the first message.
 */
function pushPhaseMessage(state: SessionState, phase: 'thinking' | 'tool') {
  // Mark all previous agent messages as done so only the latest shows "working"
  for (let i = state.messages.length - 1; i >= 0; i--) {
    if (state.messages[i].role === 'agent') {
      state.messages[i].isProcessing = false;
      break;
    }
  }
  const msg: Message = {
    role: "agent",
    content: "",
    startTime: Date.now(),
    isProcessing: true,
  };
  if (phase === 'thinking') {
    msg.thinking = "";
    msg.thoughtDuration = "";
  }
  state.messages.push(msg);
  return msg;
}

/** (Re)start the no-activity timeout for the current send attempt. If the
 *  agent goes silent (no ACP events) for RETRY_TIMEOUT_MS, the attempt is
 *  treated as failed and the retry loop takes over — this is what breaks the
 *  "infinite spinner" when the agent never reports an error. */
function startRetryDeadline(sessionId: string, state: SessionState) {
  const retry = state.retry;
  if (!retry) return;
  if (retry.deadlineTimer) clearTimeout(retry.deadlineTimer);
  // 阶段化超时：
  // - 工具在途（hasActiveTool）：工具执行允许长时间静默（长 Bash 命令/构建），
  //   只要 agent 进程还活着（running/idle）就说明任务真在执行，不设中间超时，
  //   由 SESSION_TOTAL_TIMEOUT_MS 总上限兜底；只有 agent 进程也确认不在
  //   （stopped/error）才用 RETRY_TIMEOUT_MS 快速失败——结果永远不会回来了。
  // - 已确认存活（hasStarted，收到过任意事件）：用 LLM_TIMEOUT_MS。此刻 agent
  //   在调用 LLM API / 规划下一步，单个 API 调用不应静默超过几分钟。
  // - 初始阶段（未收到任何事件）：只要 agent 进程还活着（running/idle）也用
  //   LLM_TIMEOUT_MS——慢模型首 token、上游 429 排队、长上下文重放都可能在首
  //   个事件之前超时。只有进程也确认不在（stopped/error）时才用 RETRY_TIMEOUT_MS
  //   快速失败，兜底"agent 没起来/连接失败"。
  const sess = store.sessions.find((s) => s.id === sessionId);
  const agentAlive =
    !!sess && (sess.status === "running" || sess.status === "idle");
  let timeout: number;
  if (state.hasActiveTool) {
    // 工具在途：agent 活着 → 长任务静默是正常的，直接等总上限；
    // agent 已死 → 工具结果永远不会回来，快速失败。
    timeout = agentAlive ? SESSION_TOTAL_TIMEOUT_MS : RETRY_TIMEOUT_MS;
  } else if (state.hasStarted || agentAlive) {
    timeout = LLM_TIMEOUT_MS;
  } else {
    timeout = RETRY_TIMEOUT_MS;
  }
  // 总时长硬上限：一次发送从开始到结束（含自动重试）不得超过 4 小时。到点即
  // 最终失败（isRealError=true → 不再自动重试），避免长任务无限续期。
  const elapsed = Date.now() - retry.startAt;
  if (elapsed >= SESSION_TOTAL_TIMEOUT_MS) {
    handleSendFailure(
      sessionId,
      state,
      `Response exceeded the total ${SESSION_TOTAL_TIMEOUT_MS / 3_600_000}h limit`,
      true,
    );
    return;
  }
  timeout = Math.min(timeout, SESSION_TOTAL_TIMEOUT_MS - elapsed);
  const seq = state.turnSeq;
  retry.deadlineTimer = setTimeout(() => {
    retry.deadlineTimer = null;
    // 过期定时器守卫：这一轮已经结束（收到 finish）/ 已被新一轮发送取代 /
    // 组件已卸载。它的静默计时对当前轮次毫无意义，直接丢弃——这正是"会话已经
    // 完成却又冒出 Timed out 并自动重发"的根因。
    if (unmounted || state.retry !== retry || state.turnSeq !== seq) {
      console.log(`[SEND] discard stale deadline timer (${sessionId.substring(0, 8)})`);
      return;
    }
    handleSendFailure(sessionId, state, `Timed out waiting for a response (${timeout / 1000}s without activity)`);
  }, timeout);
}

/** Push the timeout back every time the agent emits an event. */
function resetRetryDeadline(sessionId: string, state: SessionState) {
  const retry = state.retry;
  if (!retry || retry.attempts > RETRY_MAX) return;
  startRetryDeadline(sessionId, state);
}

/** Cancel all retry timers and drop the retry context (success / final failure). */
function clearRetry(state: SessionState) {
  if (!state.retry) return;
  if (state.retry.timer) { clearTimeout(state.retry.timer); state.retry.timer = null; }
  if (state.retry.deadlineTimer) { clearTimeout(state.retry.deadlineTimer); state.retry.deadlineTimer = null; }
  state.retry = null;
}

// 判定一轮"已结束"的回复是否实际失败。agent 在遇到硬错误（如 API 认证失败、
// 401/403、上游错误、内部重试耗尽）时会把错误以普通文本形式输出，随后正常发
// 送 finish 事件——若只看 finish 不看内容，会话会被标记为 idle（看板 Completed
// 列），而用户实际看到的是失败。这里用强特征模式识别这类文本。启发式，覆盖
// 常见 CLI（Claude Code 等）的错误输出格式。
const TURN_ERROR_PATTERNS = [
  /Failed to authenticate/i,
  /Incorrect API key/i,
  /invalid_api_key/i,
  /authentication_error/i,
  /Upstream \d{3}/i,
  /Request failed \(try \d+\/\d+\)/i,
];

function isFailedTurn(state: SessionState): boolean {
  const last = lastAgentMsg(state.messages);
  const text = last?.content || state.activeContent || "";
  if (!text) return false;
  return TURN_ERROR_PATTERNS.some(re => re.test(text));
}

/**
 * Handle a failed send attempt. Shows the failure on the chat (with a "try
 * x/3" label), then either auto-retries after a short delay or — after the
 * last attempt — leaves the final error visible. Never leaves the UI stuck on
 * a bare spinner.
 */
function handleSendFailure(sessionId: string, state: SessionState, errMsg: string, isRealError = false) {
  // 组件已卸载（KeepAlive 淘汰 / 离开工作区）：残留回调不得再写 store / 数据库，
  // 也不得重发消息——它已经收不到任何事件，重试只会被再次判定超时。
  if (unmounted) return;
  const isActiveSession = store.activeSessionId === sessionId;
  // 本轮是否已经产出过内容（text / thinking / tool_call / tool_result）。
  const producedOutput = state.hasTurnOutput;
  // 任何失败/超时都终止当前工具等待：工具要么已结束，要么这轮请求已放弃。
  state.hasActiveTool = false;
  state.hasStarted = false;
  state.hasTurnOutput = false;
  state.isProcessing = false;
  state.thinkingStartTime = 0;
  for (const m of state.messages) {
    if (m.role === 'agent') m.isProcessing = false;
  }
  // Remove empty "start" placeholder so "..." dots disappear
  const le = lastAgentMsg(state.messages);
  if (le && !le.content && !le.thinking) {
    state.messages.pop();
  }

  const retry = state.retry;
  // A real error reported by the agent (ACP error event) is FINAL — never
  // auto-retry it, even if we're still within the retry budget. The upstream
  // failure (e.g. 401 auth) won't fix itself and re-sending only spawns
  // another round of silent retries inside the agent. Only no-activity
  // timeouts are retryable.
  //
  // 已经产出过内容的轮次同样不自动重发：agent 那边可能只是没发 finish
  // （或本地慢模型尾部静默超时），重发等于让它把整轮任务（含 Bash / 写文件
  // 类工具）再执行一遍，比报一次超时危害大得多。只有"完全没回应"才值得重试。
  // 兜底补挂的"无文本"retry（lastText=''，见 handleAcpEventInner）也没有可重发的
  // 内容，直接走最终失败路径，避免空文本被当作消息发出去。
  const canAutoRetry = !isRealError && !producedOutput && !!retry?.lastText;
  if (canAutoRetry && retry && retry.attempts < RETRY_MAX) {
    const attempt = retry.attempts;
    state.messages.push({
      role: "agent",
      content: `⚠️ Request failed (try ${attempt}/${RETRY_MAX}): ${errMsg}，${RETRY_DELAY_MS / 1000}s retring ...`,
      isProcessing: true,
    });
    if (isActiveSession) {
      syncMessagesToView(state);
      msgStore.setMessages(sessionId, messages.value);
    }
    retry.attempts = attempt + 1;
    if (retry.deadlineTimer) { clearTimeout(retry.deadlineTimer); retry.deadlineTimer = null; }
    const seq = state.turnSeq;
    retry.timer = setTimeout(() => {
      retry.timer = null;
      retrySend(sessionId, state, retry, seq);
    }, RETRY_DELAY_MS);
    return;
  }

  // Final failure — surface the real error and give up retrying. Persist it
  // regardless of whether this session is currently active, so switching back
  // to this session still shows the error (loadSessionMessages reads from DB).
  // attempts 从 1 起算（含首次发送），实际重发次数 = attempts - 1。
  const retried = retry ? retry.attempts - 1 : 0;
  const finalContent = `Error: ${errMsg}` + (retried > 0 ? `（已自动重试 ${retried} 次）` : "");
  state.messages.push({
    role: "agent",
    content: finalContent,
  });
  clearRetry(state);
  persistMessage(sessionId, "agent", finalContent);
  // Final failure (retries exhausted or a real ACP error): mark the session as
  // error so the board shows it under "Failed" instead of leaving it stuck.
  const sess = store.sessions.find(s => s.id === sessionId);
  if (sess && (sess.status === 'running' || sess.status === 'idle')) {
    sess.status = 'error';
    sess.newlyCompleted = true;
    store.sessions = [...store.sessions];
    saveSession(sessionId, sess.cli, sess.cliDisplayName, sess.title, sess.directory || "", "error", sess.pid, sess.pinned ? 1 : 0, sess.archived ? 1 : 0, sess.acpSessionId).catch(() => {});
  }
  msgStore.setMessages(sessionId, state.messages as unknown as Message[]);
  if (isActiveSession) {
    syncMessagesToView(state);
    isProcessing.value = false;
  }
}

/** Re-send the last failed text (attempts N+1). */
function retrySend(sessionId: string, state: SessionState, retry: NonNullable<SessionState["retry"]>, seq: number) {
  // 过期回调：发送轮次已被取代（新的发送 / finish 后重发）或组件已卸载 —— 不
  // 再重发，否则会话会在已经收到完整回复之后又被跑一遍。
  if (unmounted || state.retry !== retry || state.turnSeq !== seq) return;
  // 兜底补挂的无文本 retry（lastText=''）不能重发：没有内容可发，静默放弃即可
  // （handleSendFailure 已在其无 lastText 时走最终失败路径，正常情况下不会到这里）。
  if (!retry.lastText) return;
  const sess = store.sessions.find(s => s.id === sessionId);
  if (!sess || sess.status === 'stopped' || sess.status === 'error') {
    clearRetry(state);
    return;
  }
  state.isProcessing = true;
  state.turnStartTime = Date.now();
  // 重试是新一轮请求：等新的首事件，恢复初始 60s 超时
  state.hasActiveTool = false;
  state.hasStarted = false;
  state.hasTurnOutput = false;
  if (store.activeSessionId === sessionId) isProcessing.value = true;
  startRetryDeadline(sessionId, state);
  sendInput(sessionId, retry.lastText, undefined).catch((err: unknown) => {
    handleSendFailure(sessionId, state, `Send failed: ${err}`);
  });
}

function handleAcpEvent(sessionId: string, p: AcpPayload) {
  // Drop events for sessions the user already stopped or deleted — a few
  // buffered lines can still arrive in the window between Stop and the process
  // actually dying.
  const sess = store.sessions.find(s => s.id === sessionId);
  if (!sess || sess.status === 'stopped') {
    // 事件会被丢弃 → 还在等待的 retry 定时器永远等不到"活动"，必须在此时清掉，
    // 否则它会在超时后给一个已经停止的会话追加 "Error: Timed out ..."。
    const st = sessionStates.get(sessionId);
    if (st) clearRetry(st);
    return;
  }
  const t0 = performance.now();
  try {
    handleAcpEventInner(sessionId, p);
  } finally {
    recordEvent(performance.now() - t0, p.content?.length || 0);
  }
}

// ── 高频 ACP 事件日志节流 ──
// ACP chunk 是逐 token 的高频事件，Gemini 单轮可上万条。每 chunk 一次
// console.log + JSON.stringify 会引发 WKWebView 主线程的字符串分配与
// console I/O（unified-log syscalls），是"运行会话时 Web Content CPU 100%"
// 的主因。现在默认静默：仅按节流窗口聚合打一条计数摘要；需要逐事件
// 完整日志时在 URL 上加 ?debug=acp 打开（信息与之前一致，仅调试期）。
const ACP_EVENT_LOG_THROTTLE_MS = 500;
let lastAcpEventLogTime = 0;
let acpEventLogCount = 0;
const acpDebugEnabled = typeof window !== "undefined" &&
  /[?&]debug=acp/.test(window.location.search);

function logAcpEvent(sessionId: string, p: AcpPayload) {
  if (acpDebugEnabled) {
    console.log(`[ACP EVENT] ${sessionId.substring(0, 8)} type=${p.type}`, {
      type: p.type,
      content: p.content?.substring(0, 100),
      thinking: p.type === 'thinking' ? p.content?.substring(0, 100) : undefined,
      tool_name: p.tool_name,
      tool_status: p.status,
      input: p.input?.substring(0, 100),
      output: p.output?.substring(0, 100),
      stop_reason: p.stop_reason,
      error: p.message,
    });
    return;
  }
  acpEventLogCount++;
  const now = performance.now();
  if (now - lastAcpEventLogTime < ACP_EVENT_LOG_THROTTLE_MS) return;
  lastAcpEventLogTime = now;
  console.log(`[ACP EVENT] ${sessionId.substring(0, 8)} type=${p.type} (${acpEventLogCount} events since last log)`);
  acpEventLogCount = 0;
}

function handleAcpEventInner(sessionId: string, p: AcpPayload) {
  const state = getSessionState(sessionId);
  const isActiveSession = store.activeSessionId === sessionId;
  // Any live event (thinking/text/tool, etc.) means the agent is still working —
  // push back the no-activity timeout that backs the auto-retry loop. finish
  // and error are handled by their own branches.
  if (p.type !== "finish" && p.type !== "error") {
    // 收到任意非终态事件 → agent 确认存活，超时切换到宽松的活跃阶段
    state.hasStarted = true;
    // start 不算"有产出"：send_input 在派发瞬间就会合成一条 Start，agent 是否
    // 真的回过消息要看 text / thinking / tool_call / tool_result。
    if (p.type !== "start") state.hasTurnOutput = true;
    if (state.retry) {
      resetRetryDeadline(sessionId, state);
    } else {
      // 兜底补挂重试：本轮发送由非规范实例发出（新会话首条消息从 __new__ 页发出，
      // handleSend 跑在旧实例上，它不挂 retry——见 handleSend）。本实例收到首个
      // 事件即证明发送已发生，补一个"无文本"重试（lastText='' → 超时后只报错、
      // 绝不自动重发，避免把整轮任务再跑一遍）。仅当会话处于 running 时补挂：
      // finish 后 status 已为 idle，迟到的杂散事件不会误挂（例如 Gemini 的多条
      // message 一轮里穿插 finish 的边界情况）。
      const sess = store.sessions.find(s => s.id === sessionId);
      if (sess && sess.status === 'running') {
        state.retry = { attempts: 1, lastText: "", timer: null, deadlineTimer: null, startAt: Date.now() };
        startRetryDeadline(sessionId, state);
      }
    }
  }
  // 仅活动会话打日志且经节流（见 logAcpEvent）——每 chunk 的 console.log
  // + JSON.stringify 在 WKWebView 主线程上是真实开销，多个后台会话并行
  // 流式时尤其明显（统计仍由 diag 记录）
  if (isActiveSession) {
    logAcpEvent(sessionId, p);
  }

  switch (p.type) {
    case "start":
      // Save previous agent message before resetting activeContent.
      // Gemini sends multiple messages per turn; agent_message_end emits
      // Start to begin a new bubble. Without saving here, only the last
      // message would be persisted (or none if activeContent was reset).
      if (state.activeContent && sessionId) {
        persistMessage(sessionId, "agent", state.activeContent);
      }
      state.messages.push({ role: "agent", content: "", startTime: Date.now(), isProcessing: true });
      state.activeThinking = ""; state.activeContent = ""; state.thoughtDuration = "";
      state.thinkingStartTime = 0;
      // Only set turnStartTime on the real turn start (when it's 0).
      // agent_message_end emits Start to create a new bubble, but we must
      // NOT reset the turn timer — otherwise totalDurationMs in finish
      // only reflects the last message, not the whole turn.
      if (state.turnStartTime === 0) {
        state.turnStartTime = Date.now();
      }
      state.isProcessing = true;
      if (isActiveSession) {
        syncMessagesToView(state);
        isProcessing.value = true;
        msgStore.setMessages(sessionId, messages.value);
      }
      break;
    case "thinking":
      // If the last message already has content or tools, push a new message
      // so this thinking block displays linearly as a separate entry
      const lastThinkCheck = lastAgentMsg(state.messages);
      if (lastThinkCheck && (lastThinkCheck.content || (lastThinkCheck.toolCalls && lastThinkCheck.toolCalls.length > 0))) {
        pushPhaseMessage(state, 'thinking');
        state.activeThinking = "";
        state.thoughtDuration = "";
        state.thinkingStartTime = 0;
      }

      if (p.content) {
        // Track thinking start time on first chunk
        if (state.thinkingStartTime === 0) {
          state.thinkingStartTime = Date.now();
        }
        // Gemini sends each agent_thought_chunk as a FULL SNAPSHOT of the
        // thought text so far (its stream frames carry the complete thought,
        // not incremental deltas). Appending unconditionally would duplicate
        // the same thought repeatedly ("A"+"AB"+"ABC" → "AABABC").
        // Claude/Codex send incremental deltas instead. Heuristic that works
        // for both: if the new chunk starts with the already-accumulated text,
        // treat it as a snapshot and REPLACE; otherwise APPEND (delta).
        // Use trimEnd() to tolerate trailing whitespace differences between
        // snapshots (same fix as the text handler above).
        const prev = state.activeThinking;
        const prevTrimmed = prev.trimEnd();
        if (prevTrimmed && p.content.length > prevTrimmed.length && p.content.startsWith(prevTrimmed)) {
          state.activeThinking = p.content; // snapshot — replace
        } else {
          state.activeThinking += p.content; // delta — append
        }
        const l = ensureAgentMsg(state);
        l.thinking = state.activeThinking;
      }
      if (p.status==="done") {
        // agent_thought_end: freeze the thinking timer
        if (state.thinkingStartTime > 0) {
          state.thoughtDuration = formatDuration(Date.now() - state.thinkingStartTime);
          state.thinkingStartTime = 0;
        } else {
          state.thoughtDuration = p.duration || state.thoughtDuration;
        }
        const l = ensureAgentMsg(state);
        l.thoughtDuration = state.thoughtDuration;
      } else if (p.duration) {
        state.thoughtDuration = p.duration;
        const l = ensureAgentMsg(state);
        l.thoughtDuration = p.duration;
      }
      if (isActiveSession) {
        syncMessagesToView(state);
        msgStore.setMessages(sessionId, messages.value);
      }
      break;
    case "text":
      // Handle ACP session ID notification (special marker)
      if (p.content && p.content.startsWith("__ACP_SESSION_ID__")) {
        const acpSessionId = p.content.substring("__ACP_SESSION_ID__".length);
        console.log(`[ACP] Received ACP session ID: ${acpSessionId}`);
        const s = store.sessions.find(s => s.id === sessionId);
        if (s) {
          s.acpSessionId = acpSessionId;
          saveSession(sessionId, s.cli, s.cliDisplayName, s.title, s.directoryId ? store.directories.find(d => d.id === s.directoryId)?.path || "" : "", s.status, s.pid, s.pinned ? 1 : 0, s.archived ? 1 : 0, acpSessionId).catch(() => {});
        }
        break;
      }
      // Transition from thinking to text: freeze the thinking timer
      if (state.thinkingStartTime > 0) {
        state.thoughtDuration = formatDuration(Date.now() - state.thinkingStartTime);
        state.thinkingStartTime = 0;
        const lt2 = ensureAgentMsg(state);
        lt2.thoughtDuration = state.thoughtDuration;
      }
      // Gemini agent_message_chunk sends the FULL message text each time
      // (snapshot), not incremental deltas. Claude also sends snapshots but
      // with inconsistent trailing whitespace between chunks (e.g. "properly. "
      // then "properly.I'll"), which would break a naive startsWith check and
      // cause cumulative duplication. Trim trailing whitespace from the
      // accumulated text before comparing so snapshot detection is robust.
      // Claude/Codex deltas don't start with the accumulated text, so they
      // still take the append branch correctly.
      const prev = state.activeContent;
      const prevTrimmed = prev.trimEnd();
      if (prevTrimmed && p.content && p.content.length > prevTrimmed.length && p.content.startsWith(prevTrimmed)) {
        state.activeContent = p.content;
      } else {
        state.activeContent += (p.content||"");
      }
      // 新一轮文本的开始（activeContent 为空，通常是被 tool_call 清空后）：
      // 若最后一条 agent 消息是纯工具消息（有 toolCalls、无 content），push
      // 新消息承载本段文本，避免文本落进工具消息形成混合气泡。
      const lastForText = lastAgentMsg(state.messages);
      if (!prev && lastForText && lastForText.toolCalls && lastForText.toolCalls.length > 0 && !lastForText.content) {
        pushPhaseMessage(state, 'tool');
      }
      const lt = ensureAgentMsg(state);
      lt.content = state.activeContent;
      if (isActiveSession) {
        syncMessagesToView(state);
        msgStore.setMessages(sessionId, messages.value);
      }
      break;
    case "tool_call": {
      // 有工具正在执行：等待 tool_result 期间豁免无活动超时（长命令正常静默）
      state.hasActiveTool = true;
      // 文本回复结束、转入工具调用：清空 activeContent，避免下一轮文本 chunk
      // 追加到上一轮残留文本上（codex 在每轮文本与工具调用之间不发
      // agent_message_end → 前端 start 事件不触发 → activeContent 不会自动
      // 重置；不重置的话每轮文本都会叠加历史文本，形成"雪球式重复"）。
      state.activeContent = "";
      // If the last message already has text content, push a new message for this tool
      const lastToolCheck = lastAgentMsg(state.messages);
      if (lastToolCheck && lastToolCheck.content) {
        pushPhaseMessage(state, 'tool');
      }

      const tc = ensureAgentMsg(state);
      if (!tc.toolCalls) tc.toolCalls = [];
      const toolName = p.tool_name || "";
      const isRunning = p.status === "running";

      if (isRunning) {
        // tool_call_update (running) — update existing entry, don't push a new one
        let found = false;
        for (let i = tc.toolCalls.length - 1; i >= 0; i--) {
          const existing = tc.toolCalls[i];
          if ((existing.status === "started" || existing.status === "running") && existing.toolName === toolName) {
            // Update input if we got new info
            if (p.input) existing.input = p.input;
            if (p.title) existing.title = p.title;
            existing.status = "running";
            found = true;
            break;
          }
        }
        if (!found) {
          // Fallback: push as new
          tc.toolCalls.push({
            toolName,
            input: p.input || "",
            status: "running",
            startTime: p.start_time,
            title: p.title,
          });
        }
      } else {
        // tool_call (started) — push new entry
        tc.toolCalls.push({
          toolName,
          input: p.input || "",
          status: p.status || "started",
          startTime: p.start_time,
          title: p.title,
        });
      }
      if (isActiveSession) {
        syncMessagesToView(state);
        msgStore.setMessages(sessionId, messages.value);
      }
      // 进入工具执行阶段：按工具的长超时重算 deadline。上面的通用分支在
      // hasActiveTool 置位前执行，用的还是 LLM 短超时，必须在这里切换。
      if (state.retry) resetRetryDeadline(sessionId, state);
      break;
    }
    case "tool_result": {
      // 工具返回，解除超时豁免；之后的静默按正常规则计时
      state.hasActiveTool = false;
      // Use lastToolMsg to find the message that has toolCalls (not the last agent msg)
      // This ensures tool_result goes to the right message even if a new thinking
      // message was pushed after the tool_call
      const tr = lastToolMsg(state.messages) || ensureAgentMsg(state);
      if (tr.toolCalls && tr.toolCalls.length > 0) {
        // Find the last tool call with matching tool_name that's still started/running
        const toolName = p.tool_name || "";
        let found = false;
        for (let i = tr.toolCalls.length - 1; i >= 0; i--) {
          const tc = tr.toolCalls[i];
          if (tc.status === "started" || tc.status === "running") {
            if (!toolName || tc.toolName === toolName) {
              tc.output = p.output || "";
              // Detect tool failure: check output for known error indicators
              const output = (p.output || "").toLowerCase();
              const isFailed = (
                (output.includes("error:") || output.includes("failed:")) &&
                !output.includes("completed with no output")
              );
              tc.status = isFailed ? "failed" : "completed";
              if (p.duration_ms !== undefined) {
                tc.durationMs = p.duration_ms;
              }
              if (p.title) tc.title = p.title;
              found = true;
              break;
            }
          }
        }
        // Fallback: if no matching running tool, update the last one
        if (!found) {
          const last = tr.toolCalls[tr.toolCalls.length - 1];
          last.output = p.output || "";
          last.status = "completed";
          if (p.duration_ms !== undefined) {
            last.durationMs = p.duration_ms;
          }
        }
      }
      if (isActiveSession) {
        syncMessagesToView(state);
        msgStore.setMessages(sessionId, messages.value);
      }
      // 工具已返回：解除长超时豁免，按 LLM 短超时重算 deadline（agent 接下来
      // 要消化工具结果并调用下一次 LLM API）。
      if (state.retry) resetRetryDeadline(sessionId, state);
      break;
    }
    case "permission_request": {
      // Each permission request gets its own dedicated message so multiple
      // simultaneous permissions (e.g. WebSearch + WebFetch) don't overwrite each other
      state.messages.push({
        role: "agent",
        content: "",
        isProcessing: true,
        permission: {
          requestId: p.request_id || "",
          prompt: p.prompt || "",
          options: p.options || [],
          sessionId: sessionId,
        },
      });
      if (isActiveSession) {
        syncMessagesToView(state);
        msgStore.setMessages(sessionId, messages.value);
      }
      break;
    }
    case "interaction": {
      // Agent is asking the user to choose from options
      const im = ensureAgentMsg(state);
      // If there's a fresh interaction, we create it as a new mini-message-like block
      // But since interaction can come mid-stream, attach to last agent message
      const currentSid = effectiveSessionId.value || sessionId;
      im.interaction = {
        prompt: p.prompt || "",
        options: p.options || [],
        sessionId: currentSid,
      };
      if (isActiveSession) {
        syncMessagesToView(state);
        msgStore.setMessages(sessionId, messages.value);
      }
      break;
    }
    case "finish":
      clearRetry(state);
      state.hasActiveTool = false;
      state.hasStarted = false;
      state.hasTurnOutput = false;
      state.isProcessing = false;
      state.thinkingStartTime = 0;
      // Mark all agent messages in this turn as not processing
      for (const m of state.messages) {
        if (m.role === 'agent') m.isProcessing = false;
      }
      // Remove trailing empty message (Gemini agent_message_end emits Start
      // which pushes a new empty bubble; the last one has no content).
      {
        const last = lastAgentMsg(state.messages);
        if (last && !last.content && !last.thinking && (!last.toolCalls || last.toolCalls.length === 0)) {
          state.messages.pop();
        }
      }
      // Attach total duration and token count to the last agent message
      {
        const last = lastAgentMsg(state.messages);
        if (last) {
          if (state.turnStartTime > 0) {
            last.totalDurationMs = Date.now() - state.turnStartTime;
          }
          if (p.input_tokens) {
            last.inputTokens = p.input_tokens;
          }
          if (p.output_tokens) {
            last.outputTokens = p.output_tokens;
          }
          if (p.cached_tokens) {
            last.cachedTokens = p.cached_tokens;
          }
          const total = (p.input_tokens || 0) + (p.output_tokens || 0);
          if (total > 0) {
            last.totalTokens = total;
          }
        }
      }
      state.turnStartTime = 0;
      // Save the last agent message if activeContent is non-empty.
      // For Gemini, earlier messages are already saved in the start handler
      // (triggered by agent_message_end → Start). The last message may or may
      // not have a trailing agent_message_end; if it does, activeContent is
      // empty here (already saved). If not, activeContent holds the last
      // message and we save it now.
      if (sessionId && state.activeContent) {
        persistMessage(sessionId, "agent", state.activeContent);
      }
      // 会话轮次结束：清空累积缓冲，防止残留到下一轮（下一轮 text chunk 会
      // 追加到旧内容上形成雪球）。activeContent 已被 persistMessage 固化或
      // 为空，重置是安全的。
      state.activeContent = "";
      state.activeThinking = "";
      // Mark session as idle (process still alive, waiting for next message),
      // or as error if this turn actually ended in a hard failure (e.g. auth /
      // API error surfaced as plain text before the finish event).
      const sess = store.sessions.find(s => s.id === sessionId);
      if (sess && sess.status === 'running') {
        sess.status = isFailedTurn(state) ? 'error' : 'idle';
        sess.newlyCompleted = true;
        // 回复完成未读语义：无论用户是否正打开着该会话，完成即标记为"新完成
        // 未读"→ 看板 Completed 列。直到用户点击查看（selectSession 会置
        // unread=false）才转入 History 列。若按 activeSessionId 判断，用户自己
        // 发消息时（会话恒为 active）回复完成后会直接进 History，Completed 列
        // 形同虚设。
        sess.unread = true;
        store.sessions = [...store.sessions];
        saveSession(sessionId, sess.cli, sess.cliDisplayName, sess.title, sess.directory || "", sess.status, sess.pid, sess.pinned ? 1 : 0, sess.archived ? 1 : 0, sess.acpSessionId).catch(() => {});
      }
      if (isActiveSession) {
        syncMessagesToView(state);
        isProcessing.value = false;
        msgStore.setMessages(sessionId, messages.value);
      }
      break;
    case "error": {
      // Log unconditionally (even for background sessions) so a missed error
      // shows up in devtools without requiring this session to be active.
      console.log(`[ACP ERROR EVENT] ${sessionId.substring(0, 8)}:`, p.message);
      handleSendFailure(sessionId, state, p.message || "Unknown", true);
      break;
    }
  }
}

function lastAgentMsg(msgs: Message[]): Message|null { 
  for(let i=msgs.length-1;i>=0;i--) if(msgs[i].role==="agent")return msgs[i]; 
  return null; 
}

/** Ensure there's an agent message to attach content to. Returns it. */
function ensureAgentMsg(state: SessionState): Message {
  let m = lastAgentMsg(state.messages);
  if (!m) {
    m = { role: "agent", content: "", startTime: Date.now(), isProcessing: true };
    state.messages.push(m);
    state.isProcessing = true;
  }
  return m;
}

function formatDuration(ms: number): string {
  const s = Math.floor(ms / 1000);
  if (s < 60) return s + 's';
  return Math.floor(s / 60) + 'm ' + (s % 60) + 's';
}

function closeDropdowns(e: MouseEvent) {
  const target = e.target as HTMLElement;
  if (!target.closest('.permission-selector') && !target.closest('.model-selector') && !target.closest('.dir-selector') && !target.closest('.more-agents-selector') && !target.closest('.message-list-dropdown') && !target.closest('.skills-selector') && !target.closest('.context-ring')) {
    showPermissionDropdown.value = false;
    showModelDropdown.value = false;
    showDirMenu.value = false;
    showMoreAgents.value = false;
    showSkillsPopover.value = false;
    showContextPopover.value = false;
    closeMessageList();
  }
}

// Keep local agents ref in sync with the store (updated by Settings pages after install/uninstall)
watch(() => agentStore.agents, (newAgents) => {
  if (newAgents.length > 0) {
    agents.value = newAgents;
  }
});

onMounted(() => {
  startTyping();
  getLastAgent().then(id => { if(id) selectedAgentId.value = id; }).catch(()=>{});
  // Cache home dir so we can compute default session paths (~/.runjam/session/{id})
  // for per-session skill management on sessions without an explicit directory.
  homeDir().then(h => { cachedHomeDir.value = h; }).catch(() => {});
  // Load built-in skills catalog. Per-session selection is synced separately
  // by the activeSessionCwd watcher (so it reflects the active session's disk).
  listSkills().then(skills => {
    availableSkills.value = skills;
  }).catch(() => {});
  if (agents.value.length === 0) {
    getAgentStatuses().then(list => { 
      if(list) { 
        agents.value = list; 
        agentStore.agents = list; // also update store so all views stay in sync
      }
    }).catch(()=>{});
  }
  if (modelList.value.length === 0) {
    getModels().then(list => { if(list) modelList.value = list; }).catch(()=>{});
  }
  loadAgentModels();
  checkServerRunning();
  document.addEventListener('click', closeDropdowns);
  // Auto-focus the textarea on the new session page
  nextTick(() => {
    if (!store.activeSession && newSessionTextarea.value) {
      newSessionTextarea.value.focus();
    }
  });
});

onBeforeUnmount(() => {
  // 监听注销后本实例再也收不到 ACP 事件，任何残留的 deadline / 重发定时器
  // 都只会误判"无活动"→ 报超时 → 自动重发消息（并写库把已完成会话标成失败）。
  // 必须在卸载时全部停掉（KeepAlive 超过 max 淘汰、路由离开都会走到这里）。
  unmounted = true;
  for (const [, st] of sessionStates) {
    clearRetry(st);
  }
  stopTyping();
  for (const [_, unlisten] of unlisteners) {
    try { unlisten(); } catch {}
  }
  unlisteners.clear();
  for (const stop of msgSyncStops.values()) {
    try { stop(); } catch {}
  }
  msgSyncStops.clear();
  document.removeEventListener('click', closeDropdowns);
});

async function loadModels() { try{modelList.value=await getModels();}catch{modelList.value=[];} }
async function loadAgentModels() {
  try { assignedModels.value = await getAgentModels(selectedAgentId.value); } catch { assignedModels.value = []; }
}
watch(() => store.activeSession, async (session) => { 
  // In KeepAlive mode, only react when the active session matches this instance
  if (props.sessionId && session?.id !== props.sessionId) return;
  if (session) {
    selectedAgentId.value = session.cli;
    if (session.model) {
      selectedModel.value = session.model;
    }
  } else {
    // Auto-focus textarea when returning to the new session page
    nextTick(() => {
      newSessionTextarea.value?.focus();
    });
  }
}, { immediate: true });

watch(selectedAgentId, async (id) => { 
  try{await setLastAgent(id);}catch{} 
  await loadModels(); 
  await loadAgentModels();
  // If the active session carries its own model, honor it over the per-agent
  // default — otherwise opening an old session would silently fall back to the
  // last globally-selected model (e.g. a local model) and break it.
  const activeModel = store.activeSession?.model;
  if (activeModel && modelList.value.some(m => m.id === activeModel)) {
    selectedModel.value = activeModel;
    return;
  }
  const savedModel = await loadSessionModel(id);
  if (savedModel && modelList.value.some(m => m.id === savedModel)) {
    selectedModel.value = savedModel;
  } else if (assignedModels.value.length > 0) {
    selectedModel.value = assignedModels.value[0].id;
  }
}, { immediate: true });

// Sync selectedSkills with the active session's deployed skills on disk.
// Each session stores its skills in its own working directory, so switching
// sessions must re-read from that session's skills folder — otherwise skill
// selections would leak across sessions (one session's picks showing in others).
watch(activeSessionCwd, async (cwd) => {
  if (suppressSkillSync) { console.log("[SKILL-SYNC] suppressed during createSession"); return; }
  const session = store.activeSession;
  console.log("[SKILL-SYNC] watcher fired", { cwd, sessionId: session?.id, cli: session?.cli, hasDir: !!session?.directory });
  if (cwd && session) {
    try {
      const names = await listSessionSkills(cwd, session.cli);
      console.log("[SKILL-SYNC] listSessionSkills returned", { sessionId: session.id, cwd, cli: session.cli, names });
      // Guard against a stale async result if the user switched away meanwhile.
      if (store.activeSession?.id === session.id) {
        selectedSkills.value = new Set(names);
      } else {
        console.log("[SKILL-SYNC] stale result, user switched away");
      }
    } catch (err) {
      console.error("[SKILL-SYNC] Failed to load session skills:", err);
    }
  } else {
    // Either no active session (new-session page) or an active session whose
    // cwd is still unknown (homeDir not yet cached). Clear to prevent the
    // previous session's skills from leaking in; skills will reload once the
    // cwd becomes available.
    console.log("[SKILL-SYNC] clearing selectedSkills (no cwd or no session)");
    selectedSkills.value = new Set();
  }
}, { immediate: true });

watch(messages, async () => {
  // 仅当用户还停留在底部附近时才跟随滚动——上翻查看历史时不打扰。
  // 每次更新都整体替换数组（handleAcpEvent/loadSessionMessages 均
  // `messages.value = [...]`），浅监听即可，避免 deep 在流式热路径上
  // 遍历整个数组。
  if (!stickToBottom.value) return;
  await nextTick();
  if (messageContainer.value) {
    messageContainer.value.scrollTop = messageContainer.value.scrollHeight;
  }
});

// ── @ Mention picker functions ──────────────────────────────

function currentTextarea(): HTMLTextAreaElement | null {
  return store.activeSession ? activeSessionTextarea.value : newSessionTextarea.value;
}

function onTextareaInput(e: Event) {
  const ta = e.target as HTMLTextAreaElement;
  const pos = ta.selectionStart;
  const val = ta.value;
  // Detect @ just typed
  if (pos > 0 && val[pos - 1] === "@") {
    const prev = pos >= 2 ? val[pos - 2] : "";
    // Only trigger at start of line or after whitespace (not mid-word/email)
    if (prev === "" || prev === "\n" || /\s/.test(prev)) {
      mentionAnchorIndex.value = pos - 1;
      mentionAnchorRect.value = ta.getBoundingClientRect();
      showMentionPicker.value = true;
      mentionQuery.value = "";
      return;
    }
  }
  // Sync search query & close popup when appropriate
  if (showMentionPicker.value) {
    if (pos <= mentionAnchorIndex.value) {
      closeMentionPicker();
      return;
    }
    const segment = val.slice(mentionAnchorIndex.value + 1, pos);
    if (/\s/.test(segment)) {
      closeMentionPicker();
    } else {
      // Sync the text after @ as the live search query
      mentionQuery.value = segment;
    }
  }
}

function onTextareaKeydown(e: KeyboardEvent) {
  if (!showMentionPicker.value) {
    // Popup closed: handle Enter as send (original behavior)
    if (e.key === "Enter" && !e.shiftKey && !e.altKey && !e.ctrlKey && !e.metaKey) {
      e.preventDefault();
      handleSend();
    }
    return;
  }
  // Popup open: intercept navigation keys and forward to MentionPicker
  if (["ArrowDown", "ArrowUp", "Enter", "Escape", "Tab"].includes(e.key)) {
    e.preventDefault();
    e.stopPropagation();
    mentionPickerRef.value?.handleKeydown(e);
  }
}

function onMentionSelect(entry: FileEntry) {
  const cwd = mentionCwd.value;
  if (!cwd) {
    closeMentionPicker();
    return;
  }
  // Compute relative path for display
  let rel = entry.path;
  if (entry.path.startsWith(cwd)) {
    rel = entry.path.slice(cwd.length).replace(/^\/+/, "");
  } else {
    // Fallback: just the filename
    rel = entry.name;
  }
  const displayToken = `@${rel}`;
  const start = mentionAnchorIndex.value;
  // Replace only the "@query" portion (from @ through the search text typed
  // in the textarea), preserving any text that followed the cursor.
  const queryLen = mentionQuery.value.length;
  const end = start + 1 + queryLen;
  inputText.value =
    inputText.value.slice(0, start) + displayToken + inputText.value.slice(end);
  // Store mapping: relativePath → absolutePath
  mentionsMap.value.set(rel, entry.path);
  closeMentionPicker();
  // Focus textarea and move caret after the token
  nextTick(() => {
    const ta = currentTextarea();
    if (ta) {
      const caret = start + displayToken.length;
      ta.focus();
      ta.setSelectionRange(caret, caret);
    }
  });
}

function closeMentionPicker() {
  showMentionPicker.value = false;
  mentionAnchorIndex.value = -1;
  mentionAnchorRect.value = null;
  mentionQuery.value = "";
}

async function handleSend() {
  const wasNewSession = !store.activeSession;
  let text = inputText.value.trim(); if(!text)return;

  // Expand @relativePath mentions to @absolutePath for the LLM
  for (const [rel, abs] of mentionsMap.value) {
    text = text.split(`@${rel}`).join(`@${abs}`);
  }
  mentionsMap.value = new Map();

  if (!selectedModel.value) {
    showWarning("Please select a model before sending.");
    return;
  }

  // Block send when the accumulated context has exceeded the per-session cap.
  // The user has to start a new session to keep going.
  if (contextOverLimit.value) {
    showWarning("Context limit reached. Please start a new session to continue.");
    return;
  }

  inputText.value = "";

  // Persist the pending question before starting a session. If the webview is
  // reloaded mid-send (dev HMR full-reload from agent files landing in the
  // project tree), the backend session survives but the send is lost — the
  // restored session page can then bring the text back into the input box.
  try {
    localStorage.setItem("runjam-pending-send", JSON.stringify({ text, at: Date.now() }));
  } catch {}

  // 发送给 LLM 的文本：含附件解析出的完整内容
  let sendText = text;
  let attachNames: string[] = [];
  if (attachedFiles.value.length > 0) {
    attachNames = attachedFiles.value.map(f => f.name);
    sendText = await buildAttachmentMessage(text);
  }
  // 用户消息显示：原文 + 附件文件名列表（不暴露文件内容）
  const userDisplay = attachNames.length > 0
    ? `${text}\n\n${attachNames.map(n => `📎 ${n}`).join("\n")}`
    : text;

  if(!store.activeSession) {
    const a=agents.value.find(a=>a.id===selectedAgentId.value)!;
    const title = text.substring(0, 30) + (text.length > 30 ? '...' : '');
    // Suppress the activeSessionCwd watcher during creation: it would fire
    // before the backend deploys skills to disk and clear the in-memory picks.
    suppressSkillSync = true;
    try {
      await store.createSession(a.id, agentDisplayName(a), dirPath.value||undefined, title, selectedModel.value || undefined, selectedMode.value, selectedPermissionMode.value, Array.from(selectedSkills.value));
      // 记住该会话进程启动时用的模型（创建时可能尚未选模型 → null，
      // 首次发送时会用空标记触发进程重启，确保用上正确模型）。
      const created = store.activeSession as Session | null;
      if (created) {
        lastUsedModel.set(created.id, created.model || null);
      }
    } catch (err) {
      // Session failed to start — show error and abort send.
      showWarning(`Failed to start session: ${err}`);
      // Restore the input text so the user can retry.
      inputText.value = text;
      // The session is kept (marked "error") so the UI stays on the session
      // page; enqueue the user message (plus a visible error) so the first
      // message of a brand-new session is never silently lost.
      if (store.activeSessionId) {
        const sid = store.activeSessionId;
        const state = getSessionState(sid);
        state.messages.push({ role: "user", content: userDisplay });
        state.messages.push({ role: "agent", content: `Error: Failed to start session: ${err}`, isProcessing: false });
        syncMessagesToView(state);
        msgStore.setMessages(sid, [...state.messages]);
        persistMessage(sid, "user", userDisplay);
      }
      return;
    } finally {
      suppressSkillSync = false;
    }
  } else if (store.activeSession.status === 'stopped' || store.activeSession.status === 'error') {
    // Restart backend only if process is truly dead. After a webview reload
    // (dev HMR full-reload from agent files landing in the project tree) the
    // backend process usually survives — only the webview reloaded. Restarting
    // it anyway would wipe its in-process session memory, so the next message
    // lands in an empty context ("I only received '1'"). Check liveness first.
    const s = store.activeSession;
    const alive = await sessionAlive(s.id);
    if (!alive) {
      // Process is truly gone — start a fresh one. freshAgentProcess makes the
      // send flow below inject the recent conversation as context.
      try {
        await apiStartSession(s.cli, s.cliDisplayName, s.directory || dirPath.value || undefined, s.id, s.model || undefined, selectedMode.value, selectedPermissionMode.value, Array.from(selectedSkills.value));
      } catch (err) {
        console.error("Failed to restart session:", err);
        // Show error in chat instead of calling sendInput (which would fail with "No client for session")
        const state = getSessionState(s.id);
        state.messages.push({role:"user",content:userDisplay});
        state.messages.push({role:"agent",content:`Error: Failed to restart session. ${err}`});
        syncMessagesToView(state);
        msgStore.setMessages(s.id, [...state.messages]);
        persistMessage(s.id, "user", userDisplay);
        return;
      }
      s.freshAgentProcess = true;
    }
    s.status = 'running';
    store.sessions = [...store.sessions];
    // 加载历史消息用于恢复会话上下文
    await loadSessionMessages(s.id);
  }

  if (store.activeSession?.id) {
    // 用户消息入列。放在模型操作（setAgentModel / ensureAgentProcessUsesModel，
    // 可能耗时或抛错）之前，保证这条消息无论后续发生什么都一定显示在会话里——
    // 新会话创建后视图实例刚切换，任何一次异常都可能导致它丢失。
    const sid = effectiveSessionId.value;
    // Bump the session's last-active timestamp on the backend so the sidebar
    // reorders this session to the top, even after a page reload. The
    // backend's save_message also touches it, but that's not awaited and the
    // optimistic local update here is what drives the immediate sidebar sort.
    store.touchSession(sid);
    const state = getSessionState(sid);
    // 用户发起新一轮：清空上一轮的文本/思考累积缓冲（若 finish 未及时清，
    // 这里兜底，防止新回复的 chunk 追加到旧文本上）。
    state.activeContent = "";
    state.activeThinking = "";
    state.messages.push({role:"user",content:userDisplay});
    syncMessagesToView(state);
    msgStore.setMessages(sid, [...state.messages]);
    persistMessage(sid, "user", userDisplay);
  }

  if (store.activeSession?.id) {
    const sessionId = effectiveSessionId.value;
    if (store.activeSession.status === 'idle') {
      store.activeSession.status = 'running';
      store.sessions = [...store.sessions];
    }
    invoke("set_reasoning_disabled", { disabled: !noThinking.value }).catch(() => {});
    // The agent's on-disk model config is shared per-agent (e.g. ~/.claude/
    // settings.json), not per-session. Re-apply THIS session's own model before
    // sending so switching models in another session doesn't leak into this one.
    const sessionModel = store.activeSession.model || selectedModel.value;
    if (sessionModel) {
      // 先把模型写入 agent 配置文件（重启后的新进程会读到最新值），
      // 必须等待写入完成再重启进程
      await setAgentModel(store.activeSession.cli, sessionModel).catch(() => {});
      // 若进程启动时用的模型与本次不同，运行中的进程仍使用启动时的旧模型
      //（进程环境变量在启动时固化，如 gemini 的 GEMINI_MODEL），必须重启
      // 进程才能让新模型生效。重启会把 freshAgentProcess 置为 true，下面的
      // 历史注入就会带上最近对话，新进程因此仍保有上下文。重启失败不阻塞
      // 发送（旧进程可能仍可用），只记录错误。
      try {
        await ensureAgentProcessUsesModel(sessionId, sessionModel);
      } catch (err) {
        console.error("ensureAgentProcessUsesModel failed:", err);
      }
    }
    // 上下文注入：仅当 Agent 进程是刚启动的（freshAgentProcess）才需要把历史
    // 消息拼进第一条 prompt。必须在模型处理（可能重启进程）之后计算——否则进程
    // 刚被重启而历史已经消费掉，新进程收到空上下文（“I only received '1'”）。
    // 用户消息已在上面入列，这里先排除最后一条（当前消息），避免它被重复计入
    // 历史（"Previous conversation: ...user: 1... --- New message: 1"）。
    let history: string[] | undefined = undefined;
    if (store.activeSession?.freshAgentProcess) {
      const sessionMsgs = msgStore.getMessages(sessionId);
      const prevMsgs = sessionMsgs.length > 0 ? sessionMsgs.slice(0, -1) : sessionMsgs;
      if (prevMsgs.length > 0) {
        // 最多取最后2轮对话（4条消息：user/agent/user/agent）
        const recentMsgs = prevMsgs.slice(-4);
        history = recentMsgs
          .filter(m => m.content)
          .map(m => `${m.role}: ${m.content}`);
      }
      // 历史已取出，标记为非新进程
      store.activeSession.freshAgentProcess = false;
      store.sessions = [...store.sessions];
    }
    // Arm the auto-retry loop before dispatching: the first failure shows
    // "try 1/3" on the chat and re-sends after a short delay, up to RETRY_MAX
    // attempts total. The timeout also catches agents that go silent without
    // ever reporting an error (otherwise the UI hangs on a bare spinner).
    const st = getSessionState(sessionId);
    // 新一轮发送：先彻底清掉上一轮的 retry（含挂起的 deadline / 重发定时器），
    // 否则旧定时器会残留到这一轮，在其"无活动"窗口到点后误报超时并重发消息。
    clearRetry(st);
    st.turnSeq++;
    // 新一轮发送：清除上一轮的挂起工具标记（重试/新消息都会重新派发 tool_call）
    st.hasActiveTool = false;
    st.hasStarted = false;
    st.hasTurnOutput = false;
    // 只允许"本会话的规范实例"挂重试定时器。新会话的首条消息从 __new__ 页发出时，
    // handleSend 运行在即将被 KeepAlive 替换掉的旧实例上（props.sessionId=''），它
    // 监听的是 acp: 空通道，永远收不到该会话的事件——在这里挂定时器既无法被事件
    // 续期、也无法被 finish 清除，300s 后必然误报超时并自动重发消息（"会话已完成
    // 却又冒出 try 1/3 → 2/3 → Error" 的根因，claude/gemini/codex 均复现）。
    // 该轮的重试由规范实例（props.sessionId === sessionId，真实监听 acp:{sid}）
    // 在收到首个事件时兜底补挂（见 handleAcpEventInner）。
    if (props.sessionId === undefined || props.sessionId === sessionId) {
      st.retry = { attempts: 1, lastText: sendText, timer: null, deadlineTimer: null, startAt: Date.now() };
      startRetryDeadline(sessionId, st);
    }
    // Telemetry: a message was dispatched (fire-and-forget, never blocks).
    track("message_sent", {
      attachCount: attachNames.length,
      model: store.activeSession?.model || selectedModel.value || "",
      newSession: wasNewSession,
    });
    sendInput(sessionId, sendText, history).catch(err=>{
      const state = getSessionState(sessionId);
      clearRetry(state);
      state.messages.push({role:"agent",content:`Error:${err}`});
      state.isProcessing = false;
      if (store.activeSessionId === sessionId) {
        syncMessagesToView(state);
        isProcessing.value = false;
      }
      msgStore.setMessages(sessionId, [...state.messages]);
    });
    // The send was dispatched (or errored) — drop the reload-recovery marker.
    try { localStorage.removeItem("runjam-pending-send"); } catch {}
  }
}

// 若指定会话的 agent 进程启动时使用的模型与期望不一致，则重启进程让
// 新模型生效（agent 进程的模型环境变量在启动时固化，改配置文件不会影响
// 已运行进程）。重启后 freshAgentProcess 会把最近的对话作为上下文注入。
async function ensureAgentProcessUsesModel(sessionId: string, sessionModel: string) {
  const s = store.activeSession;
  if (!s || !sessionModel) return;
  const used = lastUsedModel.get(sessionId);
  // 仅在“明确知道”进程用的是不同模型时才重启。lastUsedModel 是内存 Map，
  // webview 刷新（dev 下 agent 文件落盘会触发 vite 整页重载）后被清空——
  // 此时 used 为 undefined。若照旧重启，会杀掉仍健康、且内存里带着完整会话
  // 上下文的进程，下一次消息就落进一个空上下文的新会话（“I only received…”）。
  // 无记录时假定运行中的进程用的就是会话持久化的模型（它本来就是用它启动的）。
  if (used !== undefined && used !== sessionModel && s.status !== 'stopped' && s.status !== 'error') {
    console.log(`[MODEL-CHANGE] ${sessionId}: ${used ?? '(none)'} -> ${sessionModel}, restarting agent process`);
    try { await tauriStopSession(sessionId); } catch {}
    // tauriStopSession 直接杀进程，不产生 finish 事件：若被杀时消息还在
    // 处理中，其 isProcessing 会残留 true（hasLiveActivity 永远为 true，
    // ChatMessages 持续重渲染/滚动）。重启前清干净。
    const st = sessionStates.get(sessionId);
    if (st) {
      for (const m of st.messages) {
        if (m.isProcessing === true) m.isProcessing = false;
      }
      syncMessagesToView(st);
    }
    await apiStartSession(s.cli, s.cliDisplayName, s.directory || dirPath.value || undefined, sessionId, sessionModel, selectedMode.value, selectedPermissionMode.value, Array.from(selectedSkills.value));
    s.status = 'running';
    s.freshAgentProcess = true;
    store.sessions = [...store.sessions];
  }
  lastUsedModel.set(sessionId, sessionModel);
}

function handleModelSelect(model: ModelEntry) {
  selectedModel.value = model.id;
  showModelDropdown.value = false;

  track("model_changed", { model: model.id });
  saveSessionModel(selectedAgentId.value, model.id);
  // Remember the model per-session (persisted to DB) so this selection only
  // affects the current session, not every session of the same agent.
  if (store.activeSession) {
    store.activeSession.model = model.id;
    updateSessionModel(store.activeSession.id, model.id).catch(() => {});
  }
  // Immediately update agent config so the agent uses the right model name
  setAgentModel(selectedAgentId.value, model.id).catch(() => {});
}

// Clicking anywhere in the input box container (except buttons / interactive
// elements) focuses the textarea, so the user doesn't have to hit the field
// itself. Interactive targets (buttons, the skills popover, mention picker,
// dropdowns, links) are ignored so their own click handlers win.
function focusInputOnContainerClick(e: MouseEvent) {
  const target = e.target as HTMLElement;
  if (target.closest("button, a, [data-copy], .skills-selector, .mention-picker, input, select")) return;
  const ta = store.activeSession ? activeSessionTextarea.value : newSessionTextarea.value;
  if (ta) ta.focus();
}

async function toggleSkill(name: string) {
  const session = store.activeSession;
  const cwd = session ? activeSessionCwd.value : null;
  console.log("[SKILL-TOGGLE]", { name, sessionId: session?.id, cwd, cli: session?.cli, sessionDir: session?.directory });
  track("skill_toggled", { skill: name, enabled: !selectedSkills.value.has(name) });
  if (session && cwd) {
    // Active session: persist the change to this session's own skills
    // directory so it is isolated per session and survives reloads.
    const isOn = selectedSkills.value.has(name);
    try {
      if (isOn) {
        await removeSessionSkill(cwd, session.cli, name);
      } else {
        await deploySessionSkill(cwd, session.cli, name);
      }
    } catch (err) {
      // Keep the UI in sync with the disk — don't toggle on failure.
      showWarning(`Failed to update skill: ${err}`);
      return;
    }
    const next = new Set(selectedSkills.value);
    if (isOn) next.delete(name); else next.add(name);
    selectedSkills.value = next;
  } else {
    // New-session page: in-memory only; deployed when the session starts.
    const next = new Set(selectedSkills.value);
    if (next.has(name)) next.delete(name); else next.add(name);
    selectedSkills.value = next;
  }
}

async function handleStop() {
  if (store.activeSession) {
    track("message_stopped");
    const sid = effectiveSessionId.value;
    const state = getSessionState(sid);
    clearRetry(state);
    state.hasActiveTool = false;
    state.hasStarted = false;
    state.hasTurnOutput = false;
    state.isProcessing = false;
    // Clear isProcessing on ALL agent messages, not just the last one. A
    // crashed/stopped session can leave earlier messages stuck at
    // isProcessing=true, which would keep ChatMessages.hasLiveActivity true and
    // drive the `now` tick (and a full list re-render) every 500ms forever.
    for (const m of state.messages) {
      if (m.role === 'agent') {
        m.isProcessing = false;
        // Also settle any tool calls stuck in "started"/"running" — they'd
        // otherwise keep hasLiveActivity true the same way. Mark them "failed"
        // (not "completed"): the session was stopped, so the tool did not
        // actually finish, and showing a green "completed" check would mislead
        // (e.g. a write_file that was killed).
        if (m.toolCalls) {
          for (const tc of m.toolCalls) {
            if (tc.status === "started" || tc.status === "running") {
              tc.status = "failed";
            }
          }
        }
      }
    }
    isProcessing.value = false;
    syncMessagesToView(state);
    msgStore.setMessages(sid, [...state.messages]);
    await store.stopSession(sid);
  }
}

// 兜底清理残留的 live 标记。正常停止/完成路径（handleStop / finish /
// handleSendFailure）都会清掉消息上的 isProcessing 与 running toolCalls；
// 但存在不经过它们的停止路径（进程被外部 kill 后 finish 事件丢失、模型切换
// 时 tauriStopSession 直接杀进程等），消息会残留 isProcessing=true，让
// ChatMessages.hasLiveActivity 永远为 true：now tick 每 500ms 驱动全列表
// 重渲染，"working Xs" 持续变化，配合 ResizeObserver→contentUpdated 表现为
// "会话没在跑了，消息区却一直往下滚"。会话一被标记为 stopped/error 就立即
// 兜底清理本实例缓存的状态（后台 KeepAlive 实例的 watch 同样存活，各自清理）。
// 正常路径下这是 no-op（dirty=false），无副作用。
watch(
  () => store.sessions.map(s => `${s.id}:${s.status}`).join("\n"),
  () => {
    for (const s of store.sessions) {
      if (s.status !== "stopped" && s.status !== "error") continue;
      const state = sessionStates.get(s.id);
      if (!state) continue;
      let dirty = false;
      for (const m of state.messages) {
        if (m.isProcessing === true) {
          m.isProcessing = false;
          dirty = true;
        }
        if (m.toolCalls) {
          for (const tc of m.toolCalls) {
            if (tc.status === "started" || tc.status === "running") {
              tc.status = "failed";
              dirty = true;
            }
          }
        }
      }
      if (dirty) {
        syncMessagesToView(state);
        msgStore.setMessages(s.id, [...state.messages]);
      }
    }
  },
);

async function pickDirectory() {
  const selected = await open({ directory: true, multiple: false });
  if (selected && typeof selected === "string") {
    dirPath.value = selected;
    saveRecentDir(selected);
    recentDirs.value = loadRecentDirs();
  }
  showDirMenu.value = false;
}

// Sync messages to message store for search.
// 流式 chunk 是原地 mutate（lt.content = ...），deep watch 每 chunk 触发；
// 全量数组拷贝 + store 替换会引发搜索订阅方每次 chunk 的重算。用 rAF 把
// 同步合并到每帧至多一次（60Hz 上限），高 chunk 频率时显著节省拷贝开销。
let pendingSyncMsgs: Message[] | null = null;
let pendingSyncSid = "";
let syncRafHandle: number | null = null;
watch(messages, (msgs) => {
  const sid = effectiveSessionId.value;
  if (!sid) return;
  pendingSyncMsgs = msgs;
  pendingSyncSid = sid;
  if (syncRafHandle !== null) return;
  syncRafHandle = requestAnimationFrame(() => {
    syncRafHandle = null;
    if (pendingSyncMsgs) {
      const batch = pendingSyncMsgs;
      const batchSid = pendingSyncSid;
      pendingSyncMsgs = null;
      msgStore.setMessages(batchSid, [...batch]);
    }
  });
}, { deep: true });
</script>

<template>
  <main class="flex-1 flex flex-col min-h-0">
    <template v-if="store.activeSession">
      <div class="flex-shrink-0">
        <div class="max-w-4xl mx-auto px-5 py-2">
          <div class="flex items-center gap-3">
            <AgentIcon :agent-id="store.activeSession.cli" />
            <span v-if="sessionRename" class="text-[13px] font-semibold text-gray-900 flex-1 min-w-0">
              <input v-model="sessionRenameText" @blur="doSessionRename" @keydown.enter="doSessionRename" @click.stop @mousedown.stop
                class="bg-transparent border-b border-gray-300 outline-none text-[13px] font-semibold w-full max-w-[400px]" />
            </span>
            <span v-else @click="startSessionRename" class="text-[13px] font-semibold text-gray-900 cursor-pointer hover:text-gray-600 truncate max-w-[400px]">{{ store.activeSession.title || store.activeSession.cliDisplayName }}</span>
            
            <span class="flex-1" />
          <span v-if="activeDirectory" class="text-[11px] text-gray-400 truncate max-w-[300px] select-none" :title="activeDirectory">{{ activeDirectory }}</span>
          </div>
        </div>
      </div>
      
      <div class="flex-1 relative min-h-0">
        <div ref="messageContainer" class="h-full overflow-y-auto chat-scrollbar-hidden" @scroll="onChatScroll">
          <div class="max-w-4xl mx-auto px-6 pt-5 pb-40">
            <ChatMessages ref="chatMessagesRef" :messages="messages" :agent-id="selectedAgentId" :active="isActiveView" @content-updated="onContentUpdated" />
          </div>
        </div>

        <!-- Loading overlay: 切换/加载会话时覆盖消息区。不能依赖
             messages.length === 0 —— 已加载过的会话消息在内存（msgStore /
             sessionStates），切回时 messages 非空，原条件永远不显示。 -->
        <div v-if="isSessionLoading" class="absolute inset-0 flex items-center justify-center bg-white/70 z-10">
          <div class="flex items-center gap-2 text-gray-400">
            <div class="w-4 h-4 border-2 border-gray-300 border-t-gray-600 rounded-full animate-spin"></div>
            <span class="text-[13px]">{{ $t("session.loading") }}</span>
          </div>
        </div>

        <!-- Scroll-to-bottom button -->
        <button
          v-if="showScrollToBottom"
          @click="scrollToBottom()"
          class="absolute bottom-6 left-1/2 -translate-x-1/2 w-9 h-9 rounded-full bg-white border border-gray-200 shadow-md flex items-center justify-center text-gray-500 hover:text-gray-700 hover:border-gray-300 hover:shadow-lg transition-all duration-200 cursor-pointer z-10"
          :title="$t('session.scrollToBottom')"
        >
          <ArrowDown :size="16" />
        </button>

        <!-- Right-side message list strip -->
        <div
          v-if="userMessages.length > 0"
          class="absolute right-0 top-1/2 -translate-y-1/2 z-10 message-list-dropdown"
          @mouseenter="handleMouseEnterTrigger"
          @mouseleave="handleMouseLeave"
        >
          <!-- Message list panel (hover area extends through gap) -->
          <div
            v-if="showMessageList"
            class="absolute right-full top-1/2 -translate-y-1/2 w-72 max-h-[70vh] overflow-y-auto bg-white rounded-xl shadow-xl border border-gray-100 z-50 py-1"
            style="margin-right: 8px;"
            @click.stop
            @mouseenter="handleMouseEnterTrigger"
          >
            <div class="px-3 py-2 flex items-center justify-between border-b border-gray-100">
              <span class="text-[11px] font-semibold text-gray-500 uppercase tracking-wider">{{ $t("board.messages") }}</span>
              <span class="text-[10px] text-gray-400">{{ $t("session.itemsCount", { count: userMessages.length }) }}</span>
            </div>
            <button
              v-for="msg in userMessages"
              :key="msg.index"
              @click="scrollToMessage(msg.index)"
              class="w-full text-left px-3 py-2.5 text-[12px] text-gray-700 hover:bg-gray-50 transition-colors cursor-pointer border-b border-gray-50 last:border-b-0"
            >
              <span class="line-clamp-2 leading-relaxed">{{ msg.content }}</span>
            </button>
          </div>
          
          <!-- Trigger button -->
          <button
            @click.stop="toggleMessageList"
            :class="[
              'relative flex flex-col items-center justify-center gap-1.5 py-3 px-1.5 rounded-l-xl bg-white/85 backdrop-blur-sm border border-r-0 border-gray-200 shadow-sm text-gray-500 hover:text-gray-800 hover:bg-white transition-all duration-200 cursor-pointer',
              showMessageList ? 'bg-white text-gray-800 shadow-md dark:bg-gray-200 dark:shadow-none' : ''
            ]"
            :title="$t('session.messageList')"
            @mouseenter="handleMouseEnterTrigger"
          >
            <span
              v-for="i in 4"
              :key="i"
              class="block h-[2px] w-4 rounded-full bg-current opacity-70"
            ></span>
          </button>
        </div>
      </div>
      
      <div class="flex-shrink-0">
        <div class="max-w-4xl mx-auto px-4 py-3">
          <div @click="focusInputOnContainerClick" class="relative rounded-2xl border border-gray-200 bg-white focus-within:border-gray-300 shadow-[0_2px_12px_rgba(0,0,0,0.06)] focus-within:shadow-[0_4px_16px_rgba(0,0,0,0.08)] transition-all duration-150">
            <!-- Skills tags row: same UI as new-session page -->
            <div class="skills-selector flex items-center gap-1.5 px-3 pt-2.5 pb-1.5 min-h-[34px]">
              <div class="flex items-center gap-1.5 overflow-x-auto flex-1 min-w-0" style="scrollbar-width: none; -ms-overflow-style: none;">
                <span
                  v-for="name in selectedSkillNames" :key="name"
                  @click="toggleSkill(name)"
                  class="inline-flex items-center gap-1 px-2 py-0.5 rounded-md text-[11px] bg-gray-100 text-gray-700 cursor-pointer hover:bg-gray-200 transition-colors flex-shrink-0"
                >
                  {{ name }}
                  <X :size="9" />
                </span>
              </div>
              <button
                @click.stop="showSkillsPopover = !showSkillsPopover"
                :disabled="availableSkills.length === 0"
                :class="[
                  'inline-flex items-center justify-center w-6 h-6 rounded-md transition-colors cursor-pointer flex-shrink-0',
                  availableSkills.length > 0
                    ? showSkillsPopover
                      ? 'bg-gray-100 text-gray-700'
                      : 'text-gray-400 hover:bg-gray-100 hover:text-gray-600'
                    : 'text-gray-200 cursor-not-allowed',
                ]"
                :title="$t('session.skills')"
              >
                <Wand2 :size="13" />
              </button>
            </div>

            <!-- Skills popover: card grid, stays open for multi-select -->
            <div
              v-if="showSkillsPopover && availableSkills.length > 0"
              class="absolute bottom-full left-0 right-0 mb-1 bg-white rounded-xl border border-gray-100 shadow-lg z-50 overflow-hidden"
            >
              <div class="flex items-center justify-between px-3 py-2 border-b border-gray-50">
                <span class="text-[11px] font-medium text-gray-500">{{ $t("session.skillsCount", { count: selectedSkills.size, total: availableSkills.length }) }}</span>
                <button @click.stop="showSkillsPopover = false" class="w-5 h-5 rounded flex items-center justify-center text-gray-400 hover:bg-gray-100 hover:text-gray-600 cursor-pointer transition-colors">
                  <X :size="12" />
                </button>
              </div>
              <div class="p-2 grid grid-cols-2 gap-1.5 max-h-[260px] overflow-y-auto">
                <div
                  v-for="skill in availableSkills" :key="skill.name"
                  @click="toggleSkill(skill.name)"
                  :title="skill.description"
                  :class="[
                    'px-2.5 py-2 rounded-lg cursor-pointer transition-all border',
                    selectedSkills.has(skill.name)
                      ? 'bg-gray-900 border-gray-900 text-white dark:bg-zinc-800 dark:border-zinc-800'
                      : 'bg-white border-gray-100 hover:border-gray-200 hover:bg-gray-50 text-gray-700',
                  ]"
                >
                  <div class="text-[13px] font-medium leading-tight mb-0.5 truncate">{{ skill.name }}</div>
                  <div :class="['text-[10px] leading-snug line-clamp-3', selectedSkills.has(skill.name) ? 'text-gray-300' : 'text-gray-400']">{{ skill.description }}</div>
                </div>
              </div>
            </div>

            <textarea v-model="inputText" ref="activeSessionTextarea" :placeholder="typingPlaceholder" rows="2" class="w-full px-4 pt-2 bg-transparent border-none outline-none resize-none text-[14px] text-gray-900 leading-relaxed" @input="onTextareaInput" @keydown="onTextareaKeydown" :disabled="isProcessing" />
            <div class="flex items-center justify-between px-3 pb-2 gap-2">
              <div class="relative flex items-center gap-1.5 flex-shrink-0">
                <button
                  @click="handleAttachFiles"
                  :disabled="isProcessing"
                  class="p-1.5 rounded-lg text-gray-400 hover:bg-gray-100 hover:text-gray-600 transition-colors duration-150 cursor-pointer flex-shrink-0"
                  :class="isProcessing && 'opacity-50 cursor-not-allowed'"
                  title="Attach files"
                >
                  <Paperclip :size="14" />
                </button>
                <!-- Attachment count badge → opens attachment list popover -->
                <button
                  v-if="attachedFiles.length > 0"
                  @click.stop="showAttachList = !showAttachList"
                  class="min-w-[18px] h-[18px] px-1 rounded-full bg-gray-200 text-gray-700 text-[11px] font-semibold flex items-center justify-center cursor-pointer hover:bg-gray-300 transition-colors flex-shrink-0"
                  :title="$t('session.viewAttachedFiles')"
                >{{ attachedFiles.length }}</button>

                <!-- Attachment list popover -->
                <div
                  v-if="showAttachList && attachedFiles.length > 0"
                  class="absolute bottom-full left-0 mb-1 w-72 bg-white rounded-xl shadow-lg border border-gray-100 overflow-hidden z-50"
                  @click.stop
                >
                  <div class="flex items-center justify-between px-3 py-2 border-b border-gray-50">
                    <span class="text-[11px] font-medium text-gray-500">Attached Files ({{ attachedFiles.length }})</span>
                    <button @click.stop="showAttachList = false" class="w-5 h-5 rounded flex items-center justify-center text-gray-400 hover:bg-gray-100 hover:text-gray-600 cursor-pointer transition-colors">
                      <X :size="12" />
                    </button>
                  </div>
                  <div class="max-h-[240px] overflow-y-auto py-1">
                    <div
                      v-for="f in attachedFiles" :key="f.path"
                      class="group flex items-center gap-2 px-3 py-1.5 hover:bg-gray-50"
                    >
                      <Paperclip :size="12" class="text-gray-400 flex-shrink-0" />
                      <div class="flex-1 min-w-0">
                        <div class="text-[12px] text-gray-800 truncate">{{ f.name }}</div>
                        <div class="text-[10px] text-gray-400 truncate">{{ f.path }}</div>
                      </div>
                      <span class="text-[10px] text-gray-400 flex-shrink-0">{{ formatFileSize(f.size) }}</span>
                      <button @click="removeAttachedFile(f.path)" class="p-1 rounded text-gray-300 hover:text-red-500 hover:bg-red-50 transition-colors flex-shrink-0 cursor-pointer" :title="$t('session.remove')">
                        <X :size="11" />
                      </button>
                    </div>
                  </div>
                </div>
              </div>
              <div class="flex items-center gap-2 flex-shrink-0">
                <!-- Permission mode selector -->
                <div class="relative permission-selector">
                  <button @click.stop="showPermissionDropdown = !showPermissionDropdown"
                    class="flex items-center gap-1 px-2.5 py-1.5 rounded-lg text-[11px] font-medium text-gray-600 hover:bg-gray-50 transition-all duration-150 cursor-pointer">
                    <Shield :size="11" />
                    <span :class="props.compact ? 'hidden' : 'hidden md:inline'">{{ permissionModeLabel }}</span>
                    <ChevronDown :size="10" :class="props.compact ? 'hidden' : ''" />
                  </button>
                  <div v-if="showPermissionDropdown" class="absolute bottom-full right-0 mb-1 w-48 bg-white rounded-xl shadow-lg border border-gray-100 overflow-hidden z-50">
                    <div v-for="o in permissionDisplayLabels" :key="o.id"
                      @click="selectedPermissionMode = o.id; showPermissionDropdown = false"
                      :class="['flex items-center gap-2 px-3 py-2 text-[12px] cursor-pointer transition-colors', selectedPermissionMode === o.id ? 'bg-gray-100 text-gray-900 font-medium' : 'text-gray-700 hover:bg-gray-50']">
                      <Shield :size="11" :class="selectedPermissionMode === o.id ? 'text-gray-600' : 'text-gray-300'" />
                      <span class="flex-1">{{ o.label }}</span>
                      <span class="relative group">
                        <HelpCircle :size="12" class="text-gray-300 hover:text-gray-500 transition-colors cursor-help" />
                        <span class="absolute left-full ml-2 top-1/2 -translate-y-1/2 px-2.5 py-1.5 text-[11px] text-white bg-gray-900 rounded-lg opacity-0 invisible group-hover:opacity-100 group-hover:visible transition-all duration-150 whitespace-nowrap z-50 shadow-lg max-w-[200px] dark:bg-zinc-800">
                          {{ o.description }}
                        </span>
                      </span>
                    </div>
                  </div>
                </div>

                <!-- Model selector -->
                <div class="relative model-selector">
                  <button @click.stop="toggleModelDropdown"
                    class="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-[11px] font-medium text-gray-600 hover:bg-gray-50 transition-all duration-150 cursor-pointer">
                    <img v-if="selectedModelInfo" :src="getProviderLogo(getProviderByName(selectedModelInfo.provider_name)?.id || 'custom')" :alt="selectedModelInfo.provider_name" class="w-4 h-4 object-contain" />
                    <Sparkles v-else :size="11" />
                    <span class="text-left" :class="props.compact ? 'hidden' : 'hidden md:inline'">
                      <span>{{ selectedModelInfo?.alias || selectedModelInfo?.name || $t('session.selectModel') }}</span>
                      <span v-if="selectedModelInfo && selectedModelInfo.alias"
                            class="text-[10px] text-gray-400 ml-1">{{ selectedModelInfo.name }}</span>
                    </span>
                    <ChevronDown :size="10" :class="props.compact ? 'hidden' : ''" />
                  </button>
                  <div v-if="showModelDropdown" class="absolute bottom-full right-0 mb-1 w-64 bg-white rounded-xl shadow-lg border border-gray-100 overflow-hidden z-50 max-h-72 overflow-y-auto">
                    <div class="px-3 py-1.5 text-[10px] font-semibold text-gray-400 uppercase tracking-wider">{{ $t("session.commercialModels") }}</div>
                    <div v-for="model in modelList.filter(m => m.provider !== 'llama')" :key="model.id"
                      @click="handleModelSelect(model)"
                      :class="['flex items-center gap-2 px-3 py-2 text-left cursor-pointer transition-colors', selectedModel === model.id ? 'bg-gray-100 text-gray-900 font-medium' : 'text-gray-700 hover:bg-gray-50']">
                      <img :src="getProviderLogo(getProviderByName(model.provider_name)?.id || 'custom')" :alt="model.provider_name" class="w-4 h-4 object-contain" />
                      <div class="flex-1 min-w-0">
                        <div class="text-[12px] font-medium truncate">{{ model.alias || model.name }}</div>
                        <div v-if="model.alias" class="text-[10px] text-gray-400 truncate">{{ model.name }}</div>
                      </div>
                    </div>
                    <div v-if="modelList.some(m => m.provider === 'llama')" class="border-t border-gray-200 mt-1">
                      <div class="px-3 py-2 text-[10px] font-semibold text-gray-500 uppercase tracking-wider flex items-center gap-1">
                        {{ $t("session.localModels") }}
                      </div>
                      <div v-for="model in modelList.filter(m => m.provider === 'llama')" :key="model.id"
                        @click="isLocalModelRunning(model) ? handleModelSelect(model) : router.push('/settings/models')"
                        :class="['flex items-center gap-2 px-3 py-2 text-left cursor-pointer transition-colors', isLocalModelRunning(model) ? (selectedModel === model.id ? 'bg-gray-100 text-gray-900 font-medium' : 'text-gray-700 hover:bg-gray-50') : 'text-gray-400 cursor-not-allowed opacity-60']">
                        <span :class="['w-2 h-2 rounded-full flex-shrink-0', isLocalModelRunning(model) ? 'bg-emerald-500' : 'bg-gray-300']"></span>
                        <img :src="getProviderLogo('llama')" :alt="model.provider_name" class="w-4 h-4 object-contain" />
                        <div class="flex-1 min-w-0">
                          <div class="text-[12px] font-medium truncate">{{ model.alias || model.name }}</div>
                          <div v-if="model.alias" class="text-[10px] text-gray-400 truncate">{{ model.name }}</div>
                        </div>
                        <span v-if="!isLocalModelRunning(model)" class="text-[10px] text-gray-400">{{ $t("session.startServer") }}</span>
                      </div>
                    </div>
                    <div v-if="modelList.length === 0" class="px-3 py-4 text-center text-[12px] text-gray-400">
                      {{ $t("session.noModels") }}
                    </div>
                  </div>
                </div>

                <button @click="noThinking = !noThinking" :disabled="isProcessing" class="p-1.5 rounded-lg transition-colors duration-150 mr-2 flex-shrink-0" :class="[noThinking ? 'bg-amber-100 text-amber-700 hover:bg-amber-200 cursor-pointer' : 'bg-gray-100 text-gray-400 hover:bg-gray-200 cursor-pointer', isProcessing && 'opacity-50 cursor-not-allowed']" :title="$t('session.toggleReasoning')">
                  <Sparkles :size="14" />
                </button>

                <!-- Context size ring: shows accumulated message body text
                     (content only) plus the current input as a fraction of the
                     200k cap. Click to expand the exact number. -->
                <div class="relative flex-shrink-0 mr-1 context-ring">
                  <button
                    @click.stop="showContextPopover = !showContextPopover"
                    :title="contextRingTitle"
                    class="relative w-8 h-8 flex items-center justify-center rounded-full transition-colors duration-150 hover:bg-gray-100 cursor-pointer"
                    :class="contextOverLimit ? 'ring-1 ring-red-200' : ''"
                  >
                    <svg class="absolute inset-0 w-8 h-8 -rotate-90" viewBox="0 0 32 32">
                      <circle cx="16" cy="16" r="13" fill="none" stroke="#e5e7eb" stroke-width="2.5" />
                      <circle
                        cx="16" cy="16" r="13" fill="none"
                        :stroke="contextRingColor"
                        stroke-width="2.5"
                        stroke-linecap="round"
                        :stroke-dasharray="`${contextFillRatio * 81.68} 81.68`"
                        class="transition-all duration-300"
                      />
                    </svg>
                    <span
                      class="relative text-[9px] font-semibold tabular-nums leading-none"
                      :class="contextOverLimit ? 'text-red-600' : 'text-gray-600'"
                    >{{ contextRingLabel }}</span>
                  </button>
                  <div
                    v-if="showContextPopover"
                    @click.stop
                    class="absolute bottom-full right-0 mb-2 w-56 bg-white rounded-xl shadow-lg border border-gray-100 overflow-hidden z-50"
                  >
                    <div class="px-3 py-2 border-b border-gray-50 flex items-center justify-between">
                      <span class="text-[11px] font-medium text-gray-500">{{ $t("session.contextSize") }}</span>
                      <button @click="showContextPopover = false" class="w-5 h-5 rounded flex items-center justify-center text-gray-400 hover:bg-gray-100 hover:text-gray-600 cursor-pointer transition-colors">
                        <X :size="12" />
                      </button>
                    </div>
                    <div class="px-3 py-2.5">
                      <div class="text-[12px] text-gray-800 tabular-nums font-medium">
                        {{ contextCharCount.toLocaleString() }} / {{ contextMaxChars.toLocaleString() }}
                      </div>
                      <div class="text-[10px] text-gray-400 mt-0.5">{{ $t("session.contextCharsLabel") }}</div>
                      <div
                        v-if="contextOverLimit"
                        class="mt-2 text-[11px] text-red-600 leading-snug"
                      >
                        Context limit reached. Please start a new session to continue.
                      </div>
                    </div>
                  </div>
                </div>

                <button v-if="!isProcessing" @click="handleSend" :disabled="!inputText.trim() || !selectedModel || contextOverLimit" class="flex items-center gap-1.5 px-3 py-1.5 rounded-xl transition-all duration-200 text-[12px] font-medium shadow-sm relative flex-shrink-0" :class="(inputText.trim() && selectedModel && !contextOverLimit)?'bg-gray-900 text-white hover:bg-gray-800 cursor-pointer dark:bg-zinc-800 dark:hover:bg-zinc-700':'bg-gray-200 text-gray-400 cursor-not-allowed'">
                  <Send :size="12" />{{ $t("input.send") }}
                  <span v-if="!selectedModel" class="absolute -top-8 right-0 px-2 py-1 text-[10px] text-white bg-gray-700 rounded-lg opacity-0 hover:opacity-100 dark:bg-zinc-700 transition-opacity whitespace-nowrap pointer-events-none z-50">{{ $t("session.selectModelHint") }}</span>
                  <span v-else-if="contextOverLimit" class="absolute -top-8 right-0 px-2 py-1 text-[10px] text-white bg-gray-700 rounded-lg opacity-0 hover:opacity-100 dark:bg-zinc-700 transition-opacity whitespace-nowrap pointer-events-none z-50">{{ $t("session.contextLimitShort") }}</span>
                </button>
                <button v-else @click="handleStop" class="flex items-center gap-1.5 px-3 py-1.5 rounded-xl bg-gray-900 text-white hover:bg-red-600 transition-all duration-200 cursor-pointer text-[12px] font-medium shadow-sm flex-shrink-0 dark:bg-zinc-800 dark:hover:bg-red-600"><Square :size="12" />{{ $t("input.stop") }}</button>
              </div>
            </div>
          </div>
        </div>
      </div>
    </template>

    <div v-else class="flex-1 flex flex-col min-h-0">
      <div class="flex-1 flex flex-col items-center justify-center px-8 min-h-0 overflow-y-auto">
        <div class="w-full max-w-[640px]">

        <!-- Slogan -->
        <p v-if="hasAnyAgentInstalled && hasAnyModel && (!selectedAgent || selectedAgent.installed)" class="text-center text-[28px] font-semibold text-gray-800 mb-6 tracking-tight">
          {{ $t("session.whatBuilding") }}
        </p>

        <div class="flex justify-center mb-5">
          <div class="inline-flex bg-gray-100 rounded-2xl p-1 gap-0.5">
            <template v-if="enabledAgents.length > 0">
              <button v-for="a in enabledAgents" :key="a.id" @click="selectedAgentId=a.id" :class="['flex items-center gap-1.5 px-4 py-2 rounded-xl text-[13px] font-medium transition-all duration-200 cursor-pointer',selectedAgentId===a.id?'bg-white text-gray-900 shadow-sm dark:bg-gray-200 dark:shadow-none':'text-gray-500 hover:text-gray-700']">
                <AgentIcon :agent-id="a.id" />
                {{ agentDisplayName(a) }}
              </button>
            </template>
            <button v-else @click="router.push('/settings/agents')" class="flex items-center gap-1.5 px-4 py-2 rounded-xl text-[13px] font-medium text-gray-400 hover:text-gray-600 transition-all duration-200 cursor-pointer">
              <Package :size="14" />
              {{ $t("session.installAnAgent") }}
            </button>
            <!-- More agents dropdown -->
            <div class="relative more-agents-selector">
              <button @click.stop="showMoreAgents = !showMoreAgents" class="flex items-center gap-1 px-3 py-2 rounded-xl text-[13px] font-medium text-gray-500 hover:text-gray-700 transition-all duration-200 cursor-pointer">
                {{ $t("session.more") }}
                <ChevronDown :size="10" :class="showMoreAgents ? 'rotate-180' : ''" class="transition-transform duration-150" />
              </button>
              <div v-if="showMoreAgents" class="absolute top-full left-0 mt-1 w-44 bg-white rounded-xl shadow-lg border border-gray-100 overflow-hidden z-50 py-1">
                <div class="px-3 py-1.5 text-[10px] font-semibold text-gray-400 uppercase tracking-wider">{{ $t("session.comingSoon") }}</div>
                <button
                  v-for="agent in otherAgents"
                  :key="agent.id"
                  class="w-full flex items-center gap-2 px-3 py-2 text-left text-[12px] text-gray-400 cursor-default"
                >
                  <span class="w-4 h-4 rounded-full bg-gray-200 flex-shrink-0" />
                  {{ agent.name }}
                </button>
              </div>
            </div>
          </div>
        </div>

        <!-- Onboarding: no agents installed at all -->
        <div v-if="!hasAnyAgentInstalled && agents.length > 0" class="mb-5 p-6 rounded-2xl border border-blue-200 bg-gradient-to-br from-blue-50 to-indigo-50 text-center">
          <div class="w-12 h-12 mx-auto mb-3 rounded-xl bg-blue-100 flex items-center justify-center">
            <Package :size="24" class="text-blue-600" />
          </div>
          <p class="text-[15px] font-semibold text-gray-800 mb-1">{{ $t("empty.welcome") }}</p>
          <p class="text-[13px] text-gray-500 mb-5 max-w-sm mx-auto leading-relaxed">
            {{ $t("session.installPrompt") }}
          </p>
          <div class="flex items-center justify-center gap-3">
            <button @click="router.push('/settings/agents')" class="inline-flex items-center gap-2 px-4 py-2.5 rounded-xl text-[13px] font-semibold bg-blue-600 text-white hover:bg-blue-700 active:scale-[0.98] transition-all cursor-pointer shadow-sm">
              <Download :size="15" /> {{ $t("session.installAgentBtn") }}
            </button>
            <button @click="router.push('/settings/models/commercial?action=add')" class="inline-flex items-center gap-2 px-4 py-2.5 rounded-xl text-[13px] font-semibold bg-white text-gray-700 border border-gray-200 hover:bg-gray-50 active:scale-[0.98] transition-all cursor-pointer shadow-sm">
              <Wand2 :size="15" /> {{ $t("session.configureModel") }}
            </button>
          </div>
        </div>

        <!-- Selected agent not installed -->
        <div v-else-if="selectedAgent && !selectedAgent.installed" class="mb-5 p-5 rounded-2xl border border-amber-200 bg-amber-50 text-center">
          <p class="text-[14px] font-semibold text-amber-800 mb-1">{{ $t("session.notInstalled", { name: agentDisplayName(selectedAgent) }) }}</p>
          <p class="text-[13px] text-amber-600 mb-1">{{ $t("session.notInstalledHint") }}</p>
          <p v-if="!hasAnyModel" class="text-[12px] text-amber-500 mb-3">{{ $t("session.needModelToo") }}</p>
          <button @click="router.push(`/settings/agents/${selectedAgentId}`)" class="inline-flex items-center gap-1.5 px-4 py-2 rounded-xl text-[13px] font-semibold bg-amber-600 text-white hover:bg-amber-700 active:scale-[0.98] transition-all cursor-pointer shadow-sm">
            <Download :size="14" /> {{ $t("session.installName", { name: agentDisplayName(selectedAgent) }) }}
          </button>
        </div>

        <!-- Agent installed but no models -->
        <div v-else-if="hasAnyAgentInstalled && !hasAnyModel" class="mb-5 p-4 rounded-2xl border border-purple-200 bg-purple-50 flex items-center gap-3">
          <Wand2 :size="18" class="text-purple-500 flex-shrink-0" />
          <div class="flex-1">
            <p class="text-[13px] font-medium text-purple-800">{{ $t("session.noModelYet") }}</p>
            <p class="text-[12px] text-purple-500">{{ $t("session.noModelHint") }}</p>
          </div>
          <button @click="router.push('/settings/models/commercial?action=add')" class="flex-shrink-0 px-3 py-1.5 rounded-lg text-[12px] font-semibold bg-purple-600 text-white hover:bg-purple-700 active:scale-[0.98] transition-all cursor-pointer">
            {{ $t("session.addModelBtn") }}
          </button>
        </div>

        <div v-else @click="focusInputOnContainerClick" class="rounded-t-2xl border border-gray-200 bg-white focus-within:border-gray-300 focus-within:shadow-sm transition-all duration-150 relative">
          <!-- Skills tags row: single-line with horizontal scroll; wand button always visible -->
          <div class="skills-selector flex items-center gap-1.5 px-3 pt-2.5 pb-1.5 min-h-[34px]">
            <div class="flex items-center gap-1.5 overflow-x-auto flex-1 min-w-0" style="scrollbar-width: none; -ms-overflow-style: none;">
              <!-- hide webkit scrollbar via inline class -->
              <span
                v-for="name in selectedSkillNames" :key="name"
                @click="toggleSkill(name)"
                class="inline-flex items-center gap-1 px-2 py-0.5 rounded-md text-[11px] bg-gray-100 text-gray-700 cursor-pointer hover:bg-gray-200 transition-colors flex-shrink-0"
              >
                {{ name }}
                <X :size="9" />
              </span>
            </div>
            <button
              @click.stop="showSkillsPopover = !showSkillsPopover"
              :disabled="availableSkills.length === 0"
              :class="[
                'inline-flex items-center justify-center w-6 h-6 rounded-md transition-colors cursor-pointer flex-shrink-0',
                availableSkills.length > 0
                  ? showSkillsPopover
                    ? 'bg-gray-100 text-gray-700'
                    : 'text-gray-400 hover:bg-gray-100 hover:text-gray-600'
                  : 'text-gray-200 cursor-not-allowed',
              ]"
              :title="$t('session.skills')"
            >
              <Wand2 :size="13" />
            </button>
          </div>

          <!-- Skills popover: card grid, stays open for multi-select -->
          <div
            v-if="showSkillsPopover && availableSkills.length > 0"
            class="absolute bottom-full left-0 right-0 mb-1 bg-white rounded-xl border border-gray-100 shadow-lg z-50 overflow-hidden"
          >
            <div class="flex items-center justify-between px-3 py-2 border-b border-gray-50">
              <span class="text-[11px] font-medium text-gray-500">{{ $t("session.skillsCount", { count: selectedSkills.size, total: availableSkills.length }) }}</span>
              <button @click.stop="showSkillsPopover = false" class="w-5 h-5 rounded flex items-center justify-center text-gray-400 hover:bg-gray-100 hover:text-gray-600 cursor-pointer transition-colors">
                <X :size="12" />
              </button>
            </div>
            <div class="p-2 grid grid-cols-2 gap-1.5 max-h-[260px] overflow-y-auto">
              <div
                v-for="skill in availableSkills" :key="skill.name"
                @click="toggleSkill(skill.name)"
                :title="skill.description"
                :class="[
                  'px-2.5 py-2 rounded-lg cursor-pointer transition-all border',
                  selectedSkills.has(skill.name)
                    ? 'bg-gray-900 border-gray-900 text-white dark:bg-zinc-800 dark:border-zinc-800'
                    : 'bg-white border-gray-100 hover:border-gray-200 hover:bg-gray-50 text-gray-700',
                ]"
              >
                <div class="text-[13px] font-medium leading-tight mb-0.5 truncate">{{ skill.name }}</div>
                <div :class="['text-[10px] leading-snug line-clamp-3', selectedSkills.has(skill.name) ? 'text-gray-300' : 'text-gray-400']">{{ skill.description }}</div>
              </div>
            </div>
          </div>

          <textarea ref="newSessionTextarea" v-model="inputText" placeholder="" rows="4" class="w-full px-4 pt-2 bg-transparent border-none outline-none resize-none text-[15px] text-gray-900 leading-relaxed" @input="onTextareaInput" @keydown="onTextareaKeydown" />
          <div v-if="!inputText" class="absolute left-4 top-4 pointer-events-none text-[15px] text-gray-400 leading-relaxed"
            :style="{ transform: 'translateY(28px)' }"
          >
            {{ typingPlaceholder }}<span class="animate-pulse">|</span>
          </div>
          <div class="flex items-center justify-between px-3 pb-2 gap-2">
            <div class="relative flex items-center gap-1.5 flex-shrink-0">
              <button
                @click="handleAttachFiles"
                class="p-1.5 rounded-lg text-gray-400 hover:bg-gray-100 hover:text-gray-600 transition-colors duration-150 cursor-pointer flex-shrink-0"
                title="Attach files"
              >
                <Paperclip :size="14" />
              </button>
              <!-- Attachment count badge → opens attachment list popover -->
              <button
                v-if="attachedFiles.length > 0"
                @click.stop="showAttachList = !showAttachList"
                class="min-w-[18px] h-[18px] px-1 rounded-full bg-gray-200 text-gray-700 text-[11px] font-semibold flex items-center justify-center cursor-pointer hover:bg-gray-300 transition-colors flex-shrink-0"
                title="View attached files"
              >{{ attachedFiles.length }}</button>

              <!-- Attachment list popover -->
              <div
                v-if="showAttachList && attachedFiles.length > 0"
                class="absolute bottom-full left-0 mb-1 w-72 bg-white rounded-xl shadow-lg border border-gray-100 overflow-hidden z-50"
                @click.stop
              >
                <div class="flex items-center justify-between px-3 py-2 border-b border-gray-50">
                  <span class="text-[11px] font-medium text-gray-500">Attached Files ({{ attachedFiles.length }})</span>
                  <button @click.stop="showAttachList = false" class="w-5 h-5 rounded flex items-center justify-center text-gray-400 hover:bg-gray-100 hover:text-gray-600 cursor-pointer transition-colors">
                    <X :size="12" />
                  </button>
                </div>
                <div class="max-h-[240px] overflow-y-auto py-1">
                  <div
                    v-for="f in attachedFiles" :key="f.path"
                    class="group flex items-center gap-2 px-3 py-1.5 hover:bg-gray-50"
                  >
                    <Paperclip :size="12" class="text-gray-400 flex-shrink-0" />
                    <div class="flex-1 min-w-0 text-[12px] text-gray-800 truncate">{{ f.name }}</div>
                    <span class="text-[10px] text-gray-400 flex-shrink-0">{{ formatFileSize(f.size) }}</span>
                    <button @click="removeAttachedFile(f.path)" class="p-1 rounded text-gray-300 hover:text-red-500 hover:bg-red-50 transition-colors flex-shrink-0 cursor-pointer" title="Remove">
                      <X :size="11" />
                    </button>
                  </div>
                </div>
              </div>
            </div>
            <div class="flex items-center gap-2 flex-shrink-0">
              <!-- Permission mode selector -->
              <div class="relative permission-selector">
                <button @click.stop="showPermissionDropdown = !showPermissionDropdown"
                  class="flex items-center gap-1 px-2.5 py-1.5 rounded-lg text-[11px] font-medium text-gray-600 hover:bg-gray-50 transition-all duration-150 cursor-pointer">
                  <Shield :size="11" />
                  <span class="hidden md:inline">{{ permissionModeLabel }}</span>
                  <ChevronDown :size="10" />
                </button>
                <div v-if="showPermissionDropdown" class="absolute bottom-full right-0 mb-1 w-48 bg-white rounded-xl shadow-lg border border-gray-100 overflow-hidden z-50">
                  <div v-for="o in permissionDisplayLabels" :key="o.id"
                    @click="selectedPermissionMode = o.id; showPermissionDropdown = false"
                    :class="['flex items-center gap-2 px-3 py-2 text-[12px] cursor-pointer transition-colors', selectedPermissionMode === o.id ? 'bg-gray-100 text-gray-900 font-medium' : 'text-gray-700 hover:bg-gray-50']">
                    <Shield :size="11" :class="selectedPermissionMode === o.id ? 'text-gray-600' : 'text-gray-300'" />
                    <span class="flex-1">{{ o.label }}</span>
                    <span class="relative group">
                      <HelpCircle :size="12" class="text-gray-300 hover:text-gray-500 transition-colors cursor-help" />
                      <span class="absolute left-full ml-2 top-1/2 -translate-y-1/2 px-2.5 py-1.5 text-[11px] text-white bg-gray-900 rounded-lg opacity-0 invisible group-hover:opacity-100 group-hover:visible transition-all duration-150 whitespace-nowrap z-50 shadow-lg max-w-[200px] dark:bg-zinc-800">
                        {{ o.description }}
                      </span>
                    </span>
                  </div>
                </div>
              </div>

              <!-- Model selector -->
              <div class="relative model-selector">
                <button @click.stop="toggleModelDropdown"
                  class="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-[11px] font-medium text-gray-600 hover:bg-gray-50 transition-all duration-150 cursor-pointer">
                  <img v-if="selectedModelInfo" :src="getProviderLogo(getProviderByName(selectedModelInfo.provider_name)?.id || 'custom')" :alt="selectedModelInfo.provider_name" class="w-4 h-4 object-contain" />
                  <Sparkles v-else :size="11" />
                  <span class="text-left hidden md:inline">
                    <span>{{ selectedModelInfo?.alias || selectedModelInfo?.name || $t('session.selectModel') }}</span>
                    <span v-if="selectedModelInfo && selectedModelInfo.alias" 
                          class="text-[10px] text-gray-400 ml-1">{{ selectedModelInfo.name }}</span>
                  </span>
                  <ChevronDown :size="10" />
                </button>
                <div v-if="showModelDropdown" class="absolute bottom-full right-0 mb-1 w-64 bg-white rounded-xl shadow-lg border border-gray-100 overflow-hidden z-50 max-h-72 overflow-y-auto">
                    <div class="px-3 py-1.5 text-[10px] font-semibold text-gray-400 uppercase tracking-wider">{{ $t("session.commercialModels") }}</div>
                    <div v-for="model in modelList.filter(m => m.provider !== 'llama')" :key="model.id"
                      @click="handleModelSelect(model)"
                      :class="['flex items-center gap-2 px-3 py-2 text-left cursor-pointer transition-colors', selectedModel === model.id ? 'bg-gray-100 text-gray-900 font-medium' : 'text-gray-700 hover:bg-gray-50']">
                      <img :src="getProviderLogo(getProviderByName(model.provider_name)?.id || 'custom')" :alt="model.provider_name" class="w-4 h-4 object-contain" />
                      <div class="flex-1 min-w-0">
                        <div class="text-[12px] font-medium truncate">{{ model.alias || model.name }}</div>
                        <div v-if="model.alias" class="text-[10px] text-gray-400 truncate">{{ model.name }}</div>
                      </div>
                    </div>
                    <div v-if="modelList.some(m => m.provider === 'llama')" class="border-t border-gray-200 mt-1">
                      <div class="px-3 py-2 text-[10px] font-semibold text-gray-500 uppercase tracking-wider flex items-center gap-1">
                        {{ $t("session.localModels") }}
                      </div>
                      <div v-for="model in modelList.filter(m => m.provider === 'llama')" :key="model.id"
                        @click="isLocalModelRunning(model) ? handleModelSelect(model) : router.push('/settings/models')"
                        :class="['flex items-center gap-2 px-3 py-2 text-left cursor-pointer transition-colors', isLocalModelRunning(model) ? (selectedModel === model.id ? 'bg-gray-100 text-gray-900 font-medium' : 'text-gray-700 hover:bg-gray-50') : 'text-gray-400 cursor-not-allowed opacity-60']">
                        <span :class="['w-2 h-2 rounded-full flex-shrink-0', isLocalModelRunning(model) ? 'bg-emerald-500' : 'bg-gray-300']"></span>
                        <img :src="getProviderLogo('llama')" :alt="model.provider_name" class="w-4 h-4 object-contain" />
                        <div class="flex-1 min-w-0">
                          <div class="text-[12px] font-medium truncate">{{ model.alias || model.name }}</div>
                          <div v-if="model.alias" class="text-[10px] text-gray-400 truncate">{{ model.name }}</div>
                        </div>
                        <span v-if="!isLocalModelRunning(model)" class="text-[10px] text-gray-400">{{ $t("session.startServer") }}</span>
                      </div>
                    </div>
                    <div v-if="modelList.length === 0" class="px-3 py-4 text-center text-[12px] text-gray-400">
                      {{ $t("session.noModels") }}
                    </div>
                    <div class="border-t border-gray-100">
                      <button
                        @click="router.push('/settings/models/commercial?action=add')"
                      class="w-full flex items-center gap-2 px-3 py-2.5 text-left text-[12px] text-gray-600 hover:bg-gray-50 transition-colors cursor-pointer font-medium"
                    >
                      <Plus :size="13" class="text-gray-400" />
                      {{ $t("session.addModelBtn") }}
                    </button>
                  </div>
                </div>
              </div>

              <!-- No thinking toggle -->
              <button @click="noThinking = !noThinking" class="p-1.5 rounded-lg transition-colors duration-150 flex-shrink-0 mr-2" :class="noThinking ? 'bg-amber-100 text-amber-700 hover:bg-amber-200 cursor-pointer' : 'bg-gray-100 text-gray-400 hover:bg-gray-200 cursor-pointer'" :title="$t('session.toggleReasoning')">
                <Sparkles :size="14" />
              </button>
              <!-- Send button -->
              <button @click="handleSend" :disabled="!inputText.trim() || !selectedModel" class="flex items-center gap-1.5 px-3 py-1.5 rounded-xl transition-all duration-200 text-[12px] font-medium shadow-sm flex-shrink-0 relative" :class="inputText.trim() && selectedModel ?'bg-gray-900 text-white hover:bg-gray-800 cursor-pointer dark:bg-zinc-800 dark:hover:bg-zinc-700':'bg-gray-200 text-gray-400 cursor-not-allowed'">
                <Send :size="12" />{{ $t("input.send") }}
                <span v-if="!selectedModel" class="absolute -top-8 right-0 px-2 py-1 text-[10px] text-white bg-gray-700 rounded-lg opacity-0 hover:opacity-100 dark:bg-zinc-700 transition-opacity whitespace-nowrap pointer-events-none z-50">{{ $t("session.selectModelHint") }}</span>
              </button>
            </div>
          </div>

          <!-- Directory picker -->
          <div class="relative flex items-center gap-2 px-4 py-1.5 bg-gray-50 rounded-b-2xl dir-selector">
            <button
              @click.stop="showDirMenu = !showDirMenu"
              class="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] text-gray-600 bg-gray-100 hover:bg-gray-200 transition-colors cursor-pointer"
            >
              <Folder :size="13" />
              <span v-if="!dirPath">{{ $t("session.workInProject") }}</span>
              <span v-else class="text-gray-700 font-medium">{{ dirPath.split('/').pop() }}</span>
            </button>
            <button
              v-if="dirPath"
              @click="dirPath = ''"
              class="p-1 rounded-md text-gray-400 hover:text-gray-600 hover:bg-gray-200/60 transition-colors cursor-pointer"
              :title="$t('session.clearProject')"
            >
              <X :size="12" />
            </button>

            <!-- Dropdown menu -->
            <div
              v-if="showDirMenu"
              class="absolute left-0 bottom-full mb-1 w-72 bg-white rounded-xl shadow-lg border border-gray-100 overflow-hidden z-50 py-1"
              @click.stop
            >
            <!-- Recent projects -->
            <div v-if="recentDirs.length > 0">
              <div class="px-3 py-1.5 text-[10px] font-semibold text-gray-400 uppercase tracking-wider">{{ $t("session.recentProjects") }}</div>
              <div
                v-for="d in recentDirs"
                :key="d"
                class="group w-full flex items-center gap-2 px-3 py-2 text-left text-[12px] text-gray-700 hover:bg-gray-50 transition-colors"
              >
                <button
                  @click="selectRecentDir(d)"
                  class="flex-1 flex items-center gap-2 min-w-0 text-left cursor-pointer"
                >
                  <Folder :size="12" class="text-gray-400 flex-shrink-0" />
                  <div class="min-w-0 flex-1">
                    <div class="truncate font-medium">{{ d.split('/').pop() }}</div>
                    <div class="text-[10px] text-gray-400 truncate">{{ d }}</div>
                  </div>
                </button>
                <button
                  @click.stop="removeRecentDir(d)"
                  class="p-1 rounded text-gray-300 hover:text-red-500 hover:bg-red-50 opacity-0 group-hover:opacity-100 transition-all flex-shrink-0 cursor-pointer"
                  :title="$t('session.remove')"
                >
                  <X :size="12" />
                </button>
              </div>
              <div class="mx-3 my-1 border-t border-gray-100" />
            </div>
            <!-- No project -->
            <button
              @click="dirPath = ''; showDirMenu = false"
              class="w-full flex items-center gap-2 px-3 py-2 text-left text-[12px] text-gray-500 hover:bg-gray-50 transition-colors cursor-pointer"
            >
              <X :size="12" class="text-gray-400" />
              {{ $t("session.noProject") }}
            </button>
            <!-- Open a new folder -->
            <button
              @click="pickDirectory"
              class="w-full flex items-center gap-2 px-3 py-2 text-left text-[12px] text-gray-700 hover:bg-gray-50 transition-colors cursor-pointer"
            >
              <FolderPlus :size="13" class="text-gray-400" />
              {{ $t("session.openNewFolder") }}
            </button>
          </div>
        </div>
        </div>
      </div>
    </div>
      <div class="flex justify-center pt-2 pb-6">
        <button
          @click="showFeedbackModal = true"
          class="inline-flex items-center justify-center w-9 h-9 rounded-full text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors cursor-pointer"
          :title="$t('session.sendFeedback')"
        >
          <MessageCircle :size="15" />
        </button>
      </div>
    </div>
  </main>

  <!-- @ Mention file picker popup (Teleported to body) -->
  <MentionPicker
    v-if="showMentionPicker"
    ref="mentionPickerRef"
    :cwd="mentionCwd"
    :anchor-rect="mentionAnchorRect"
    :external-query="mentionQuery"
    @select="onMentionSelect"
    @close="closeMentionPicker"
  />

  <!-- Feedback modal -->
  <div
    v-if="showFeedbackModal"
    class="fixed inset-0 z-[100] flex items-center justify-center"
    @click.self="closeFeedback"
  >
    <div class="absolute inset-0 bg-black/30" @click="closeFeedback" />
    <div class="relative w-[420px] max-w-[90vw] bg-white rounded-2xl shadow-2xl p-5">
      <div class="flex items-center justify-between mb-3">
        <div class="flex items-center gap-2">
          <MessageCircle :size="16" class="text-gray-500" />
          <span class="text-[15px] font-semibold text-gray-900">{{ $t("session.feedbackTitle") }}</span>
        </div>
        <button
          @click="closeFeedback"
          class="w-6 h-6 rounded-md flex items-center justify-center text-gray-400 hover:bg-gray-100 hover:text-gray-600 transition-colors cursor-pointer"
        >
          <X :size="14" />
        </button>
      </div>

      <template v-if="feedbackDone">
        <div class="py-10 flex flex-col items-center gap-2">
          <div class="w-10 h-10 rounded-full bg-green-100 flex items-center justify-center">
            <Check :size="20" class="text-green-600" />
          </div>
          <p class="text-[14px] font-medium text-gray-900">{{ $t("session.feedbackThanks") }}</p>
          <p class="text-[12px] text-gray-400">{{ $t("session.feedbackThanksDesc") }}</p>
        </div>
      </template>

      <template v-else>
        <div class="flex gap-1.5 mb-3">
          <button
            v-for="t in feedbackTypes"
            :key="t.id"
            @click="feedbackType = t.id"
            :class="[
              'flex-1 px-3 py-1.5 rounded-lg text-[12px] font-medium border transition-colors cursor-pointer',
              feedbackType === t.id
                ? 'bg-gray-900 border-gray-900 text-white dark:bg-zinc-800 dark:border-zinc-800'
                : 'bg-white border-gray-200 text-gray-600 hover:bg-gray-50',
            ]"
          >
            {{ $t(t.labelKey) }}
          </button>
        </div>

        <textarea
          v-model="feedbackContent"
          rows="5"
          :placeholder="$t('session.feedbackPlaceholder')"
          class="w-full px-3 py-2.5 rounded-xl border text-[13px] text-gray-900 placeholder:text-gray-300 bg-white focus:outline-none transition-colors resize-none"
          :class="feedbackError ? 'border-red-300 focus:border-red-400' : 'border-gray-200 focus:border-blue-400'"
          @input="feedbackError = ''"
        />
        <input
          v-model="feedbackEmail"
          type="email"
          placeholder="Email (optional)"
          class="mt-2 w-full h-9 px-3 rounded-xl border text-[13px] text-gray-900 placeholder:text-gray-300 bg-white focus:outline-none transition-colors"
          :class="feedbackError ? 'border-red-300 focus:border-red-400' : 'border-gray-200 focus:border-blue-400'"
        />

        <p v-if="feedbackError" class="mt-2 text-[12px] text-red-600">{{ feedbackError }}</p>

        <div class="mt-4 flex items-center justify-end gap-2">
          <button
            @click="closeFeedback"
            class="px-4 h-9 rounded-xl text-[13px] font-medium text-gray-600 hover:bg-gray-100 transition-colors cursor-pointer"
            :disabled="feedbackSending"
          >
            Cancel
          </button>
          <button
            @click="submitFeedbackForm"
            :disabled="!feedbackContent.trim() || feedbackSending"
            class="px-4 h-9 rounded-xl text-[13px] font-medium text-white transition-colors flex items-center gap-1.5"
            :class="feedbackContent.trim() && !feedbackSending ? 'bg-blue-600 hover:bg-blue-700 cursor-pointer' : 'bg-gray-300 cursor-not-allowed'"
          >
            {{ feedbackSending ? $t("session.feedbackSending") : $t("session.feedbackSubmit") }}
          </button>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
/* Hide scrollbar on the skills tags row while keeping it scrollable */
.skills-selector ::-webkit-scrollbar {
  display: none;
}

/* 隐藏会话消息列表右侧滚动条（内容仍可正常滚动）。
   Windows WebView2 的经典滚动条会占位约 17px，出现/消失还会引起内容宽度
   抖动，隐藏后既能满足"不需要显示滚动条"，也消除了滚动条占位导致的布局闪烁。 */
.chat-scrollbar-hidden {
  scrollbar-width: none; /* Firefox */
  -ms-overflow-style: none; /* legacy Edge */
}
.chat-scrollbar-hidden::-webkit-scrollbar {
  display: none; /* Chrome / WebView2 */
}
</style>
