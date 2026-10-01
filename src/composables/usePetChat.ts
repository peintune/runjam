/**
 * Desktop-pet conversation engine.
 *
 * The pet popup reuses the real session machinery rather than talking to the
 * model directly: questions go through a genuine agent session (ACP), so the
 * pet inherits the same models, the same permission handling and the same
 * streaming events as the main window — "复用新建会话的能力".
 *
 * It also reuses the main window's *rendering*: the transcript is the shared
 * `ChatMessages` component and the messages are the same `Message` shape, so a
 * turn looks identical in both places (markdown, thinking, tool calls,
 * permission prompts, attachments at the bottom).
 *
 * Behaviour agreed with the product owner:
 * - The pet owns a *dedicated* session (never the main window's active one),
 *   but that session IS persisted and visible in the sidebar, so the user can
 *   go back and read the full history later.
 * - It inherits the current default agent + default model, so it works with
 *   zero configuration.
 * - If more than 60 minutes pass without a new question, the next question
 *   starts a FRESH session instead of continuing the stale one.
 *
 * Sessions are app-global (the backend `SessionManager` lives on the
 * AppHandle), so a session created here can be driven from this window and its
 * `acp:<id>` events — which are broadcast to every window — are received here.
 */
import { ref, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, emit, type UnlistenFn } from "@tauri-apps/api/event";
// The popup renders its transcript with the main window's own message list, so
// it consumes/lists the very same `Message` shape (thinking, tool calls,
// permission prompts). Reusing the type — rather than a reduced copy — is what
// keeps the two surfaces in sync as the main window evolves.
import type { Message } from "../components/ChatMessages.vue";
import {
  startSession as apiStartSession,
  sendInput,
  stopSession,
  sessionAlive,
  setSessionPermissionMode,
} from "../api/sessions";
import {
  getLastAgent,
  setLastAgent,
  getModels,
  getAgentModels,
  getSessionModel,
  setSessionModel,
  getAgentPermissionMode,
  setAgentPermissionMode,
  type ModelEntry,
} from "../api/models";
import { getAgentStatuses, type AgentInfo } from "../api/agents";
import { listSkills, type SkillInfo } from "../api/sessions";
import { setReasoningDisabled } from "../api/proxy";
import { useLlamaStore } from "../stores/useLlamaStore";
import {
  buildAttachmentPayload,
  attachmentDisplaySuffix,
  pickAttachedFiles,
  type AttachedFile,
} from "./useAttachments";
import {
  saveSession,
  saveConversationMessage,
  updateSessionModel,
  touchSession,
} from "../api/search";

/** Idle window after which the next question starts a new session. */
export const PET_IDLE_TIMEOUT_MS = 60 * 60 * 1000;

/** Broadcast (to every window) so the main window can refresh its session list
 *  when the pet creates or touches a conversation — the pet's sessions are
 *  persisted and must appear in the sidebar. */
export const PET_SESSION_CHANGED_EVENT = "pet:session-changed";

/** Broadcast from the popup to the main window: open `sessionId` there (and
 *  bring the main window to the front). */
export const PET_OPEN_IN_MAIN_EVENT = "pet:open-in-main";

/**
 * Magic prefix the backend uses to push the ACP session id through a `text`
 * event (see `acp_client.rs`). Not model output — must never be displayed.
 */
export const ACP_SESSION_ID_MARKER = "__ACP_SESSION_ID__";

/**
 * Whether a stored pet conversation is still within its idle window.
 *
 * The pet reuses the previous conversation until the user has been idle for
 * `PET_IDLE_TIMEOUT_MS`; only then does the next question start a brand-new
 * session. Exported (and pure) so the rule is unit-testable without the Tauri
 * runtime.
 */
export function isPetSessionFresh(
  lastActiveAt: number,
  now: number,
  timeoutMs: number = PET_IDLE_TIMEOUT_MS,
): boolean {
  // A non-finite / negative age (clock skew, corrupt storage) is never fresh.
  const age = now - lastActiveAt;
  if (!Number.isFinite(age) || age < 0) return false;
  return age < timeoutMs;
}

/**
 * Permission mode to run the pet's session with.
 *
 * The pet popup is deliberately minimal, so it never renders the permission
 * prompts that a session would show — it simply cannot answer them. The
 * backend defaults an omitted mode to `ask_approval`, which would therefore
 * stall the very first tool call forever (no `finish` event ever arrives and
 * the composer stays disabled). We mirror the main window's per-agent default
 * (`approve_for_me`) so the pet always keeps making progress unattended.
 */
export function resolvePetPermissionMode(saved: string | null | undefined): string {
  return saved && saved.trim() ? saved.trim() : "approve_for_me";
}

/**
 * Fold one streamed `text` chunk into the accumulated assistant text.
 *
 * ACP agents do not agree on chunk semantics: Claude and Gemini send *full
 * snapshots* (each chunk restates the whole message so far) while Codex sends
 * *deltas*. Appending unconditionally makes snapshot agents repeat themselves
 * ("snowball duplication"). This mirrors the detection SessionView uses:
 * ignore trailing-whitespace differences, and treat a longer chunk that starts
 * with everything we already have as a replacement rather than an addition.
 */
export function mergeStreamedText(accumulated: string, chunk: string): string {
  if (!chunk) return accumulated;
  const accTrimmed = accumulated.trimEnd();
  if (accTrimmed && chunk.length > accTrimmed.length && chunk.startsWith(accTrimmed)) {
    return chunk;
  }
  return accumulated + chunk;
}

/**
 * Whether a streamed `text` chunk is internal protocol traffic rather than
 * model output.
 *
 * The backend pushes the ACP session id as a `text` event prefixed with
 * `__ACP_SESSION_ID__` (see `acp_client.rs`). It is protocol plumbing, not model
 * output, so without this it would render as the raw marker — the
 * `__ACP_SESSION_ID__xxxx` the user would otherwise see on every new
 * conversation. Exported so the rule is unit-testable.
 */
export function isInternalStreamChunk(content: string): boolean {
  return content.startsWith(ACP_SESSION_ID_MARKER);
}

/**
 * Last path segment of a model path ("a/b/Qwen3.gguf" → "Qwen3.gguf").
 *
 * llama.cpp reports the model as a full path while `models.json` stores a bare
 * file name (or another path form), so comparisons must be on the file name —
 * the same rule the main window's `getFilename` uses. Handles both separators so
 * a path recorded on Windows still matches.
 */
export function baseName(pathOrName: string): string {
  const parts = pathOrName.split(/[/\\]/);
  return parts[parts.length - 1] || pathOrName;
}

