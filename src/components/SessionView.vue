<script setup lang="ts">
import { ref, watch, onMounted, onBeforeUnmount, onActivated, computed, nextTick } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useRouter } from "vue-router";
import { useWorkspaceStore, type Session } from "../stores/useWorkspaceStore";
import { useMessageStore } from "../stores/useMessageStore";
import { useAgentStore } from "../stores/useAgentStore";
import { getModels, getLastAgent, setLastAgent, getAgentModels, setAgentModel, type ModelEntry, getProviderByName } from "../api/models";
import { getProviderLogo } from "../utils/providerIcons";
import { sendInput, startSession as apiStartSession, stopSession as tauriStopSession, listSkills, listSessionSkills, deploySessionSkill, removeSessionSkill, type SkillInfo } from "../api/sessions";
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
import { submitFeedback } from "../api/telemetry";
import { Send, Square, Download, Shield, ChevronDown, ArrowDown, Folder, X, FolderPlus, Sparkles, HelpCircle, Plus, Package, Wand2, Paperclip, MessageCircle, Check } from "lucide-vue-next";
import { useToast } from "../composables/useToast";

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
  /** Session ID when using KeepAlive cache. When provided, this component
   *  is dedicated to a single session and won't watch store.activeSessionId. */
  sessionId?: string;
}>();

const store = useWorkspaceStore();
const router = useRouter();
const msgStore = useMessageStore();
const agentStore = useAgentStore();
const messages = ref<Message[]>([]);
const unlisteners = new Map<string, UnlistenFn>();
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

function checkScrollPosition() {
  const el = messageContainer.value;
  if (!el) return;
  const threshold = 100;
  showScrollToBottom.value = el.scrollHeight - el.scrollTop - el.clientHeight > threshold;
}

// ═══ Smart auto-scroll (stick-to-bottom) ═══
// 只在用户位于底部附近时自动跟随滚动；用户向上滚动查看历史后暂停跟随，
// 滚回底部附近自动恢复。避免会话生成过程中被强制拉回底部、无法上翻。
const STICK_THRESHOLD = 100;
const stickToBottom = ref(true);

