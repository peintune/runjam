import { defineStore } from "pinia";
import { ref, computed, watch } from "vue";
import { startSession as tauriStartSession, stopSession as tauriStopSession } from "../api/sessions";
import { useMessageStore } from "./useMessageStore";
import { saveSession, getSessions, updateSessionTitle, updateSessionModel, deleteSession, archiveSession as apiArchiveSession, unarchiveSession as apiUnarchiveSession, deleteArchivedSessions, touchSession as apiTouchSession, type SessionRecord } from "../api/search";

export interface Directory {
  id: string;
  path: string;
}

export interface Session {
  id: string;
  cli: string;
  cliDisplayName: string;
  title: string;
  directoryId: string | null;
  /** Actual working-directory path of this session (the cwd the agent runs in).
   *  Captured from the backend's start_session response so default-directory
   *  sessions (~/.runjam/session/{id}) are tracked correctly. */
  directory: string | null;
  model: string | null;
  status: "running" | "idle" | "waiting" | "stopped" | "error";
  pid: number | null;
  pinned: boolean;
  archived: boolean;
  createdAt: string;
  lastActiveAt: string;
  unread: boolean;
  acpSessionId: string;
  /** True when session just finished generating and hasn't been opened yet */
  newlyCompleted: boolean;
  /** True when agent process is freshly started (after restart), needs history context */
  freshAgentProcess: boolean;
  /** Total character count of the session's messages, persisted in the DB.
   *  Lets the sidebar show every session's context size on load without
   *  fetching that session's messages into the frontend. */
  contextChars: number;
}

function generateId(): string {
  return Date.now().toString(36) + Math.random().toString(36).slice(2, 8);
}

function recordToSession(record: SessionRecord): Session {
  return {
    id: record.id,
    cli: record.cli,
    cliDisplayName: record.cli_display_name,
    title: record.title || record.cli_display_name,
    model: record.model || null,
    directoryId: record.directory || null,
    directory: record.directory || null,
    // After a page reload, no session is truly running/waiting — backend process is gone
    status: (record.status === 'running' || record.status === 'waiting' || record.status === 'idle') ? 'stopped' : record.status as Session["status"],
    pid: record.pid || null,
    pinned: record.pinned === 1,
    archived: record.archived === 1,
    createdAt: record.created_at,
    // Prefer the backend's last_active_at (persisted across reloads) and
    // fall back to created_at for sessions that predate the column.
    lastActiveAt: record.last_active_at || record.created_at,
    unread: false,
    acpSessionId: record.acp_session_id || "",
    newlyCompleted: false,
    freshAgentProcess: false,
    contextChars: record.context_chars || 0,
  };
}

function timeAgo(iso: string): string {
  const diff = Date.now() - new Date(iso).getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return '刚刚';
  if (mins < 60) return `${mins}分钟前`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}小时前`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}天前`;
  return new Date(iso).toLocaleDateString();
}

// Persist the last active session across webview reloads. Directory-based
// sessions can write into the project tree (skills deployment, agent files),
// which triggers a vite full-page reload in dev; the backend session survives
// but all in-memory store state is wiped, leaving the app on the new-session
// page. Restoring activeSessionId re-opens the real session after the reload.
const ACTIVE_SESSION_KEY = "runjam-active-session-id";

function loadPersistedActiveSession(): string | null {
  try {
    return localStorage.getItem(ACTIVE_SESSION_KEY);
  } catch {
    return null;
  }
}

