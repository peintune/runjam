/**
 * Desktop-pet conversation engine.
 *
 * The pet popup is a *lightweight* Q&A surface, but it deliberately reuses the
 * real session machinery rather than talking to the model directly: questions
 * go through a genuine agent session (ACP), so the pet inherits the same
 * models, the same permission handling and the same streaming events as the
 * main window — "复用新建会话的能力".
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
import {
  startSession as apiStartSession,
  sendInput,
  stopSession,
  sessionAlive,
  setSessionPermissionMode,
  respondPermission,
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
 * `__ACP_SESSION_ID__` (see `acp_client.rs`). The popup is a plain-text surface
 * with no filtering, so without this it renders the raw marker — the
 * `__ACP_SESSION_ID__xxxx` the user sees on every new conversation.
 * Exported so the rule is unit-testable.
 */
export function isInternalStreamChunk(content: string): boolean {
  return content.startsWith(ACP_SESSION_ID_MARKER);
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

export interface PetMessage {
  role: "user" | "agent";
  content: string;
  /** True while this agent message is still streaming. */
  streaming?: boolean;
  error?: boolean;
}

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

  let unlisten: UnlistenFn | null = null;
  let lastActivityAt = 0;
  /** Accumulates the current streaming agent message. */
  let streamBuffer = "";

  const hasMessages = computed(() => messages.value.length > 0);
  const canSend = computed(() => input.value.trim().length > 0 && !sending.value);

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

  /** Load the models assigned to `agent`. */
  async function loadModelsFor(agent: string) {
    if (!agent) {
      availableModels.value = [];
      return;
    }
    try {
      availableModels.value = await getAgentModels(agent);
    } catch {
      availableModels.value = [];
    }
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
    await Promise.all([loadAgents(), loadSkills()]);
    await loadModelsFor(config.value.agentId);
    recentDirs.value = loadRecentDirs();
  }

  /** Surface an error in the transcript. The pet windows do not mount App.vue,
   *  so the global toast host is unavailable — errors belong inline here. */
  function pushError(text: string) {
    messages.value.push({ role: "agent", content: text, error: true });
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
    saveStored(null);
  }

  /** Subscribe to this session's streamed ACP events and fold them into the
   *  message list. Only text/error/finish matter for a Q&A popup — tool calls
   *  and thinking are ignored to keep the UI minimal. */
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
        case "permission_request": {
          // Safety net. The session is started with `approve_for_me`, so the
          // backend normally auto-approves before this ever reaches us. If the
          // mode is unavailable for an agent (or the user picked read_only),
          // the request would otherwise hang the turn: the popup has no
          // permission UI, so no `finish` would ever arrive and the composer
          // would stay disabled forever. Deny it and surface a readable note.
          const requestId = p.request_id || "";
          if (requestId) {
            respondPermission(id, requestId, "deny").catch(() => {});
          }
          const what = p.prompt ? `「${p.prompt}」` : "一个操作";
          pushError(`需要授权才能执行${what}，已自动拒绝。可改用主窗口完成该操作。`);
          finalizeStreaming();
          sending.value = false;
          break;
        }
        case "start":
          streamBuffer = "";
          break;
        case "text": {
          // The backend smuggles the ACP session id through a `text` event with
          // this magic prefix (see acp_client.rs). It is protocol plumbing, not
          // model output — showing it would leak `__ACP_SESSION_ID__xxxx` into
          // the transcript. The main window filters it the same way.
          const content = p.content ?? "";
          if (isInternalStreamChunk(content)) break;
          // Snapshot-aware merge: Claude/Gemini resend the whole message each
          // chunk, Codex sends deltas — see `mergeStreamedText`.
          streamBuffer = mergeStreamedText(streamBuffer, content);
          upsertStreaming(streamBuffer);
          break;
        }
        case "finish": {
          // Commit whatever we have; a finish with no text means the agent
          // produced nothing (e.g. it answered via a tool only).
          finalizeStreaming();
          sending.value = false;
          break;
        }
        case "error": {
          finalizeStreaming();
          const msg = p.message || "出错了，请重试";
          messages.value.push({
            role: "agent",
            content: msg,
            error: true,
          });
          persist("agent", msg);
          sending.value = false;
          break;
        }
        default:
          // thinking / tool_call / tool_result / interaction / permission_request
          // are intentionally not surfaced in the minimal popup.
          break;
      }
    });
  }

  /** Replace the trailing streaming message with `text`, creating it if needed. */
  function upsertStreaming(text: string) {
    const last = messages.value[messages.value.length - 1];
    if (last && last.role === "agent" && last.streaming) {
      last.content = text;
    } else {
      messages.value.push({ role: "agent", content: text, streaming: true });
    }
  }

  function finalizeStreaming() {
    const last = messages.value[messages.value.length - 1];
    if (last && last.role === "agent" && last.streaming) {
      last.streaming = false;
      // Drop an empty bubble produced by a start/finish pair with no text.
      if (!last.content.trim()) {
        messages.value.pop();
        return;
      }
      persist("agent", last.content);
    }
    streamBuffer = "";
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
          // running session of an agent. If the user switched this agent to
          // `ask_approval` mid-flight, the reused session would start gating
          // tool calls on a prompt this popup cannot render — force the mode
          // back to unattended for the pet's own session.
          setSessionPermissionMode(stored.id, "approve_for_me").catch(() => {});
          await attachListener(stored.id);
          // Same conversation, but the user changed agent/model/folder/skills:
          // swap the process in place. The transcript stays; only the agent
          // behind it is replaced.
          if (stored.configKey !== petConfigKey(config.value)) {
            await restartSessionProcess(stored.id);
          }
          return true;
        }
      }
      // Stale or dead: retire it and start over. A stale-but-still-alive
      // session would otherwise leak its agent process for the rest of the
      // app's lifetime — `endSession` stops the agent, not just our state.
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

  /** Send the current input as a question. */
  async function send() {
    const text = input.value.trim();
    if (!text || sending.value) return;

    const ok = await ensureSession();
    if (!ok || !sessionId.value) return;

    messages.value.push({ role: "user", content: text });
    persist("user", text);
    input.value = "";
    sending.value = true;
    streamBuffer = "";
    lastActivityAt = Date.now();
    // Refresh the stored activity stamp: this is what `ensureSession` measures
    // the idle window against, so the conversation stays "alive" as long as the
    // user keeps asking things.
    const stored = loadStored();
    if (stored && stored.id === sessionId.value) {
      saveStored({ ...stored, lastActiveAt: lastActivityAt });
    }

    try {
      await sendInput(sessionId.value, text);
      // Bump the session's recency so it sorts to the top of the sidebar.
      touchSession(sessionId.value).catch(() => {});
    } catch (err) {
      messages.value.push({ role: "agent", content: `发送失败：${err}`, error: true });
      sending.value = false;
    }
  }

  /** Start a brand-new conversation, ending the current one. */
  async function newConversation() {
    await endSession();
    messages.value = [];
    input.value = "";
    sending.value = false;
    await createSession();
  }

  /**
   * Hand the current conversation over to the main window: bring it to front and
   * switch it to this session, where the full transcript (tool calls, thinking,
   * permission dialogs) is available.
   *
   * The popup deliberately renders only plain text, so anything that needs the
   * full session UI — reviewing what the agent actually did, answering a
   * permission prompt — happens in the main window instead.
   */
  async function openInMainWindow() {
    // Make sure there is something to open on the other side.
    if (!sessionId.value) {
      await ensureSession();
    }
    // The main window cannot be focused from a background webview, so ask the
    // backend to do it; the event carries which session to show.
    await invoke("focus_main_window").catch((err) => {
      console.error("[pet] failed to focus main window:", err);
    });
    emit(PET_OPEN_IN_MAIN_EVENT, { id: sessionId.value }).catch(() => {});
    await close_pet_chat_silently();
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
      // Pre-fill the new agent's own default model so the toolbar shows what
      // will actually run instead of an empty selector.
      if (availableModels.value.length > 0) {
        config.value.modelId = availableModels.value[0].id;
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

  /** Load a short greeting referencing the active agent/model. */
  function greet() {
    if (agentName.value || modelName.value) return;
    messages.value.push({
      role: "agent",
      content: "你好，我是 RunJam 助手。有什么可以帮你的？",
    });
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
    starting,
    sessionId,
    agentId,
    agentName,
    modelId,
    modelName,
    hasMessages,
    canSend,
    send,
    newConversation,
    openInMainWindow,
    ensureSession,
    greet,
    dispose,
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
    idleTimeoutMs: PET_IDLE_TIMEOUT_MS,
    getLastActivityAt: () => lastActivityAt,
  };
}

/** Minimal shape of the broadcast `acp:<id>` payload (subset of SessionView's). */
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
  message?: string;
  stop_reason?: string;
  request_id?: string;
  /** Human-readable label of the tool/action a `permission_request` is about. */
  prompt?: string;
}