function onChatScroll() {
  const el = messageContainer.value;
  if (el) {
    const distFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    stickToBottom.value = distFromBottom <= STICK_THRESHOLD;
  }
  checkScrollPosition();
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

async function handleAttachFiles() {
  try {
    const selected = await open({
      multiple: true,
      filters: [{ name: "Supported Files", extensions: ATTACH_ACCEPTED_EXTS }],
    });
    if (!selected) return;
    const list = Array.isArray(selected) ? selected : [selected];
    const existing = new Set(attachedFiles.value.map(f => f.path));
    for (const file of list) {
      if (existing.has(file)) continue;
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
const noThinking = ref(false);

const placeholderText = "Ask anything — any question, any file, any code. Type @ to pick a file";
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
    if (typingIndex < placeholderText.length) {
      typingPlaceholder.value += placeholderText[typingIndex];
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

const permissionLabels: Record<string, Record<string, string>> = {
  "claude-code": { read_only: "Plan Mode", ask_approval: "Accept Edits", approve_for_me: "Auto Mode", full_access: "Bypass Permissions" },
  "codex-cli": { read_only: "Read Only", ask_approval: "Ask for approval", approve_for_me: "Approve for me", full_access: "Full Access" },
  "gemini-cli": { read_only: "plan", ask_approval: "auto_edit", approve_for_me: "auto", full_access: "yolo" },
};

const permissionDescriptions: Record<string, string> = {
  read_only: "The agent can only read files and plan actions, but cannot make any changes.",
  ask_approval: "The agent will ask for your confirmation before making any changes to files.",
  approve_for_me: "The agent will automatically approve most actions, but may ask for critical changes.",
  full_access: "The agent has full access to read and modify files without asking for approval.",
};

const permissionModeLabel = computed(() => {
  return permissionLabels[selectedAgentId.value]?.[selectedPermissionMode.value] || selectedPermissionMode.value;
});

const permissionDisplayLabels = computed(() => {
  return permissionModeOptions.map(o => ({
    ...o,
    label: permissionLabels[selectedAgentId.value]?.[o.id] || o.id,
    description: permissionDescriptions[o.id],
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
const feedbackTypes = [
  { id: "bug", label: "Bug Report" },
  { id: "feature", label: "Feature Request" },
  { id: "other", label: "Other" },
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
  nextTick(() => {
    if (messageContainer.value) {
      messageContainer.value.scrollTop = messageContainer.value.scrollHeight;
      // For completed sessions, double-check after paint
      requestAnimationFrame(() => {
        if (messageContainer.value) {
          messageContainer.value.scrollTop = messageContainer.value.scrollHeight;
        }
      });
    }
  });
}

// Effective session ID: use prop in KeepAlive mode, store otherwise
const effectiveSessionId = computed(() => props.sessionId || store.activeSessionId || '');

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
    // Only sync messages if they changed while deactivated (ACP events may
    // have added new messages). Avoids expensive re-render of cached DOM.
    if (state.messages.length !== messages.value.length) {
      messages.value = [...state.messages];
    }
    isProcessing.value = state.isProcessing;
    scrollToBottom();
  }
});

async function initSession(sid: string) {
  activeThinking.value = ""; activeContent.value = ""; thoughtDuration.value = "";
  inputText.value = "";
  mentionsMap.value = new Map();
  closeMentionPicker();
  const state = getSessionState(sid);
  messages.value = [...state.messages];
  isProcessing.value = state.isProcessing;
  isSessionLoading.value = true;
  if (!unlisteners.has(sid)) {
    try {
      const un = await listen<AcpPayload>(`acp:${sid}`, (e) => handleAcpEvent(sid, e.payload));
      unlisteners.set(sid, un);
    } catch {}
  }
  await loadSessionMessages(sid);
  restorePendingSend(sid);
  scrollToBottom();
  setTimeout(() => { isSessionLoading.value = false; }, 200);
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
  if (props.sessionId) return; // Skip if using KeepAlive mode
  activeThinking.value = ""; activeContent.value = ""; thoughtDuration.value = "";
  inputText.value = "";
  mentionsMap.value = new Map();
  closeMentionPicker();
  if (newId) {
    const state = getSessionState(newId);
    messages.value = [...state.messages];
    isProcessing.value = state.isProcessing;
    isSessionLoading.value = true;
    if (!unlisteners.has(newId)) {
      try { 
        const un = await listen<AcpPayload>(`acp:${newId}`, (e) => handleAcpEvent(newId, e.payload)); 
        unlisteners.set(newId, un);
      } catch {}
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
  /** Send retry state. Set when a message is dispatched; drives the
   *  auto-retry loop (max 3 attempts) that shows "try x/3" on the chat. */
  retry: {
    attempts: number;
    lastText: string;
    timer: ReturnType<typeof setTimeout> | null;
    deadlineTimer: ReturnType<typeof setTimeout> | null;
  } | null;
}

// Max send attempts including the first try; failures auto-retry up to this.
const RETRY_MAX = 3;
const RETRY_DELAY_MS = 1000;
const RETRY_TIMEOUT_MS = 60_000;

const sessionStates = new Map<string, SessionState>();

function getSessionState(sessionId: string): SessionState {
  let state = sessionStates.get(sessionId);
  if (!state) {
    state = {
      messages: msgStore.getMessages(sessionId) || [],
      activeThinking: "",
      activeContent: "",
      thoughtDuration: "",
      thinkingStartTime: 0,
      isProcessing: false,
      loaded: false,
      turnStartTime: 0,
      retry: null,
    };
    sessionStates.set(sessionId, state);
  }
  return state;
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
      state.messages = loadedMessages;
      state.loaded = true;
      msgStore.setMessages(sessionId, [...state.messages]);
      if (effectiveSessionId.value === sessionId) {
        messages.value = [...state.messages];
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
  retry.deadlineTimer = setTimeout(() => {
    retry.deadlineTimer = null;
    handleSendFailure(sessionId, state, `Timed out waiting for a response (${RETRY_TIMEOUT_MS / 1000}s without activity)`);
  }, RETRY_TIMEOUT_MS);
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

/**
 * Handle a failed send attempt. Shows the failure on the chat (with a "try
 * x/3" label), then either auto-retries after a short delay or — after the
 * last attempt — leaves the final error visible. Never leaves the UI stuck on
 * a bare spinner.
 */
function handleSendFailure(sessionId: string, state: SessionState, errMsg: string) {
  const isActiveSession = store.activeSessionId === sessionId;
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
  if (retry && retry.attempts < RETRY_MAX) {
    const attempt = retry.attempts;
    state.messages.push({
      role: "agent",
      content: `⚠️ Request failed (try ${attempt}/${RETRY_MAX}): ${errMsg}，${RETRY_DELAY_MS / 1000}s 后自动重试...`,
      isProcessing: true,
    });
    if (isActiveSession) {
      messages.value = [...state.messages];
      msgStore.setMessages(sessionId, messages.value);
    }
    retry.attempts = attempt + 1;
    if (retry.deadlineTimer) { clearTimeout(retry.deadlineTimer); retry.deadlineTimer = null; }
    retry.timer = setTimeout(() => {
      retry.timer = null;
      retrySend(sessionId, state);
    }, RETRY_DELAY_MS);
    return;
  }

  // Final failure — surface the real error and give up retrying.
  state.messages.push({
    role: "agent",
    content: retry ? `Error: ${errMsg}（已自动重试 ${retry.attempts} 次）` : `Error: ${errMsg}`,
  });
  clearRetry(state);
  if (isActiveSession) {
    messages.value = [...state.messages];
    isProcessing.value = false;
    msgStore.setMessages(sessionId, messages.value);
  }
}

/** Re-send the last failed text (attempts N+1). */
function retrySend(sessionId: string, state: SessionState) {
  const retry = state.retry;
  if (!retry) return;
  const sess = store.sessions.find(s => s.id === sessionId);
  if (!sess || sess.status === 'stopped' || sess.status === 'error') {
    clearRetry(state);
    return;
  }
  state.isProcessing = true;
  state.turnStartTime = Date.now();
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
  if (!sess || sess.status === 'stopped') return;
  const t0 = performance.now();
  try {
    handleAcpEventInner(sessionId, p);
  } finally {
    recordEvent(performance.now() - t0, p.content?.length || 0);
  }
}

function handleAcpEventInner(sessionId: string, p: AcpPayload) {
  const detail = {
    type: p.type,
    content: p.content?.substring(0, 100),
    thinking: p.type === 'thinking' ? p.content?.substring(0, 100) : undefined,
    tool_name: p.tool_name,
    tool_status: p.status,
    input: p.input?.substring(0, 100),
    output: p.output?.substring(0, 100),
    stop_reason: p.stop_reason,
    error: p.message,
  };
  const state = getSessionState(sessionId);
  const isActiveSession = store.activeSessionId === sessionId;
  // Any live event (thinking/text/tool, etc.) means the agent is still working —
  // push back the no-activity timeout that backs the auto-retry loop. finish
  // and error are handled by their own branches.
  if (state.retry && p.type !== "finish" && p.type !== "error") {
    resetRetryDeadline(sessionId, state);
  }
  // 仅活动会话打逐事件日志——每 chunk 的 console.log + JSON.stringify 在
  // 主线程上是真实开销，多个后台会话并行流式时尤其明显（统计仍由 diag 记录）
  if (isActiveSession) {
    console.log(`[ACP EVENT] ${sessionId.substring(0,8)} type=${p.type}`, JSON.stringify(detail));
  }

  switch (p.type) {
    case "start":
      // Save previous agent message before resetting activeContent.
      // Gemini sends multiple messages per turn; agent_message_end emits
      // Start to begin a new bubble. Without saving here, only the last
      // message would be persisted (or none if activeContent was reset).
      if (state.activeContent && sessionId) {
        saveConversationMessage(sessionId, "agent", state.activeContent).catch(()=>{});
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
        messages.value = [...state.messages];
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
        messages.value = [...state.messages];
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
      const lt = ensureAgentMsg(state);
      lt.content = state.activeContent;
      if (isActiveSession) {
        messages.value = [...state.messages];
        msgStore.setMessages(sessionId, messages.value);
      }
      break;
    case "tool_call": {
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
        messages.value = [...state.messages];
        msgStore.setMessages(sessionId, messages.value);
      }
      break;
    }
    case "tool_result": {
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
        messages.value = [...state.messages];
        msgStore.setMessages(sessionId, messages.value);
      }
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
        messages.value = [...state.messages];
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
        messages.value = [...state.messages];
        msgStore.setMessages(sessionId, messages.value);
      }
      break;
    }
    case "finish":
      clearRetry(state);
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
        saveConversationMessage(sessionId, "agent", state.activeContent).catch(()=>{});
      }
      // Mark session as idle (process still alive, waiting for next message)
      const sess = store.sessions.find(s => s.id === sessionId);
      if (sess && sess.status === 'running') {
        sess.status = 'idle';
        sess.newlyCompleted = true;
        store.sessions = [...store.sessions];
      }
      if (isActiveSession) {
        messages.value = [...state.messages];
        isProcessing.value = false;
        msgStore.setMessages(sessionId, messages.value);
      }
      break;
    case "error":
      handleSendFailure(sessionId, state, p.message || "Unknown");
      break;
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
  if (!target.closest('.permission-selector') && !target.closest('.model-selector') && !target.closest('.dir-selector') && !target.closest('.more-agents-selector') && !target.closest('.message-list-dropdown') && !target.closest('.skills-selector')) {
    showPermissionDropdown.value = false;
    showModelDropdown.value = false;
    showDirMenu.value = false;
    showMoreAgents.value = false;
    showSkillsPopover.value = false;
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
  for (const [_, unlisten] of unlisteners) {
    try { unlisten(); } catch {}
  }
  unlisteners.clear();
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
      await store.createSession(a.id, a.display_name, dirPath.value||undefined, title, selectedModel.value || undefined, selectedMode.value, selectedPermissionMode.value, Array.from(selectedSkills.value));
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
      return;
    } finally {
      suppressSkillSync = false;
    }
  } else if (store.activeSession.status === 'stopped' || store.activeSession.status === 'error') {
    // Restart backend only if process is truly dead
    const s = store.activeSession;
    try {
      const dirPathForRestart = s.directory || dirPath.value || undefined;
      // If the session was restored after a webview reload, the backend process
      // may still be alive (only the webview reloaded). Stop it first so the
      // restart doesn't orphan the old agent process.
      try { await tauriStopSession(s.id); } catch {}
      await apiStartSession(s.cli, s.cliDisplayName, dirPathForRestart, s.id, s.model || undefined, selectedMode.value, selectedPermissionMode.value, Array.from(selectedSkills.value));
      s.status = 'running';
      s.freshAgentProcess = true;
      store.sessions = [...store.sessions];
      // 加载历史消息用于恢复会话上下文
      await loadSessionMessages(s.id);
    } catch (err) {
      console.error("Failed to restart session:", err);
      // Show error in chat instead of calling sendInput (which would fail with "No client for session")
      const state = getSessionState(s.id);
      state.messages.push({role:"user",content:userDisplay});
      state.messages.push({role:"agent",content:`Error: Failed to restart session. ${err}`});
      messages.value = [...state.messages];
      msgStore.setMessages(s.id, [...state.messages]);
      saveConversationMessage(s.id, "user", userDisplay).catch(()=>{});
      return;
    }
  }

  if (store.activeSession?.id) {
    const sid = effectiveSessionId.value;
    const state = getSessionState(sid);
    state.messages.push({role:"user",content:userDisplay});
    messages.value = [...state.messages];
    msgStore.setMessages(sid, [...state.messages]);
    saveConversationMessage(sid, "user", userDisplay).catch(()=>{});
  }

  if(store.activeSession) {
    const sessionId = effectiveSessionId.value;
    if (store.activeSession.status === 'idle') {
      store.activeSession.status = 'running';
      store.sessions = [...store.sessions];
    }
    // 如果是新启动的Agent进程，把历史消息传给后端作为上下文（最多2轮）
    let history: string[] | undefined = undefined;
    if (store.activeSession?.freshAgentProcess) {
      const sessionMsgs = msgStore.getMessages(sessionId);
      if (sessionMsgs.length > 1) {
        // 最多取最后2轮对话（4条消息：user/agent/user/agent）
        const recentMsgs = sessionMsgs.slice(-4);
        history = recentMsgs
          .filter(m => m.content)
          .map(m => `${m.role}: ${m.content}`);
      }
      // 发送完历史消息后，标记为非新进程
      store.activeSession.freshAgentProcess = false;
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
      // 进程才能让新模型生效。重启后 freshAgentProcess 会把历史作为上下文。
      await ensureAgentProcessUsesModel(sessionId, sessionModel);
    }
    // Arm the auto-retry loop before dispatching: the first failure shows
    // "try 1/3" on the chat and re-sends after a short delay, up to RETRY_MAX
    // attempts total. The timeout also catches agents that go silent without
    // ever reporting an error (otherwise the UI hangs on a bare spinner).
    const st = getSessionState(sessionId);
    st.retry = { attempts: 1, lastText: sendText, timer: null, deadlineTimer: null };
    startRetryDeadline(sessionId, st);
    sendInput(sessionId, sendText, history).catch(err=>{
      const state = getSessionState(sessionId);
      clearRetry(state);
      state.messages.push({role:"agent",content:`Error:${err}`});
      state.isProcessing = false;
      if (store.activeSessionId === sessionId) {
        messages.value = [...state.messages];
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
  if (used !== sessionModel && s.status !== 'stopped' && s.status !== 'error') {
    console.log(`[MODEL-CHANGE] ${sessionId}: ${used ?? '(none)'} -> ${sessionModel}, restarting agent process`);
    try { await tauriStopSession(sessionId); } catch {}
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
    const sid = effectiveSessionId.value;
    const state = getSessionState(sid);
    clearRetry(state);
    state.isProcessing = false;
    const lm = lastAgentMsg(state.messages);
    if (lm) lm.isProcessing = false;
    isProcessing.value = false;
    messages.value = [...state.messages];
    msgStore.setMessages(sid, [...state.messages]);
    await store.stopSession(sid);
  }
}

async function pickDirectory() {
  const selected = await open({ directory: true, multiple: false });
  if (selected && typeof selected === "string") {
    dirPath.value = selected;
    saveRecentDir(selected);
    recentDirs.value = loadRecentDirs();
  }
  showDirMenu.value = false;
}

// Sync messages to message store for search
watch(messages, (msgs) => {
  const sid = effectiveSessionId.value;
  if (sid) {
    msgStore.setMessages(sid, [...msgs]);
  }
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
        <div ref="messageContainer" class="h-full overflow-y-auto" @scroll="onChatScroll">
          <div class="max-w-4xl mx-auto px-6 pt-5 pb-40">
            <ChatMessages ref="chatMessagesRef" :messages="messages" :agent-id="selectedAgentId" />
            <div v-if="isSessionLoading && messages.length === 0" class="flex items-center justify-center py-8">
              <div class="flex items-center gap-2 text-gray-400">
                <div class="w-4 h-4 border-2 border-gray-300 border-t-gray-600 rounded-full animate-spin"></div>
                <span class="text-[13px]">Loading...</span>
              </div>
            </div>
          </div>
        </div>

        <!-- Scroll-to-bottom button -->
        <button
          v-if="showScrollToBottom"
          @click="scrollToBottom()"
          class="absolute bottom-6 left-1/2 -translate-x-1/2 w-9 h-9 rounded-full bg-white border border-gray-200 shadow-md flex items-center justify-center text-gray-500 hover:text-gray-700 hover:border-gray-300 hover:shadow-lg transition-all duration-200 cursor-pointer z-10"
          title="Scroll to bottom"
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
              <span class="text-[11px] font-semibold text-gray-500 uppercase tracking-wider">Messages</span>
              <span class="text-[10px] text-gray-400">{{ userMessages.length }} items</span>
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
              showMessageList ? 'bg-white text-gray-800 shadow-md' : ''
            ]"
            title="Message list"
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
                title="Skills"
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
                <span class="text-[11px] font-medium text-gray-500">Skills ({{ selectedSkills.size }}/{{ availableSkills.length }})</span>
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
                      ? 'bg-gray-900 border-gray-900 text-white'
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
                      <div class="flex-1 min-w-0">
                        <div class="text-[12px] text-gray-800 truncate">{{ f.name }}</div>
                        <div class="text-[10px] text-gray-400 truncate">{{ f.path }}</div>
                      </div>
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
                        <span class="absolute left-full ml-2 top-1/2 -translate-y-1/2 px-2.5 py-1.5 text-[11px] text-white bg-gray-900 rounded-lg opacity-0 invisible group-hover:opacity-100 group-hover:visible transition-all duration-150 whitespace-nowrap z-50 shadow-lg max-w-[200px]">
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
                      <span>{{ selectedModelInfo?.alias || selectedModelInfo?.name || 'Select Model' }}</span>
                      <span v-if="selectedModelInfo && selectedModelInfo.alias"
                            class="text-[10px] text-gray-400 ml-1">{{ selectedModelInfo.name }}</span>
                    </span>
                    <ChevronDown :size="10" :class="props.compact ? 'hidden' : ''" />
                  </button>
                  <div v-if="showModelDropdown" class="absolute bottom-full right-0 mb-1 w-64 bg-white rounded-xl shadow-lg border border-gray-100 overflow-hidden z-50 max-h-72 overflow-y-auto">
                    <div class="px-3 py-1.5 text-[10px] font-semibold text-gray-400 uppercase tracking-wider">Commercial Models</div>
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
                        Local Models
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
                        <span v-if="!isLocalModelRunning(model)" class="text-[10px] text-gray-400">Start server</span>
                      </div>
                    </div>
                    <div v-if="modelList.length === 0" class="px-3 py-4 text-center text-[12px] text-gray-400">
                      No models configured
                    </div>
                  </div>
                </div>

                <button @click="noThinking = !noThinking" :disabled="isProcessing" class="p-1.5 rounded-lg transition-colors duration-150 mr-2 flex-shrink-0" :class="[noThinking ? 'bg-amber-100 text-amber-700 hover:bg-amber-200 cursor-pointer' : 'bg-gray-100 text-gray-400 hover:bg-gray-200 cursor-pointer', isProcessing && 'opacity-50 cursor-not-allowed']" title="Toggle reasoning mode">
                  <Sparkles :size="14" />
                </button>
                <button v-if="!isProcessing" @click="handleSend" :disabled="!inputText.trim() || !selectedModel" class="flex items-center gap-1.5 px-3 py-1.5 rounded-xl transition-all duration-200 text-[12px] font-medium shadow-sm relative flex-shrink-0" :class="(inputText.trim() && selectedModel)?'bg-gray-900 text-white hover:bg-gray-800 cursor-pointer':'bg-gray-200 text-gray-400 cursor-not-allowed'">
                  <Send :size="12" />Send
                  <span v-if="!selectedModel" class="absolute -top-8 right-0 px-2 py-1 text-[10px] text-white bg-gray-700 rounded-lg opacity-0 hover:opacity-100 transition-opacity whitespace-nowrap pointer-events-none z-50">Please select a model</span>
                </button>
                <button v-else @click="handleStop" class="flex items-center gap-1.5 px-3 py-1.5 rounded-xl bg-gray-900 text-white hover:bg-red-600 transition-all duration-200 cursor-pointer text-[12px] font-medium shadow-sm flex-shrink-0"><Square :size="12" />Stop</button>
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
          What are we building today?
        </p>

        <div class="flex justify-center mb-5">
          <div class="inline-flex bg-gray-100 rounded-2xl p-1 gap-0.5">
            <template v-if="enabledAgents.length > 0">
              <button v-for="a in enabledAgents" :key="a.id" @click="selectedAgentId=a.id" :class="['flex items-center gap-1.5 px-4 py-2 rounded-xl text-[13px] font-medium transition-all duration-200 cursor-pointer',selectedAgentId===a.id?'bg-white text-gray-900 shadow-sm':'text-gray-500 hover:text-gray-700']">
                <AgentIcon :agent-id="a.id" />
                {{ a.display_name }}
              </button>
            </template>
            <button v-else @click="router.push('/settings/agents')" class="flex items-center gap-1.5 px-4 py-2 rounded-xl text-[13px] font-medium text-gray-400 hover:text-gray-600 transition-all duration-200 cursor-pointer">
              <Package :size="14" />
              Install an Agent
            </button>
            <!-- More agents dropdown -->
            <div class="relative more-agents-selector">
              <button @click.stop="showMoreAgents = !showMoreAgents" class="flex items-center gap-1 px-3 py-2 rounded-xl text-[13px] font-medium text-gray-500 hover:text-gray-700 transition-all duration-200 cursor-pointer">
                More
                <ChevronDown :size="10" :class="showMoreAgents ? 'rotate-180' : ''" class="transition-transform duration-150" />
              </button>
              <div v-if="showMoreAgents" class="absolute top-full left-0 mt-1 w-44 bg-white rounded-xl shadow-lg border border-gray-100 overflow-hidden z-50 py-1">
                <div class="px-3 py-1.5 text-[10px] font-semibold text-gray-400 uppercase tracking-wider">Coming Soon</div>
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
          <p class="text-[15px] font-semibold text-gray-800 mb-1">Welcome to RunJam</p>
          <p class="text-[13px] text-gray-500 mb-5 max-w-sm mx-auto leading-relaxed">
            To start chatting with an AI agent, you need to install an agent and configure at least one model.
          </p>
          <div class="flex items-center justify-center gap-3">
            <button @click="router.push('/settings/agents')" class="inline-flex items-center gap-2 px-4 py-2.5 rounded-xl text-[13px] font-semibold bg-blue-600 text-white hover:bg-blue-700 active:scale-[0.98] transition-all cursor-pointer shadow-sm">
              <Download :size="15" /> Install Agent
            </button>
            <button @click="router.push('/settings/models?action=add')" class="inline-flex items-center gap-2 px-4 py-2.5 rounded-xl text-[13px] font-semibold bg-white text-gray-700 border border-gray-200 hover:bg-gray-50 active:scale-[0.98] transition-all cursor-pointer shadow-sm">
              <Wand2 :size="15" /> Configure Model
            </button>
          </div>
        </div>

        <!-- Selected agent not installed -->
        <div v-else-if="selectedAgent && !selectedAgent.installed" class="mb-5 p-5 rounded-2xl border border-amber-200 bg-amber-50 text-center">
          <p class="text-[14px] font-semibold text-amber-800 mb-1">{{ selectedAgent.display_name }} is not installed</p>
          <p class="text-[13px] text-amber-600 mb-1">Install it to start chatting with AI.</p>
          <p v-if="!hasAnyModel" class="text-[12px] text-amber-500 mb-3">You'll also need to configure a model after installation.</p>
          <button @click="router.push(`/settings/agents/${selectedAgentId}`)" class="inline-flex items-center gap-1.5 px-4 py-2 rounded-xl text-[13px] font-semibold bg-amber-600 text-white hover:bg-amber-700 active:scale-[0.98] transition-all cursor-pointer shadow-sm">
            <Download :size="14" /> Install {{ selectedAgent.display_name }}
          </button>
        </div>

        <!-- Agent installed but no models -->
        <div v-else-if="hasAnyAgentInstalled && !hasAnyModel" class="mb-5 p-4 rounded-2xl border border-purple-200 bg-purple-50 flex items-center gap-3">
          <Wand2 :size="18" class="text-purple-500 flex-shrink-0" />
          <div class="flex-1">
            <p class="text-[13px] font-medium text-purple-800">No model configured yet</p>
            <p class="text-[12px] text-purple-500">Add a model provider to start chatting.</p>
          </div>
          <button @click="router.push('/settings/models?action=add')" class="flex-shrink-0 px-3 py-1.5 rounded-lg text-[12px] font-semibold bg-purple-600 text-white hover:bg-purple-700 active:scale-[0.98] transition-all cursor-pointer">
            Add Model
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
              title="Skills"
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
              <span class="text-[11px] font-medium text-gray-500">Skills ({{ selectedSkills.size }}/{{ availableSkills.length }})</span>
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
                    ? 'bg-gray-900 border-gray-900 text-white'
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
                      <span class="absolute left-full ml-2 top-1/2 -translate-y-1/2 px-2.5 py-1.5 text-[11px] text-white bg-gray-900 rounded-lg opacity-0 invisible group-hover:opacity-100 group-hover:visible transition-all duration-150 whitespace-nowrap z-50 shadow-lg max-w-[200px]">
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
                    <span>{{ selectedModelInfo?.alias || selectedModelInfo?.name || 'Select Model' }}</span>
                    <span v-if="selectedModelInfo && selectedModelInfo.alias" 
                          class="text-[10px] text-gray-400 ml-1">{{ selectedModelInfo.name }}</span>
                  </span>
                  <ChevronDown :size="10" />
                </button>
                <div v-if="showModelDropdown" class="absolute bottom-full right-0 mb-1 w-64 bg-white rounded-xl shadow-lg border border-gray-100 overflow-hidden z-50 max-h-72 overflow-y-auto">
                    <div class="px-3 py-1.5 text-[10px] font-semibold text-gray-400 uppercase tracking-wider">Commercial Models</div>
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
                        Local Models
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
                        <span v-if="!isLocalModelRunning(model)" class="text-[10px] text-gray-400">Start server</span>
                      </div>
                    </div>
                    <div v-if="modelList.length === 0" class="px-3 py-4 text-center text-[12px] text-gray-400">
                      No models configured
                    </div>
                    <div class="border-t border-gray-100">
                      <button
                        @click="router.push('/settings/models?action=add')"
                      class="w-full flex items-center gap-2 px-3 py-2.5 text-left text-[12px] text-gray-600 hover:bg-gray-50 transition-colors cursor-pointer font-medium"
                    >
                      <Plus :size="13" class="text-gray-400" />
                      Add Model
                    </button>
                  </div>
                </div>
              </div>

              <!-- No thinking toggle -->
              <button @click="noThinking = !noThinking" class="p-1.5 rounded-lg transition-colors duration-150 flex-shrink-0 mr-2" :class="noThinking ? 'bg-amber-100 text-amber-700 hover:bg-amber-200 cursor-pointer' : 'bg-gray-100 text-gray-400 hover:bg-gray-200 cursor-pointer'" title="Toggle reasoning mode">
                <Sparkles :size="14" />
              </button>
              <!-- Send button -->
              <button @click="handleSend" :disabled="!inputText.trim() || !selectedModel" class="flex items-center gap-1.5 px-3 py-1.5 rounded-xl transition-all duration-200 text-[12px] font-medium shadow-sm flex-shrink-0 relative" :class="inputText.trim() && selectedModel ?'bg-gray-900 text-white hover:bg-gray-800 cursor-pointer':'bg-gray-200 text-gray-400 cursor-not-allowed'">
                <Send :size="12" />Send
                <span v-if="!selectedModel" class="absolute -top-8 right-0 px-2 py-1 text-[10px] text-white bg-gray-700 rounded-lg opacity-0 hover:opacity-100 transition-opacity whitespace-nowrap pointer-events-none z-50">Please select a model</span>
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
              <span v-if="!dirPath">work in a project</span>
              <span v-else class="text-gray-700 font-medium">{{ dirPath.split('/').pop() }}</span>
            </button>
            <button
              v-if="dirPath"
              @click="dirPath = ''"
              class="p-1 rounded-md text-gray-400 hover:text-gray-600 hover:bg-gray-200/60 transition-colors cursor-pointer"
              title="Clear project"
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
              <div class="px-3 py-1.5 text-[10px] font-semibold text-gray-400 uppercase tracking-wider">Recent Projects</div>
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
                  title="Remove"
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
              No project
            </button>
            <!-- Open a new folder -->
            <button
              @click="pickDirectory"
              class="w-full flex items-center gap-2 px-3 py-2 text-left text-[12px] text-gray-700 hover:bg-gray-50 transition-colors cursor-pointer"
            >
              <FolderPlus :size="13" class="text-gray-400" />
              Open a new folder...
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
          title="Send feedback"
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
          <span class="text-[15px] font-semibold text-gray-900">Send Feedback</span>
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
          <p class="text-[14px] font-medium text-gray-900">Thank you for your feedback!</p>
          <p class="text-[12px] text-gray-400">We'll review it shortly.</p>
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
                ? 'bg-gray-900 border-gray-900 text-white'
                : 'bg-white border-gray-200 text-gray-600 hover:bg-gray-50',
            ]"
          >
            {{ t.label }}
          </button>
        </div>

        <textarea
          v-model="feedbackContent"
          rows="5"
          placeholder="Describe your feedback..."
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
            {{ feedbackSending ? "Sending…" : "Submit" }}
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
</style>