export const useWorkspaceStore = defineStore("workspace", () => {
  const directories = ref<Directory[]>([]);
  const sessions = ref<Session[]>([]);
  const activeSessionId = ref<string | null>(loadPersistedActiveSession());
  /** Character length of the input draft in the active session. Kept here
   *  so the sidebar (SessionItem) can show the same context-size number the
   *  session view computes — the draft counts toward the context total. */
  const activeDraftChars = ref(0);

  const activeSession = computed(() =>
    sessions.value.find((s) => s.id === activeSessionId.value) ?? null,
  );

  // Keep localStorage in sync so a reload (dev HMR full-reload, crash, etc.)
  // doesn't drop the user back on the new-session page.
  watch(activeSessionId, (id) => {
    try {
      if (id) {
        localStorage.setItem(ACTIVE_SESSION_KEY, id);
      } else {
        localStorage.removeItem(ACTIVE_SESSION_KEY);
      }
    } catch {}
  });

  async function loadSessions() {
    try {
      const records = await getSessions();
      sessions.value = records.map(record => {
        const session = recordToSession(record);
        if (session.directoryId) {
          if (session.directoryId.startsWith('/')) {
            session.directoryId = ensureDirectory(session.directoryId);
          } else {
            console.warn(`Skipping invalid directory path: ${session.directoryId} for session ${session.id}`);
            session.directoryId = null;
          }
        }
        return session;
      });

      // Re-attach to the persisted active session only after a webview RELOAD
      // (dev HMR full-reload, crash, manual refresh). On a fresh app launch
      // ("navigate") keep the existing behavior: start on the new-session page.
      let navType = "";
      try {
        navType = (performance.getEntriesByType("navigation")[0] as PerformanceNavigationTiming | undefined)?.type || "";
      } catch {}
      const persisted = loadPersistedActiveSession();
      if (navType === "reload" && persisted && sessions.value.some((s) => s.id === persisted)) {
        console.log("[STORE] Restoring active session after reload:", persisted);
        activeSessionId.value = persisted;
      } else if (persisted && !sessions.value.some((s) => s.id === persisted)) {
        console.warn("[STORE] Persisted active session not found in DB, dropping:", persisted);
        try { localStorage.removeItem(ACTIVE_SESSION_KEY); } catch {}
      }
    } catch (err) {
      console.error("Failed to load sessions:", err);
    }
  }

  function ensureDirectory(dirPath: string): string | null {
    // Don't track .runjam internal directories in sidebar navigation
    if (dirPath.includes('.runjam')) return null;
    // Skip empty or invalid paths
    if (!dirPath || dirPath.trim() === '') return null;
    const existing = directories.value.find((d) => d.path === dirPath);
    if (existing) return existing.id;
    const id = generateId();
    directories.value.push({ id, path: dirPath });
    return id;
  }

  async function createSession(cli: Session["cli"], cliDisplayName: string, dirPath?: string, title?: string, model?: string, mode?: string, permissionMode?: string, skills?: string[]) {
    const directoryId = dirPath ? ensureDirectory(dirPath) : null;
    const sessionId = generateId();
    const now = new Date().toISOString();
    const session: Session = {
      id: sessionId, cli, cliDisplayName,
      title: title || cliDisplayName, directoryId, directory: dirPath || null, model: model || null, pinned: false, archived: false,
      status: "running", pid: null, createdAt: now, lastActiveAt: now, unread: false, acpSessionId: "", newlyCompleted: false, freshAgentProcess: true, contextChars: 0,
    };
    sessions.value.push(session);
    activeSessionId.value = session.id;

    try {
      await saveSession(sessionId, cli, cliDisplayName, session.title, session.directory || "", "running", null, 0, 0, "");
    } catch (err) {
      console.error("saveSession failed:", err);
    }
    // Persist the session's model so it survives restarts (the sessions table
    // stores it per-session, separate from the agent's global model config).
    if (model) {
      updateSessionModel(sessionId, model).catch(() => {});
    }

    try {
      const info = await tauriStartSession(cli, cliDisplayName, dirPath, sessionId, model, mode, permissionMode, skills);
      console.log("[STORE] createSession info.directory =", info.directory, "for session", sessionId);
      const s = sessions.value.find(s => s.id === sessionId);
      if (s) {
        s.status = info.status as Session["status"];
        s.pid = info.pid;
        s.createdAt = info.created_at;
        s.lastActiveAt = new Date().toISOString();
        // Capture the real working directory returned by the backend — for
        // default-directory sessions this is ~/.runjam/session/{id}, which the
        // frontend never knew before. Used by per-session skill management.
        if (info.directory) {
          s.directory = info.directory;
        }
        if (info.status === 'stopped') {
          s.newlyCompleted = true;
        }
        saveSession(sessionId, cli, cliDisplayName, s.title, s.directory || "", s.status, s.pid, s.pinned ? 1 : 0, s.archived ? 1 : 0, s.acpSessionId).catch(() => {});
      }
    } catch (err) {
      console.error("Failed to start session:", err);
      // Remove the placeholder session so the UI doesn't hang in "running" state.
      sessions.value = sessions.value.filter(s => s.id !== sessionId);
      if (activeSessionId.value === sessionId) {
        activeSessionId.value = null;
      }
      throw err; // Re-throw so callers (handleSend) can handle the failure.
    }
  }

  async function setSessionTitle(id: string, title: string) { 
    const s = sessions.value.find(s => s.id === id); 
    if (s) {
      s.title = title;
      updateSessionTitle(id, title).catch(err => console.error("Failed to update session title:", err));
    }
  }

  async function togglePin(id: string) {
    const s = sessions.value.find(s => s.id === id);
    if (s) {
      s.pinned = !s.pinned;
      saveSession(s.id, s.cli, s.cliDisplayName, s.title, s.directory || "", s.status, s.pid, s.pinned ? 1 : 0, s.archived ? 1 : 0, s.acpSessionId).catch(() => {});
    }
  }

  function selectSession(id: string) {
    const prevId = activeSessionId.value;
    activeSessionId.value = id;
    // Mutate reactive proxy objects in-place
    const s = sessions.value.find(s => s.id === id);
    if (s) { s.unread = false; s.newlyCompleted = false; }
    if (prevId && prevId !== id) {
      const prev = sessions.value.find(s => s.id === prevId);
      if (prev && prev.status === 'running') prev.unread = true;
    }
    // NOTE: intentionally do NOT reassign `sessions.value = [...sessions.value]`
    // here. That shallow-copy invalidates every computed/watch that depends on
    // the array (Sidebar's grouped/activeSessions/archivedSessions), forcing a
    // full re-render of the whole session list on every click even though the
    // changes above are already reactive via activeSessionId + in-place property
    // mutation. With many sessions that made clicking a conversation stutter.
  }

  function touchSession(id: string) {
    // Optimistic local update so the sidebar re-orders immediately, then
    // persist to the backend so the order survives a page reload.
    const s = sessions.value.find(s => s.id === id);
    if (s) s.lastActiveAt = new Date().toISOString();
    apiTouchSession(id).catch(err => console.error("Failed to touch session:", err));
  }

  function archiveSession(id: string) {
    const s = sessions.value.find(s => s.id === id);
    if (s) { s.archived = true; apiArchiveSession(id).catch(()=>{}); }
  }

  function unarchiveSession(id: string) {
    const s = sessions.value.find(s => s.id === id);
    if (s) { s.archived = false; apiUnarchiveSession(id).catch(()=>{}); }
  }

  async function batchDeleteAllArchived() {
    await deleteArchivedSessions();
    sessions.value = sessions.value.filter(s => !s.archived);
  }

  async function batchDelete(ids: string[]) {
    // Capture sessions before removal so we can stop their agent processes in
    // the background. The list must update immediately regardless of how long
    // the backend stop takes.
    const toDelete = sessions.value.filter(s => ids.includes(s.id));
    sessions.value = sessions.value.filter(s => !ids.includes(s.id));
    if (activeSessionId.value && ids.includes(activeSessionId.value)) {
      activeSessionId.value = sessions.value[0]?.id ?? null;
    }
    for (const id of ids) {
      useMessageStore().removeSession(id);
      const s = toDelete.find(s => s.id === id);
      if (s && (s.status === "running" || s.status === "idle")) {
        tauriStopSession(id).catch((err) => console.error("Failed to stop session:", err));
      }
    }
    for (const id of ids) {
      try {
        await deleteSession(id);
      } catch (err) {
        console.error("Failed to delete session:", err);
      }
    }
  }

  function batchPin(ids: string[]) {
    sessions.value.forEach(s => {
      if (ids.includes(s.id)) { s.pinned = true; }
    });
    ids.forEach(id => {
      const s = sessions.value.find(s => s.id === id);
      if (s) saveSession(s.id, s.cli, s.cliDisplayName, s.title, s.directory || "", s.status, s.pid, 1, s.archived ? 1 : 0, s.acpSessionId).catch(() => {});
    });
  }

  async function stopSession(id: string) {
    // Mark stopped immediately so late ACP events from the dying process are
    // dropped (SessionView.handleAcpEvent), then terminate the process.
    const s0 = sessions.value.find((s) => s.id === id);
    if (s0) s0.status = "stopped";
    try { await tauriStopSession(id); } catch (err) { console.error(err); }
    const s = sessions.value.find((s) => s.id === id);
    if (s) {
      s.status = "stopped";
      s.newlyCompleted = true;
      saveSession(id, s.cli, s.cliDisplayName, s.title, s.directory || "", "stopped", s.pid, s.pinned ? 1 : 0, s.archived ? 1 : 0, s.acpSessionId).catch(() => {});
    }
  }

  async function removeSession(id: string) {
    // Capture the session before removal so we know whether its agent process
    // still needs stopping. Removal MUST come first and must never wait on the
    // backend stop — stop_session can block for a long time (or hang) when an
    // agent process ignores SIGTERM, and awaiting it here made the session
    // appear to "not delete" (nothing happened in the list).
    const delSession = sessions.value.find((s) => s.id === id);
    sessions.value = sessions.value.filter((s) => s.id !== id);
    if (activeSessionId.value === id) {
      activeSessionId.value = sessions.value[0]?.id ?? null;
    }
    useMessageStore().removeSession(id);
    // Best-effort background cleanup: terminate any still-running agent
    // process for this session — otherwise the process keeps running and
    // emitting events after the session is deleted.
    if (delSession && (delSession.status === "running" || delSession.status === "idle")) {
      tauriStopSession(id).catch((err) => console.error("Failed to stop session:", err));
    }
    try {
      await deleteSession(id);
    } catch (err) {
      console.error("Failed to delete session:", err);
    }
  }

  return {
    directories,
    sessions,
    activeSessionId,
    activeSession,
    activeDraftChars,
    loadSessions,
    createSession,
    selectSession,
    touchSession,
    stopSession,
    removeSession,
    setSessionTitle,
    togglePin,
    archiveSession,
    unarchiveSession,
    batchDeleteAllArchived,
    batchDelete,
    batchPin,
    timeAgo,
  };
});