/**
 * Whether a model cannot be used from the popup because its local server is not
 * the one currently running.
 *
 * Only LOCAL (llama.cpp) models are affected: they are served by a single
 * process, so a model is usable only when it is the one that process has loaded.
 * Starting a server is a Settings-page action, so the popup greys these out and
 * routes the user there. Every non-local model is always usable.
 */
export function isLocalModelUnavailable(
  model: { provider: string; name: string },
  runningPort: number,
  runningModel: string | null,
): boolean {
  if (model.provider !== "llama") return false;
  if (!runningPort || !runningModel) return true;
  return baseName(runningModel) !== baseName(model.name);
}

/**
 * Settle every "still running" marker on the transcript after a turn is halted.
 *
 * ACP has no interrupt call, so stopping kills the agent process — which means
 * no `finish` event ever arrives to clear these flags. Left alone they keep the
 * composer locked (`busy`) and the transcript animating (the `now` tick) forever,
 * so a stopped session would look like it were still answering. The main window
 * has the same cleanup for the same reason.
 *
 * Tool calls are marked `failed`, not `completed`: the tool was interrupted, and
 * a success check would misreport what happened.
 *
 * Mutates in place (the message objects are reactive and shared with the view).
 */
export function settleInterruptedMessages(messages: PetMessage[]): void {
  for (const m of messages) {
    if (m.role !== "agent") continue;
    m.isProcessing = false;
    for (const tc of m.toolCalls ?? []) {
      if (tc.status === "started" || tc.status === "running") tc.status = "failed";
    }
  }
}

/** Session id + creation time of the pet's current conversation, persisted so
 *  reopening the pet (or restarting the app) within the idle window continues
 *  the same conversation. */
const STORAGE_KEY = "runjam.pet.session";

/** The configuration the user picked for the pet's next session (agent, model,
 *  directory, permission mode, skills, reasoning). Persisted separately from
 *  the session so the choices survive app restarts and are pre-filled next
 *  time. */
export const PET_CONFIG_KEY = "runjam.pet.config";

/** Shares the main window's recent-projects list so the pet's folder picker
 *  offers the same shortcuts (SessionView uses the same key). */
const RECENT_DIRS_KEY = "recent-project-dirs";

interface PetStoredSession {
  id: string;
  /**
   * Last time the user actually SENT something in this conversation.
   *
   * Deliberately "last activity", not "creation time": the session is only
   * retired once the user has been idle past the timeout, so changing the
   * agent/model/folder between messages does NOT throw the conversation away.
   */
  lastActiveAt: number;
  agentId: string;
  model: string;
  /** Fingerprint of the config currently applied to the RUNNING process. */
  configKey?: string;
}

/** Stable fingerprint of every knob that requires a *new* session to take
 *  effect (agent, model, directory, permission mode, skills, reasoning). */
export function petConfigKey(c: PetConfig): string {
  return JSON.stringify([
    c.agentId,
    c.modelId,
    c.directory,
    c.permissionMode,
    [...c.skills].sort(),
    c.noThinking,
  ]);
}

/** Everything the user can configure from the popup's toolbar — mirrors the
 *  main window's new-session page. */
export interface PetConfig {
  agentId: string;
  modelId: string;
  directory: string;
  permissionMode: string;
  skills: string[];
  noThinking: boolean;
}

export function defaultPetConfig(): PetConfig {
  return {
    agentId: "",
    modelId: "",
    directory: "",
    permissionMode: "",
    skills: [],
    noThinking: false,
  };
}

/** Coerce arbitrary parsed JSON into a well-formed `PetConfig`. Exported for
 *  testing: the stored blob is written by older versions / hand-edited, so a
 *  missing or mistyped field must never crash the popup. */
export function normalizePetConfig(raw: unknown): PetConfig {
  const base = defaultPetConfig();
  if (!raw || typeof raw !== "object") return base;
  const r = raw as Record<string, unknown>;
  return {
    agentId: typeof r.agentId === "string" ? r.agentId : "",
    modelId: typeof r.modelId === "string" ? r.modelId : "",
    directory: typeof r.directory === "string" ? r.directory : "",
    permissionMode: typeof r.permissionMode === "string" ? r.permissionMode : "",
    skills: Array.isArray(r.skills) ? r.skills.filter((s): s is string => typeof s === "string") : [],
    noThinking: r.noThinking === true,
  };
}

/**
 * The pet's transcript uses the main window's own `Message` shape (imported
 * above), so the shared `ChatMessages` component renders it unchanged. A
 * distinct alias here documents that this array is the pet's — not a second
 * type with its own fields to keep in sync.
 */
export type PetMessage = Message;

function loadStored(): PetStoredSession | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as PetStoredSession;
    if (!parsed || typeof parsed.id !== "string") return null;
    return parsed;
  } catch {
    return null;
  }
}

function saveStored(v: PetStoredSession | null) {
  try {
    if (v) localStorage.setItem(STORAGE_KEY, JSON.stringify(v));
    else localStorage.removeItem(STORAGE_KEY);
  } catch {
    // ignore storage errors
  }
}

function loadConfig(): PetConfig {
  try {
    const raw = localStorage.getItem(PET_CONFIG_KEY);
    if (!raw) return defaultPetConfig();
    return normalizePetConfig(JSON.parse(raw));
  } catch {
    return defaultPetConfig();
  }
}

function saveConfig(v: PetConfig) {
  try {
    localStorage.setItem(PET_CONFIG_KEY, JSON.stringify(v));
  } catch {
    // ignore storage errors
  }
}

function loadRecentDirs(): string[] {
  try {
    const raw = localStorage.getItem(RECENT_DIRS_KEY);
    const parsed = raw ? (JSON.parse(raw) as unknown) : [];
    return Array.isArray(parsed) ? parsed.filter((d): d is string => typeof d === "string") : [];
  } catch {
    return [];
  }
}

function rememberRecentDir(path: string) {
  try {
    const dirs = loadRecentDirs().filter((d) => d !== path);
    dirs.unshift(path);
    localStorage.setItem(RECENT_DIRS_KEY, JSON.stringify(dirs.slice(0, 8)));
  } catch {
    // ignore storage errors
  }
}

