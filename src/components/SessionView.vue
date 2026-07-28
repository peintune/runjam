<script setup lang="ts">
import { ref, watch, onMounted, onBeforeUnmount, computed, nextTick } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useRouter } from "vue-router";
import { useWorkspaceStore } from "../stores/useWorkspaceStore";
import { useMessageStore } from "../stores/useMessageStore";
import { useAgentStore } from "../stores/useAgentStore";
import { getModels, getLastAgent, setLastAgent, getAgentModels, setAgentModel, type ModelEntry, getProviderByName } from "../api/models";
import { getProviderLogo } from "../utils/providerIcons";
import { sendInput, startSession as apiStartSession } from "../api/sessions";
import { saveConversationMessage, getConversationMessages, saveSession } from "../api/search";
import { getAgentStatuses, type AgentInfo } from "../api/agents";
import { open } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import ChatMessages, { type Message } from "./ChatMessages.vue";
import AgentIcon from "./AgentIcon.vue";
import { Send, Square, Download, Shield, ChevronDown, ArrowDown, Folder, X, FolderPlus, Sparkles, HelpCircle, Plus, Package, Wand2 } from "lucide-vue-next";
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

function scrollToMessage(msgIndex: number) {
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
    const allUserEls = el.querySelectorAll('[data-user-msg-index]');
    const targetMsg = messages.value[msgIndex];
    if (!targetMsg) return;
    for (const userEl of allUserEls) {
      const textEl = userEl.querySelector('.msg-user-bubble');
      if (textEl && textEl.textContent === targetMsg.content) {
        userEl.scrollIntoView({ behavior: 'smooth', block: 'center' });
        return;
      }
    }
  });
}

const agents = ref<AgentInfo[]>(agentStore.agents.length > 0 ? agentStore.agents : []);
const selectedAgentId = ref<string>("claude-code");

const enabledAgents = computed(() => agents.value.filter(a => a.enabled));
const selectedAgent = computed(() => agents.value.find(a => a.id === selectedAgentId.value));

const modelList = ref<ModelEntry[]>(agentStore.models.length > 0 ? agentStore.models : []);
const selectedModel = ref("");
const assignedModels = ref<ModelEntry[]>([]);

// Onboarding state
const hasAnyAgentInstalled = computed(() => agents.value.some(a => a.installed));
const hasAnyModel = computed(() => modelList.value.length > 0);

const inputText = ref("");
const dirPath = ref("");
const showDirMenu = ref(false);
const showMoreAgents = ref(false);

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

const placeholderText = "Ask anything — write code, debug, refactor, run parallel agents, orchestrate multi-agent workflows, or explore your entire codebase. What do you want to build?";
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
const newSessionTextarea = ref<HTMLTextAreaElement | null>(null);
const showModelDropdown = ref(false);
const runningServerPort = ref(0);
const runningServerModel = ref<string | null>(null);
const { showWarning } = useToast();

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