export function usePetChat() {
  const messages = ref<PetMessage[]>([]);
  const input = ref("");
  const sending = ref(false);
  /** Agent + model of the current conversation, shown in a small header. */
  const agentId = ref("");
  const agentName = ref("");
  const modelId = ref("");
  const modelName = ref("");
  const sessionId = ref("");
  const starting = ref(false);

  // ── Toolbar state (mirrors the main window's new-session page) ──────────
  /** Configuration the NEXT session will use. Editing anything here ends the
   *  current session and starts a fresh one — the popup has no per-session
   *  settings, so "change the model" can only mean "restart with this model". */
  const config = ref<PetConfig>(loadConfig());
  /** Agents that are installed AND enabled, i.e. selectable. */
  const availableAgents = ref<AgentInfo[]>([]);
  /** Models assigned to the currently selected agent. */
  const availableModels = ref<ModelEntry[]>([]);
  /** Built-in skill catalog. */
  const availableSkills = ref<SkillInfo[]>([]);
  /** Recent project directories (shared with the main window). */
  const recentDirs = ref<string[]>(loadRecentDirs());
  /** Files attached to the message being composed (parsed at send time). */
  const attachedFiles = ref<AttachedFile[]>([]);

  // Local llama.cpp server state, shared with the settings page via Pinia, so
  // the popup can tell whether a local model is actually running (only then is
  // it selectable — starting a server is done from Settings).
  const llamaStore = useLlamaStore();

  let unlisten: UnlistenFn | null = null;
  let lastActivityAt = 0;
  /** Accumulates the current streaming agent message's text. */
  let streamBuffer = "";
  /** Accumulates the current thinking block (separate from `streamBuffer`:
   *  a turn interleaves thought → text → tools, each into its own bubble). */
  let thinkingBuffer = "";
  /** When the current thinking block began, for the "Thought • 3s" label. */
  let thinkingStartTime = 0;
  /** Frozen thinking duration once the block ends (agent switched to text). */
  let thoughtDuration = "";
  /** When the whole turn began, for the message's total duration. */
  let turnStartTime = 0;
  /**
   * True when the session's agent process was (re)started and therefore has no
   * memory of the transcript so far.
   *
   * Set by `restartSessionProcess`. The next `send` then hands the recent turns
   * to the agent as context — without it, a process swapped in after a config
   * change or a Stop would answer with no idea what the conversation was about.
   */
  let processFresh = false;

  const hasMessages = computed(() => messages.value.length > 0);
  const canSend = computed(() => input.value.trim().length > 0 && !sending.value);

  /**
   * Independent snapshot of whether the composer must be locked.
   *
   * `sending` is set by `send()` and cleared on `finish`; but a backgrounded
   * popup can miss the finish (window hidden mid-turn), so the transcript's own
   * `isProcessing` flags are the source of truth for the UI. Derived rather than
   * duplicated so the two can never disagree.
   */
  const busy = computed(
    () => sending.value || messages.value.some((m) => m.role === "agent" && m.isProcessing === true),
  );

  /** Persist the toolbar choices so they pre-fill next time. */
  function persistConfig() {
    config.value = { ...config.value };
    saveConfig(config.value);
  }

  /** Load the agents the user can pick from. */
  async function loadAgents() {
    try {
      const all = await getAgentStatuses();
      availableAgents.value = all.filter((a) => a.installed && a.enabled);
    } catch {
      availableAgents.value = [];
    }
  }

  /**
   * Load the models the popup can pick from, mirroring the main window.
   *
   * Uses `getModels()` (ALL configured models) rather than `getAgentModels()`
   * (only those explicitly assigned to one agent): the agent-joins filter hid
   * every model the user had not pre-assigned, which in particular hid local
   * llama models — so the popup could not select a running local model at all.
   *
   * `agent` is accepted but no longer filters: the main window shows the same
   * full list regardless of the selected agent, and the popup must match it.
   */
  async function loadModelsFor(agent: string) {
    void agent; // kept for call-site compatibility; see the note above
    try {
      const list = await getModels();
      availableModels.value = withRunningLocalModel(list);
    } catch {
      availableModels.value = [];
    }
  }

  /**
   * Ensure the currently-running local model appears in the list.
   *
   * A llama server started outside the settings page (another instance, a
   * previous run) is not in `models.json`, so the popup would have no way to
   * pick the model that is actually serving. The main window auto-adds it the
   * same way; add it here only when the running model is known.
   */
  function withRunningLocalModel(list: ModelEntry[]): ModelEntry[] {
    const port = llamaStore.runningPort;
    const running = llamaStore.runningModel;
    if (!port || !running) return list;
    const file = baseName(running);
    if (list.some((m) => m.provider === "llama" && baseName(m.name) === file)) return list;
    return [
      ...list,
      {
        id: `llama-${running}-auto`,
        name: running,
        alias: file,
        provider: "llama",
        provider_name: "Llama",
        provider_icon: "llama",
        api_base: `http://localhost:${port}/v1`,
        api_key: "llama",
        protocol: "openai_chat",
        context_window: 0,
        support_reasoning: false,
        support_tools: true,
        tags: [],
        use_proxy: false,
        force_reasoning_none: false,
      } as ModelEntry,
    ];
  }

  /** Load the built-in skill catalog. */
  async function loadSkills() {
    try {
      availableSkills.value = await listSkills();
    } catch {
      availableSkills.value = [];
    }
  }

  /** Refresh everything the toolbar needs. */
  async function loadOptions() {
    // The popup is its OWN webview with its OWN Pinia instance: nothing has
    // populated the llama store here (App.vue does that for the main window), so
    // without this the local server would always look stopped and every local
    // model would be greyed out as unavailable. Refresh BEFORE loading models so
    // `withRunningLocalModel` sees the running model.
    await Promise.all([
      loadAgents(),
      loadSkills(),
      llamaStore.refresh().catch(() => {}),
    ]);
    await loadModelsFor(config.value.agentId);
    recentDirs.value = loadRecentDirs();
  }

  /** Surface an error in the transcript. The pet windows do not mount App.vue,
   *  so the global toast host is unavailable — errors belong inline here. */
  function pushError(text: string) {
    // No dedicated `error` flag on the shared `Message` type — the main window
    // reports failures as ordinary agent text too, so they render identically.
    messages.value.push({ role: "agent", content: text, isProcessing: false });
  }

  /** Resolve which agent to use: the toolbar's explicit pick when it is still
   *  usable, else the last-used agent, else the first usable one. */
  async function resolveDefaultAgent(): Promise<AgentInfo | null> {
    let agents: AgentInfo[] = availableAgents.value;
    if (agents.length === 0) {
      try {
        agents = (await getAgentStatuses()).filter((a) => a.installed && a.enabled);
      } catch {
        agents = [];
      }
    }
    if (agents.length === 0) return null;

    // Explicit pick wins.
    if (config.value.agentId) {
      const picked = agents.find((a) => a.id === config.value.agentId);
      if (picked) return picked;
    }

    try {
      const last = await getLastAgent();
      if (last) {
        const match = agents.find((a) => a.id === last);
        if (match) return match;
      }
    } catch {
      // fall through to first usable
    }
    return agents[0];
  }

  /** Resolve the model to use: the toolbar's explicit pick, else the agent's
   *  stored session-model default, else the first model assigned to it. */
  async function resolveDefaultModel(agent: string): Promise<ModelEntry | null> {
    // The popup's own selectable list FIRST: it is a superset of the persisted
    // models, because it also contains the auto-added entry for a local model
    // that is running but not recorded in `models.json` (started by another
    // instance, or renamed on disk). Looking only at `getModels()` would fail to
    // find that id and silently fall back to a DIFFERENT model — the user would
    // then chat with the wrong model believing they picked the local one.
    if (config.value.modelId) {
      const pickedHere = availableModels.value.find((m) => m.id === config.value.modelId);
      if (pickedHere) return pickedHere;
    }

    let models: ModelEntry[] = [];
    try {
      models = await getModels();
    } catch {
      models = [];
    }
    if (config.value.modelId) {
      const picked = models.find((m) => m.id === config.value.modelId);
      if (picked) return picked;
    }
    try {
      const saved = await getSessionModel(agent);
      if (saved) {
        const match = models.find((m) => m.id === saved);
        if (match) return match;
      }
    } catch {
      // fall through
    }
    try {
      const assigned = await getAgentModels(agent);
      if (assigned.length > 0) return assigned[0];
    } catch {
      // fall through
    }
    return null;
  }

  /** Resolve the permission mode for the pet session: the toolbar's pick, else
   *  the agent's saved preference, else the main window's per-agent default —
   *  see `resolvePetPermissionMode`. */
  async function resolvePermissionMode(agent: string): Promise<string> {
    if (config.value.permissionMode) return config.value.permissionMode;
    let saved = "";
    try {
      saved = await getAgentPermissionMode(agent);
    } catch {
      saved = "";
    }
    return resolvePetPermissionMode(saved);
  }

  /** Tear down the current session's agent process and forget it. */
  async function endSession() {
    const id = sessionId.value;
    if (id) {
      stopSession(id).catch(() => {});
    }
    if (unlisten) {
      try { unlisten(); } catch {}
      unlisten = null;
    }
    sessionId.value = "";
    streamBuffer = "";
    thinkingBuffer = "";
    thoughtDuration = "";
    thinkingStartTime = 0;
    turnStartTime = 0;
    processFresh = false;
    saveStored(null);
  }

  /** Subscribe to this session's streamed ACP events and fold them into the
   *  transcript.
   *
   * Mirrors the main window's `handleAcpEvent`: a turn is not just text — it
   * interleaves thinking blocks, tool calls and (sometimes) permission prompts,
   * and each phase gets its own bubble. The shared `ChatMessages` component
   * renders all of it, so the popup has to accumulate the same structure the
   * main window does, or those sections would simply never appear. */
  async function attachListener(id: string) {
    if (unlisten) {
      try { unlisten(); } catch {}
      unlisten = null;
    }
    unlisten = await listen<PetAcpPayload>(`acp:${id}`, (e) => {
      const p = e.payload;
      if (p.session_id !== id) return;
      lastActivityAt = Date.now();
      switch (p.type) {
        case "start": {
          // A new agent bubble. If the previous one already carried text, keep
          // it (Gemini emits several messages per turn) — just open a fresh
          // bubble for what follows.
          if (streamBuffer) persist("agent", streamBuffer);
          messages.value.push({ role: "agent", content: "", startTime: Date.now(), isProcessing: true });
          streamBuffer = "";
          thinkingBuffer = "";
          thoughtDuration = "";
          thinkingStartTime = 0;
          if (turnStartTime === 0) turnStartTime = Date.now();
          sending.value = true;
          break;
        }
        case "thinking": {
          // If the current bubble already has text or tools, the thought belongs
          // to a NEW bubble so the transcript reads linearly.
          const lastThink = lastAgentMsg(messages.value);
          if (lastThink && (lastThink.content || (lastThink.toolCalls?.length ?? 0) > 0)) {
            pushPhaseMessage();
            thinkingBuffer = "";
            thoughtDuration = "";
            thinkingStartTime = 0;
          }
          if (p.content) {
            if (thinkingStartTime === 0) thinkingStartTime = Date.now();
            // Snapshot vs delta: Gemini resends the whole thought each chunk,
            // Claude/Codex send deltas — same heuristic as text (see
            // `mergeStreamedText`).
            thinkingBuffer = mergeStreamedText(thinkingBuffer, p.content);
            ensureAgentMsg().thinking = thinkingBuffer;
          }
          if (p.status === "done" || p.duration) {
            thoughtDuration = thinkingStartTime > 0
              ? formatDuration(Date.now() - thinkingStartTime)
              : (p.duration || thoughtDuration);
            if (p.status === "done") thinkingStartTime = 0;
            ensureAgentMsg().thoughtDuration = thoughtDuration;
          }
          break;
        }
        case "text": {
          const content = p.content ?? "";
          // The backend smuggles the ACP session id through a `text` event with
          // this magic prefix (see acp_client.rs). It is protocol plumbing, not
          // model output — showing it would leak `__ACP_SESSION_ID__xxxx` into
          // the transcript. The main window filters it the same way.
          if (isInternalStreamChunk(content)) break;
          // Thinking → text transition freezes the thought's timer.
          if (thinkingStartTime > 0) {
            thoughtDuration = formatDuration(Date.now() - thinkingStartTime);
            thinkingStartTime = 0;
            ensureAgentMsg().thoughtDuration = thoughtDuration;
          }
          // Snapshot-aware merge: Claude/Gemini resend the whole message each
          // chunk, Codex sends deltas — see `mergeStreamedText`.
          streamBuffer = mergeStreamedText(streamBuffer, content);
          // Text resuming after a tool call belongs in a new bubble, otherwise
          // it mixes into the tool-only bubble.
          const lastText = lastAgentMsg(messages.value);
          if (!content && lastText && (lastText.toolCalls?.length ?? 0) > 0 && !lastText.content) {
            pushPhaseMessage();
          }
          ensureAgentMsg().content = streamBuffer;
          break;
        }
        case "tool_call": {
          // Text phase ended → next text belongs to a later bubble.
          streamBuffer = "";
          const lastToolCheck = lastAgentMsg(messages.value);
          if (lastToolCheck && lastToolCheck.content) pushPhaseMessage();

          const tc = ensureAgentMsg();
          if (!tc.toolCalls) tc.toolCalls = [];
          const toolName = p.tool_name || "";
          const isRunning = p.status === "running";
          if (isRunning) {
            // `tool_call_update` (running) refreshes the in-flight entry rather
            // than appending a duplicate.
            let found = false;
            for (let i = tc.toolCalls.length - 1; i >= 0; i--) {
              const existing = tc.toolCalls[i];
              if ((existing.status === "started" || existing.status === "running") && existing.toolName === toolName) {
                if (p.input) existing.input = p.input;
                if (p.title) existing.title = p.title;
                existing.status = "running";
                found = true;
                break;
              }
            }
            if (!found) {
              tc.toolCalls.push({ toolName, input: p.input || "", status: "running", startTime: p.start_time, title: p.title });
            }
          } else {
            tc.toolCalls.push({ toolName, input: p.input || "", status: p.status || "started", startTime: p.start_time, title: p.title });
          }
          break;
        }
        case "tool_result": {
          // Attach the output to the message that owns the tool call — which may
          // not be the last one if a thinking bubble was pushed meanwhile.
          const tr = lastToolMsg(messages.value) || ensureAgentMsg();
          const toolName = p.tool_name || "";
          if (tr.toolCalls && tr.toolCalls.length > 0) {
            let found = false;
            for (let i = tr.toolCalls.length - 1; i >= 0; i--) {
              const tc = tr.toolCalls[i];
              if (tc.status === "started" || tc.status === "running") {
                if (!toolName || tc.toolName === toolName) {
                  tc.output = p.output || "";
                  const out = (p.output || "").toLowerCase();
                  const failed =
                    (out.includes("error:") || out.includes("failed:")) &&
                    !out.includes("completed with no output");
                  tc.status = failed ? "failed" : "completed";
                  if (p.duration_ms !== undefined) tc.durationMs = p.duration_ms;
                  if (p.title) tc.title = p.title;
                  found = true;
                  break;
                }
              }
            }
            if (!found) {
              const last = tr.toolCalls[tr.toolCalls.length - 1];
              last.output = p.output || "";
              last.status = "completed";
              if (p.duration_ms !== undefined) last.durationMs = p.duration_ms;
            }
          }
          break;
        }
        case "permission_request": {
          // Each request gets its own bubble so simultaneous prompts (e.g.
          // WebSearch + WebFetch) don't overwrite one another. The shared
          // `ChatMessages` component renders the option buttons and answers via
          // `respondPermission` itself, so unlike the old popup we no longer
          // auto-deny here.
          messages.value.push({
            role: "agent",
            content: "",
            isProcessing: true,
            permission: {
              requestId: p.request_id || "",
              prompt: p.prompt || "",
              options: p.options || [],
              sessionId: id,
            },
          });
          break;
        }
        case "interaction": {
          const im = ensureAgentMsg();
          im.interaction = { prompt: p.prompt || "", options: p.options || [], sessionId: id };
          break;
        }
        case "finish": {
          // Close out the turn: no bubble is "processing" any more, and a
          // trailing empty one (opened by a `start` that produced nothing) is
          // dropped rather than left as a blank bubble.
          sending.value = false;
          thinkingStartTime = 0;
          for (const m of messages.value) {
            if (m.role === "agent") m.isProcessing = false;
          }
          const last = lastAgentMsg(messages.value);
          if (last && !last.content && !last.thinking && (last.toolCalls?.length ?? 0) === 0) {
            messages.value.pop();
          }
          const tail = lastAgentMsg(messages.value);
          if (tail) {
            if (turnStartTime > 0) tail.totalDurationMs = Date.now() - turnStartTime;
            const inTok = p.input_tokens || 0;
            const outTok = p.output_tokens || 0;
            if (p.input_tokens) tail.inputTokens = p.input_tokens;
            if (p.output_tokens) tail.outputTokens = p.output_tokens;
            if (p.cached_tokens) tail.cachedTokens = p.cached_tokens;
            if (inTok + outTok > 0) tail.totalTokens = inTok + outTok;
          }
          if (streamBuffer) persist("agent", streamBuffer);
          streamBuffer = "";
          thinkingBuffer = "";
          thoughtDuration = "";
          turnStartTime = 0;
          break;
        }
        case "error": {
          const msg = p.message || "出错了，请重试";
          sending.value = false;
          for (const m of messages.value) {
            if (m.role === "agent") m.isProcessing = false;
          }
          // Drop a trailing empty placeholder so the error reads as its own line.
          const le = lastAgentMsg(messages.value);
          if (le && !le.content && !le.thinking) messages.value.pop();
          messages.value.push({ role: "agent", content: msg, isProcessing: false });
          persist("agent", msg);
          streamBuffer = "";
          thinkingBuffer = "";
          turnStartTime = 0;
          break;
        }
        default:
          break;
      }
    });
  }

  /** The last agent message, or null when the transcript has none yet. */
  function lastAgentMsg(msgs: PetMessage[]): PetMessage | null {
    for (let i = msgs.length - 1; i >= 0; i--) if (msgs[i].role === "agent") return msgs[i];
    return null;
  }

  /** The last agent message that carries tool calls (falls back to the last
   *  agent message) — tool results belong there, not on a later bubble. */
  function lastToolMsg(msgs: PetMessage[]): PetMessage | null {
    for (let i = msgs.length - 1; i >= 0; i--) {
      const m = msgs[i];
      if (m.role === "agent" && m.toolCalls && m.toolCalls.length > 0) return m;
    }
    return lastAgentMsg(msgs);
  }

  /** Get (or create) the agent bubble that streamed content attaches to. */
  function ensureAgentMsg(): PetMessage {
    let m = lastAgentMsg(messages.value);
    if (!m) {
      m = { role: "agent", content: "", startTime: Date.now(), isProcessing: true };
      messages.value.push(m);
    }
    return m;
  }

  /** Close the current bubble and open a fresh one for the next phase, so a
   *  turn's thinking / tool / text phases read as separate entries. */
  function pushPhaseMessage(): PetMessage {
    for (let i = messages.value.length - 1; i >= 0; i--) {
      if (messages.value[i].role === "agent") {
        messages.value[i].isProcessing = false;
        break;
      }
    }
    const msg: PetMessage = { role: "agent", content: "", startTime: Date.now(), isProcessing: true };
    messages.value.push(msg);
    return msg;
  }

  /** Compact duration label for the thinking block ("Thought • 3s"). */
  function formatDuration(ms: number): string {
    const s = Math.floor(ms / 1000);
    if (s < 60) return `${s}s`;
    return `${Math.floor(s / 60)}m ${s % 60}s`;
  }

  /** Persist a turn so the pet conversation can be reopened from the sidebar
   *  later (the pet owns a real session — "可回溯历史"). */
  function persist(role: "user" | "agent", content: string) {
    const id = sessionId.value;
    if (!id || !content.trim()) return;
    saveConversationMessage(id, role, content).catch(() => {});
  }

  /** Ensure a usable session: reuse the stored one while the user has been
   *  active within the idle window, restarting its process if the toolbar
   *  selection changed — only start a NEW session once the conversation went
   *  stale. */
  async function ensureSession(): Promise<boolean> {
    const stored = loadStored();
    if (stored) {
      const fresh = isPetSessionFresh(stored.lastActiveAt, Date.now());
      if (fresh) {
        let alive = false;
        try {
          alive = await sessionAlive(stored.id);
        } catch {
          alive = false;
        }
        if (alive) {
          sessionId.value = stored.id;
          agentId.value = stored.agentId;
          modelId.value = stored.model;
          // The main window live-propagates permission-mode changes to every
          // running session of an agent. Without this, a change made for the
          // main window would silently re-gate the pet's tools on a prompt the
          // user did not ask for here. Re-assert the mode this popup was told to
          // use — the user's explicit pick when they made one, otherwise the
          // unattended default (`resolvePetPermissionMode`). The popup CAN render
          // permission prompts now, so an explicit `ask_approval` is honoured
          // rather than overridden.
          setSessionPermissionMode(stored.id, config.value.permissionMode || "approve_for_me").catch(() => {});
          await attachListener(stored.id);
          // Same conversation, but the user changed agent/model/folder/skills:
          // swap the process in place. The transcript stays; only the agent
          // behind it is replaced.
          if (stored.configKey !== petConfigKey(config.value)) {
            await restartSessionProcess(stored.id);
          }
          return true;
        }
        // The conversation is still within its idle window but its process is
        // gone — the user pressed Stop, or the process died. Keep the SAME
        // session (id, transcript, sidebar entry) and just bring a process back:
        // a brand-new session here is what littered the sidebar with duplicate
        // "🐾 宠物会话" rows, which the user asked us to stop doing.
        if (await restartSessionProcess(stored.id)) return true;
      }
      // Stale (past the idle window) or unrestartable: retire it and start over.
      // A stale-but-still-alive session would otherwise leak its agent process
      // for the rest of the app's lifetime — `endSession` stops the agent, not
      // just our state.
      await endSession();
    }
    return await createSession();
  }

  /**
   * Restart the SAME session's agent process so a changed toolbar selection
   * takes effect.
   *
   * Why a restart instead of a new session: the agent reads its model, working
   * directory and skills from disk/env at *process start*, so those knobs only
   * apply to a fresh process. Starting a NEW session for every toolbar tweak is
   * what used to litter the sidebar with "🐾 宠物会话" entries — the user wants a
   * single conversation, so we keep the id and swap the process underneath it.
   */
  async function restartSessionProcess(id: string): Promise<boolean> {
    try {
      const agent = await resolveDefaultAgent();
      if (!agent) {
        pushError("没有可用的 agent，请先在设置里安装并启用一个");
        return false;
      }
      const model = await resolveDefaultModel(agent.id);
      const permissionMode = await resolvePermissionMode(agent.id);
      const directory = config.value.directory || undefined;
      const skills = config.value.skills.length > 0 ? [...config.value.skills] : undefined;

      // Reasoning is a process-wide proxy setting; apply it before the restart
      // so the very first answer already honours it.
      setReasoningDisabled(config.value.noThinking).catch(() => {});

      // Drop the old process first — otherwise the manager keeps the stale
      // client and the new configuration never actually reaches the agent.
      await stopSession(id).catch(() => {});

      await apiStartSession(
        agent.id,
        agent.display_name,
        directory,
        id,
        model?.id || undefined,
        undefined,
        permissionMode,
        skills,
      );

      agentId.value = agent.id;
      agentName.value = agent.display_name;
      modelId.value = model?.id || "";
      modelName.value = model?.name || model?.id || "";
      if (!config.value.agentId) config.value.agentId = agent.id;
      if (!config.value.modelId && model) config.value.modelId = model.id;
      if (!config.value.permissionMode) config.value.permissionMode = permissionMode;
      persistConfig();
      if (model) updateSessionModel(id, model.id).catch(() => {});
      setLastAgent(agent.id).catch(() => {});

      // Record the config now baked into the running process, so the next
      // `ensureSession` does not restart us over and over for the same choice.
      const stored = loadStored();
      if (stored && stored.id === id) {
        saveStored({ ...stored, configKey: petConfigKey(config.value), agentId: agent.id, model: model?.id || "" });
      }
      // The new process starts with no memory of the transcript; the next send
      // hands it the recent turns as context.
      processFresh = true;
      await attachListener(id);
      return true;
    } catch (err) {
      console.error("[pet] failed to restart session process:", err);
      pushError(`切换配置失败：${err}`);
      return false;
    }
  }

  /** Create the pet's dedicated session using the toolbar configuration. */
  async function createSession(): Promise<boolean> {
    if (starting.value) return false;
    starting.value = true;
    try {
      const agent = await resolveDefaultAgent();
      if (!agent) {
        pushError("没有可用的 agent，请先在设置里安装并启用一个");
        return false;
      }
      const model = await resolveDefaultModel(agent.id);
      const permissionMode = await resolvePermissionMode(agent.id);
      const directory = config.value.directory || undefined;
      const skills = config.value.skills.length > 0 ? [...config.value.skills] : undefined;

      // Reasoning is a process-wide proxy setting (not per-session), matching
      // the main window's toggle. Apply it before the agent starts so the very
      // first response already honours it.
      setReasoningDisabled(config.value.noThinking).catch(() => {});

      const id = `pet-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
      const title = `🐾 宠物会话`;

      // Persist up-front so the session shows in the sidebar and survives a
      // crash before the agent process starts.
      await saveSession(
        id,
        agent.id,
        agent.display_name,
        title,
        directory ?? "",
        "running",
        null,
        0,
        0,
        "",
      ).catch(() => {});
      if (model) {
        updateSessionModel(id, model.id).catch(() => {});
      }

      const info = await apiStartSession(
        agent.id,
        agent.display_name,
        directory,
        id,
        model?.id || undefined,
        // Ask the backend to launch without prompt gating: the popup cannot
        // render permission dialogs (see resolvePetPermissionMode).
        undefined,
        permissionMode,
        skills,
      );
      sessionId.value = info.id;
      agentId.value = agent.id;
      agentName.value = agent.display_name;
      modelId.value = model?.id || "";
      modelName.value = model?.name || model?.id || "";
      // Remember the resolved choices so the toolbar shows what is actually
      // running (and re-opening the popup pre-fills the same values).
      if (!config.value.agentId) config.value.agentId = agent.id;
      if (!config.value.modelId && model) config.value.modelId = model.id;
      if (!config.value.permissionMode) config.value.permissionMode = permissionMode;
      persistConfig();
      // Mirror the main window's "last agent" so switching here also switches
      // the new-session page's default next time.
      setLastAgent(agent.id).catch(() => {});
      const now = Date.now();
      lastActivityAt = now;
      saveStored({
        id: info.id,
        lastActiveAt: now,
        agentId: agent.id,
        model: model?.id || "",
        configKey: petConfigKey(config.value),
      });
      await attachListener(info.id);
      // Let the main window refresh its sidebar so the new session appears.
      emit(PET_SESSION_CHANGED_EVENT, { id: info.id }).catch(() => {});
      return true;
    } catch (err) {
      console.error("[pet] failed to start session:", err);
      pushError(`宠物会话启动失败：${err}`);
      return false;
    } finally {
      starting.value = false;
    }
  }

  /**
   * Stop the running answer.
   *
   * Mirrors the main window's `handleStop`: ACP has no "interrupt" request, so
   * the only way to halt a turn is to terminate the agent process. The LIVE
   * flags are cleared first (the process dies without emitting a `finish`, so
   * nothing else would clear them and the transcript would keep spinning), and
   * the session is marked stopped so the sidebar reflects reality.
   *
   * The transcript is deliberately KEPT: the conversation continues, and the
   * next send restarts a process under the same id with the recent history
   * injected (see `send`).
   */
  async function stop() {
    const id = sessionId.value;
    sending.value = false;
    // Settle every live marker. Without this, an `isProcessing` message (or a
    // tool call left at "running") keeps `busy` true forever — the composer
    // would stay locked and the transcript would keep animating.
    settleInterruptedMessages(messages.value);
    // A stopped turn has no more streamed text coming.
    streamBuffer = "";
    thinkingBuffer = "";
    thinkingStartTime = 0;
    turnStartTime = 0;
    if (id) {
      await stopSession(id).catch((err) => {
        console.error("[pet] failed to stop session:", err);
      });
      // The process is gone; the next send must start a new one and give it the
      // recent transcript, or the agent would answer with no memory of it.
      processFresh = true;
      // Let the main window refresh its sidebar (the row should read "stopped").
      emit(PET_SESSION_CHANGED_EVENT, { id }).catch(() => {});
    }
  }

  /** Send the current input as a question. */
  async function send() {
    const text = input.value.trim();
    if (!text || sending.value) return;

    const ok = await ensureSession();
    if (!ok || !sessionId.value) return;

    // Attachments are parsed into the outgoing text but NOT shown in the
    // transcript (only their file names are) — same split as the main window.
    const files = attachedFiles.value;
    let sendText = text;
    let userDisplay = text;
    if (files.length > 0) {
      const { text: withAttachments, failures } = await buildAttachmentPayload(files, text);
      sendText = withAttachments;
      userDisplay = text + attachmentDisplaySuffix(files);
      if (failures.length > 0) {
        // The pet has no toast host (it never mounts App.vue), so failures are
        // reported inline before the turn starts.
        pushError(`附件解析失败：${failures.join("; ")}`);
      }
      attachedFiles.value = [];
    }

    messages.value.push({ role: "user", content: userDisplay });
    persist("user", userDisplay);
    input.value = "";
    sending.value = true;
    streamBuffer = "";
    thinkingBuffer = "";
    thoughtDuration = "";
    thinkingStartTime = 0;
    turnStartTime = 0;
    lastActivityAt = Date.now();
    // Refresh the stored activity stamp: this is what `ensureSession` measures
    // the idle window against, so the conversation stays "alive" as long as the
    // user keeps asking things.
    const stored = loadStored();
    if (stored && stored.id === sessionId.value) {
      saveStored({ ...stored, lastActiveAt: lastActivityAt });
    }

    // A freshly (re)started process has no memory of this conversation, so hand
    // it the last couple of exchanges — the same technique the main window uses
    // after a restart. The just-pushed user message is excluded (the backend
    // appends the new prompt itself); only the PRIOR turns become context.
    let history: string[] | undefined;
    if (processFresh) {
      const prior = messages.value
        .slice(0, -1)
        .filter((m) => m.content)
        .slice(-4)
        .map((m) => `${m.role}: ${m.content}`);
      if (prior.length > 0) history = prior;
      processFresh = false;
    }

    try {
      await sendInput(sessionId.value, sendText, history);
      // Bump the session's recency so it sorts to the top of the sidebar.
      touchSession(sessionId.value).catch(() => {});
    } catch (err) {
      messages.value.push({ role: "agent", content: `发送失败：${err}`, isProcessing: false });
      sending.value = false;
    }
  }

  /** Start a brand-new conversation, ending the current one. */
  async function newConversation() {
    await endSession();
    messages.value = [];
    input.value = "";
    attachedFiles.value = [];
    sending.value = false;
    await createSession();
  }

  /**
   * Hand the current conversation over to the main window: bring it to front and
   * switch it to this session, where the wider layout (file tree, terminal) is
   * available. The transcript itself is the same on both surfaces.
   */
  async function openInMainWindow() {
    // Make sure there is something to open on the other side.
    if (!sessionId.value) {
      await ensureSession();
    }
    await openMainWindowAt({ id: sessionId.value });
  }

  /**
   * Ask the main window to come forward at a given route.
   *
   * The popup is a separate webview with no router, so it cannot navigate
   * itself; the main window owns routing. Used to send a user to Settings (e.g.
   * a local model that is not running must be started there).
   */
  async function openMainWindowAt(payload: { id?: string | null; route?: string }) {
    // The main window cannot be focused from a background webview, so ask the
    // backend to do it; the event carries where to go.
    await invoke("focus_main_window").catch((err) => {
      console.error("[pet] failed to focus main window:", err);
    });
    emit(PET_OPEN_IN_MAIN_EVENT, payload).catch(() => {});
    await close_pet_chat_silently();
  }

  /** Send the user to the local-models settings page (start a server there). */
  async function openLocalModelsSettings() {
    await openMainWindowAt({ id: sessionId.value || null, route: "/settings/models" });
  }

  /** Hide the popup without waiting on the backend — used when handing off. */
  async function close_pet_chat_silently() {
    try {
      await invoke("close_pet_chat");
    } catch {
      // ignore
    }
  }

  /**
   * Apply a toolbar change and restart the conversation with it.
   *
   * There is no per-session settings surface in the popup, so changing the
   * agent/model/folder etc. can only mean "the next session uses this". We end
   * the running session (older transcript stays readable until the next send)
   * and pre-create a session so the header immediately reflects the new
   * choice — mirroring the main window, where picking a model arms the next
   * send. `createSession` is idempotent-safe: `send()` also calls
   * `ensureSession`, which reuses the session we just made.
   */
  async function applyConfig(patch: Partial<PetConfig>) {
    config.value = { ...config.value, ...patch };
    persistConfig();

    // Persist the pick as the agent's default, mirroring the main window: the
    // new-session page reads `session_model_<agent>` / `agent_permission_<id>`,
    // so a choice made in the popup also becomes the default there.
    const patchedAgent = config.value.agentId;
    if (patch.modelId !== undefined && patchedAgent && config.value.modelId) {
      setSessionModel(patchedAgent, config.value.modelId).catch(() => {});
    }
    if (patch.permissionMode !== undefined && patchedAgent && config.value.permissionMode) {
      setAgentPermissionMode(patchedAgent, config.value.permissionMode).catch(() => {});
    }

    // Changing the agent invalidates the model list AND the permission-mode
    // default (both are per-agent). Drop a model that no longer belongs to the
    // new agent and let the mode re-resolve from that agent's own default.
    if (patch.agentId !== undefined && patch.agentId !== agentId.value) {
      [config.value.modelId, config.value.permissionMode] = ["", ""];
      persistConfig();
      await loadModelsFor(config.value.agentId);
      // Pre-fill a model so the toolbar shows what will actually run instead of
      // an empty selector. Skip models that cannot be used right now: the list
      // now includes local models, and pre-selecting one whose server is not
      // running would arm a model that fails on send.
      const usable = availableModels.value.find((m) => !isUnavailableLocalModel(m));
      if (usable) {
        config.value.modelId = usable.id;
        persistConfig();
      }
    }

    // Do NOT start a new conversation here. A toolbar change only affects the
    // agent process that runs the NEXT question; `ensureSession` (called from
    // `send`) notices the changed `configKey` and swaps the process in place,
    // keeping the current transcript. Starting a fresh session per click is what
    // filled the sidebar with duplicate conversations.
    //
    // A running answer is left alone so its streamed text is not cut off; the
    // new setting applies from the next question on.
  }

  /** Toggle a skill in the toolbar's skill set. */
  async function toggleSkill(name: string) {
    const next = new Set(config.value.skills);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    await applyConfig({ skills: [...next] });
  }

  /** Toggle the reasoning switch (process-wide proxy setting). */
  async function toggleReasoning() {
    await applyConfig({ noThinking: !config.value.noThinking });
  }

  /** Pick a project directory (empty string = no project). */
  async function setDirectory(path: string) {
    if (path) rememberRecentDir(path);
    recentDirs.value = loadRecentDirs();
    await applyConfig({ directory: path });
  }

  /** Re-read the shared recent-projects list (called when the menu opens). */
  function refreshRecentDirs() {
    recentDirs.value = loadRecentDirs();
  }

  /**
   * Whether a model is a LOCAL model (llama.cpp) that is NOT currently serving.
   *
   * Such a model cannot be used from the popup: starting a llama server is a
   * settings-page action. The popup shows these greyed out and routes the user to
   * Settings instead of silently failing on send. Non-local models are always
   * selectable, and a local model that IS running is selectable too.
   */
  function isUnavailableLocalModel(model: ModelEntry): boolean {
    return isLocalModelUnavailable(model, llamaStore.runningPort, llamaStore.runningModel);
  }

  /** Re-read the local-server state so the model menu reflects reality. */
  async function refreshLocalServer() {
    try {
      await llamaStore.refresh();
    } catch {
      // leave the last known state; the menu just shows what it had
    }
    // The running model may have appeared/disappeared since the list was built.
    availableModels.value = withRunningLocalModel(availableModels.value.filter(
      // Drop a previously auto-added entry whose server has since stopped.
      (m) => !(m.provider === "llama" && m.id.startsWith("llama-") && m.id.endsWith("-auto")),
    ));
  }

  /** Load a short greeting referencing the active agent/model. */
  function greet() {
    if (agentName.value || modelName.value) return;
    messages.value.push({
      role: "agent",
      content: "你好，我是 RunJam 助手。有什么可以帮你的？",
      isProcessing: false,
    });
  }

  /** Open the native picker and stage the chosen files for the next send. */
  async function attachFiles() {
    try {
      const existing = new Set(attachedFiles.value.map((f) => f.path));
      const added = await pickAttachedFiles(existing);
      if (added.length > 0) attachedFiles.value = [...attachedFiles.value, ...added];
    } catch (err) {
      console.error("[pet] attach files failed:", err);
    }
  }

  /** Drop a staged attachment before it is sent. */
  function removeAttachedFile(path: string) {
    attachedFiles.value = attachedFiles.value.filter((f) => f.path !== path);
  }

  /** Stage a file the caller already knows about (e.g. a dropped path). */
  function addAttachedFiles(files: AttachedFile[]) {
    const existing = new Set(attachedFiles.value.map((f) => f.path));
    const fresh = files.filter((f) => !existing.has(f.path));
    if (fresh.length > 0) attachedFiles.value = [...attachedFiles.value, ...fresh];
  }

  function dispose() {
    if (unlisten) {
      try { unlisten(); } catch {}
      unlisten = null;
    }
  }

  return {
    messages,
    input,
    sending,
    busy,
    starting,
    sessionId,
    agentId,
    agentName,
    modelId,
    modelName,
    hasMessages,
    canSend,
    send,
    stop,
    newConversation,
    openInMainWindow,
    openLocalModelsSettings,
    ensureSession,
    greet,
    dispose,
    // Attachments (files staged for the next message).
    attachedFiles,
    attachFiles,
    removeAttachedFile,
    addAttachedFiles,
    // Toolbar (agent / model / folder / permission / skills / reasoning).
    config,
    availableAgents,
    availableModels,
    availableSkills,
    recentDirs,
    loadOptions,
    applyConfig,
    toggleSkill,
    toggleReasoning,
    setDirectory,
    refreshRecentDirs,
    // Local model availability (greyed out when its server is not running).
    isUnavailableLocalModel,
    refreshLocalServer,
    idleTimeoutMs: PET_IDLE_TIMEOUT_MS,
    getLastActivityAt: () => lastActivityAt,
  };
}

/** Shape of the broadcast `acp:<id>` payload (subset of SessionView's `AcpPayload`). */
interface PetAcpPayload {
  session_id: string;
  type:
    | "start"
    | "thinking"
    | "text"
    | "tool_call"
    | "tool_result"
    | "interaction"
    | "permission_request"
    | "finish"
    | "error";
  content?: string;
  status?: string;
  duration?: string;
  message?: string;
  stop_reason?: string;
  request_id?: string;
  /** Human-readable label of the tool/action a `permission_request` is about. */
  prompt?: string;
  options?: { key: string; label: string; is_default: boolean }[];
  tool_name?: string;
  input?: string;
  output?: string;
  start_time?: number;
  duration_ms?: number;
  title?: string;
  input_tokens?: number;
  output_tokens?: number;
  cached_tokens?: number;
}