watch(() => store.activeSessionId, async (newId) => {
  activeThinking.value = ""; activeContent.value = ""; thoughtDuration.value = "";
  inputText.value = "";
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
}

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
      if (store.activeSessionId === sessionId) {
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

function handleAcpEvent(sessionId: string, p: AcpPayload) {
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
  console.log(`[ACP EVENT] ${sessionId.substring(0,8)} type=${p.type}`, JSON.stringify(detail));
  const state = getSessionState(sessionId);
  const isActiveSession = store.activeSessionId === sessionId;

  switch (p.type) {
    case "start":
      state.messages.push({ role: "agent", content: "", startTime: Date.now(), isProcessing: true });
      state.activeThinking = ""; state.activeContent = ""; state.thoughtDuration = "";
      state.thinkingStartTime = 0;
      state.turnStartTime = Date.now();
      state.isProcessing = true;
      if (isActiveSession) {
        messages.value = [...state.messages];
        isProcessing.value = true;
        msgStore.setMessages(sessionId, [...state.messages]);
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
        state.activeThinking += p.content;
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
        msgStore.setMessages(sessionId, [...state.messages]);
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
      state.activeContent += (p.content||"");
      const lt = ensureAgentMsg(state);
      lt.content = state.activeContent;
      if (isActiveSession) {
        messages.value = [...state.messages];
        msgStore.setMessages(sessionId, [...state.messages]);
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
        msgStore.setMessages(sessionId, [...state.messages]);
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
        msgStore.setMessages(sessionId, [...state.messages]);
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
        msgStore.setMessages(sessionId, [...state.messages]);
      }
      break;
    }
    case "interaction": {
      // Agent is asking the user to choose from options
      const im = ensureAgentMsg(state);
      // If there's a fresh interaction, we create it as a new mini-message-like block
      // But since interaction can come mid-stream, attach to last agent message
      const currentSid = store.activeSessionId || sessionId;
      im.interaction = {
        prompt: p.prompt || "",
        options: p.options || [],
        sessionId: currentSid,
      };
      if (isActiveSession) {
        messages.value = [...state.messages];
        msgStore.setMessages(sessionId, [...state.messages]);
      }
      break;
    }
    case "finish":
      state.isProcessing = false;
      state.thinkingStartTime = 0;
      // Mark all agent messages in this turn as not processing
      for (const m of state.messages) {
        if (m.role === 'agent') m.isProcessing = false;
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
        msgStore.setMessages(sessionId, [...state.messages]);
      }
      break;
    case "error":
      state.isProcessing = false;
      state.thinkingStartTime = 0;
      // Mark all agent messages as not processing
      for (const m of state.messages) {
        if (m.role === 'agent') m.isProcessing = false;
      }
      // Remove empty "start" placeholder so "..." dots disappear
      const le = lastAgentMsg(state.messages);
      if (le && !le.content && !le.thinking) {
        state.messages.pop();
      }
      state.messages.push({ role: "agent", content: `Error: ${p.message||"Unknown"}` });
      if (isActiveSession) {
        messages.value = [...state.messages];
        isProcessing.value = false;
        msgStore.setMessages(sessionId, [...state.messages]);
      }
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
  if (!target.closest('.permission-selector') && !target.closest('.model-selector') && !target.closest('.dir-selector') && !target.closest('.more-agents-selector') && !target.closest('.message-list-dropdown')) {
    showPermissionDropdown.value = false;
    showModelDropdown.value = false;
    showDirMenu.value = false;
    showMoreAgents.value = false;
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
  const savedModel = await loadSessionModel(id);
  if (savedModel && modelList.value.some(m => m.id === savedModel)) {
    selectedModel.value = savedModel;
  } else if (assignedModels.value.length > 0) {
    selectedModel.value = assignedModels.value[0].id;
  }
}, { immediate: true });

watch(messages, async () => {
  await nextTick();
  if (messageContainer.value) {
    messageContainer.value.scrollTop = messageContainer.value.scrollHeight;
  }
}, { deep: true });

async function handleSend() {
  const text = inputText.value.trim(); if(!text)return;
  
  if (!selectedModel.value) {
    showWarning("Please select a model before sending.");
    return;
  }
  
  inputText.value = "";

  if(!store.activeSession) {
    const a=agents.value.find(a=>a.id===selectedAgentId.value)!;
    const title = text.substring(0, 30) + (text.length > 30 ? '...' : '');
    await store.createSession(a.id, a.display_name, dirPath.value||undefined, title, selectedModel.value || undefined, selectedMode.value, selectedPermissionMode.value);
  } else if (store.activeSession.status === 'stopped' || store.activeSession.status === 'error') {
    // Restart backend only if process is truly dead
    const s = store.activeSession;
    try {
      const dirPathForRestart = s.directoryId
        ? store.directories.find(d => d.id === s.directoryId)?.path
        : undefined;
      await apiStartSession(s.cli, s.cliDisplayName, dirPathForRestart || dirPath.value || undefined, s.id, s.model || undefined, selectedMode.value, selectedPermissionMode.value);
      s.status = 'running';
      s.freshAgentProcess = true;
      store.sessions = [...store.sessions];
      // 加载历史消息用于恢复会话上下文
      await loadSessionMessages(s.id);
    } catch (err) {
      console.error("Failed to restart session:", err);
      // Show error in chat instead of calling sendInput (which would fail with "No client for session")
      const state = getSessionState(s.id);
      state.messages.push({role:"user",content:text});
      state.messages.push({role:"agent",content:`Error: Failed to restart session. ${err}`});
      messages.value = [...state.messages];
      msgStore.setMessages(s.id, [...state.messages]);
      saveConversationMessage(s.id, "user", text).catch(()=>{});
      return;
    }
  }

  if (store.activeSession?.id) {
    const state = getSessionState(store.activeSession.id);
    state.messages.push({role:"user",content:text});
    messages.value = [...state.messages];
    msgStore.setMessages(store.activeSession.id, [...state.messages]);
    saveConversationMessage(store.activeSession.id, "user", text).catch(()=>{});
  }

  if(store.activeSession) {
    const sessionId = store.activeSession.id;
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
    sendInput(sessionId, text, history).catch(err=>{
      const state = getSessionState(sessionId);
      state.messages.push({role:"agent",content:`Error:${err}`});
      state.isProcessing = false;
      if (store.activeSessionId === sessionId) {
        messages.value = [...state.messages];
        isProcessing.value = false;
      }
      msgStore.setMessages(sessionId, [...state.messages]);
    });
  }
}

function handleModelSelect(model: ModelEntry) {
  selectedModel.value = model.id;
  showModelDropdown.value = false;
  
  saveSessionModel(selectedAgentId.value, model.id);
  // Immediately update agent config so the agent uses the right model name
  setAgentModel(selectedAgentId.value, model.id).catch(() => {});
}

async function handleStop() {
  if (store.activeSession) {
    const state = getSessionState(store.activeSession.id);
    state.isProcessing = false;
    const lm = lastAgentMsg(state.messages);
    if (lm) lm.isProcessing = false;
    isProcessing.value = false;
    messages.value = [...state.messages];
    msgStore.setMessages(store.activeSession.id, [...state.messages]);
    await store.stopSession(store.activeSession.id);
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
  if (store.activeSessionId) {
    msgStore.setMessages(store.activeSessionId, [...msgs]);
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
          </div>
        </div>
      </div>
      
      <div class="flex-1 relative min-h-0">
        <div ref="messageContainer" class="h-full overflow-y-auto" @scroll="checkScrollPosition">
          <div class="max-w-4xl mx-auto px-6 pt-5 pb-40">
            <ChatMessages :messages="messages" :agent-id="selectedAgentId" />
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
          <div class="rounded-2xl border border-gray-200 bg-white focus-within:border-gray-300 shadow-[0_2px_12px_rgba(0,0,0,0.06)] focus-within:shadow-[0_4px_16px_rgba(0,0,0,0.08)] transition-all duration-150">
            <textarea v-model="inputText" placeholder="" rows="2" class="w-full px-4 pt-3 bg-transparent border-none outline-none resize-none text-[14px] text-gray-900 leading-relaxed" @keydown.enter.exact.prevent="handleSend" :disabled="isProcessing" />
            <div class="flex items-center justify-end px-3 pb-2 gap-2">
              <div class="flex items-center gap-2 flex-shrink-0">
                <!-- Permission mode selector -->
                <div class="relative permission-selector">
                  <button @click.stop="showPermissionDropdown = !showPermissionDropdown"
                    class="flex items-center gap-1 px-2.5 py-1.5 rounded-lg text-[11px] font-medium text-gray-600 hover:bg-gray-50 transition-all duration-150 cursor-pointer">
                    <Shield :size="11" />
                    {{ permissionModeLabel }}
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
                    <span class="text-left">
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
                  </div>
                </div>

                <button v-if="!isProcessing" @click="noThinking = !noThinking" class="p-1.5 rounded-lg transition-colors duration-150 mr-2" :class="noThinking ? 'bg-amber-100 text-amber-700 hover:bg-amber-200 cursor-pointer' : 'bg-gray-100 text-gray-400 hover:bg-gray-200 cursor-pointer'" title="Toggle reasoning mode">
                  <Sparkles :size="14" />
                </button>
                <button v-if="!isProcessing" @click="handleSend" :disabled="!inputText.trim() || !selectedModel" class="p-1.5 rounded-lg transition-colors duration-150 relative" :class="(inputText.trim() && selectedModel)?'bg-gray-900 text-white hover:bg-gray-800 cursor-pointer':'bg-gray-200 text-gray-400 cursor-not-allowed'">
                  <Send :size="14" />
                  <span v-if="!selectedModel" class="absolute -top-8 right-0 px-2 py-1 text-[10px] text-white bg-gray-700 rounded-lg opacity-0 hover:opacity-100 transition-opacity whitespace-nowrap pointer-events-none z-50">Please select a model</span>
                </button>
                <button v-else @click="handleStop" class="flex items-center gap-1.5 px-3 py-1.5 rounded-xl bg-gray-900 text-white hover:bg-red-600 transition-all duration-200 cursor-pointer text-[12px] font-medium shadow-sm"><Square :size="12" />Stop</button>
              </div>
            </div>
          </div>
        </div>
      </div>
    </template>

    <div v-else class="flex-1 flex items-center justify-center px-8">
      <div class="w-full max-w-[640px]">
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

        <div v-else class="rounded-t-2xl border border-gray-200 bg-white focus-within:border-gray-300 focus-within:shadow-sm transition-all duration-150 relative">
          <textarea ref="newSessionTextarea" v-model="inputText" placeholder="" rows="4" class="w-full px-4 pt-4 bg-transparent border-none outline-none resize-none text-[15px] text-gray-900 leading-relaxed" @keydown.enter.exact.prevent="handleSend" />
          <div v-if="!inputText" class="absolute left-4 top-4 pointer-events-none text-[15px] text-gray-400 leading-relaxed">
            {{ typingPlaceholder }}<span class="animate-pulse">|</span>
          </div>
          <div class="flex items-center justify-end px-3 pb-2 gap-2">
            <div class="flex items-center gap-2 flex-shrink-0">
              <!-- Permission mode selector -->
              <div class="relative permission-selector">
                <button @click.stop="showPermissionDropdown = !showPermissionDropdown"
                  class="flex items-center gap-1 px-2.5 py-1.5 rounded-lg text-[11px] font-medium text-gray-600 hover:bg-gray-50 transition-all duration-150 cursor-pointer">
                  <Shield :size="11" />
                  {{ permissionModeLabel }}
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
                  <span class="text-left">
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
              <button @click="handleSend" :disabled="!inputText.trim() || !selectedModel" class="p-1.5 rounded-lg transition-colors duration-150 flex-shrink-0 relative" :class="inputText.trim() && selectedModel ?'bg-gray-900 text-white hover:bg-gray-800 cursor-pointer':'bg-gray-200 text-gray-400 cursor-not-allowed'">
                <Send :size="14" />
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
  </main>
</template>
