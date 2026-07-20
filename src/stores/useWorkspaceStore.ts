import { defineStore } from "pinia";
import { ref, computed } from "vue";
import { startSession as tauriStartSession, stopSession as tauriStopSession } from "../api/sessions";
import { useMessageStore } from "./useMessageStore";
import { saveSession, getSessions, updateSessionTitle, deleteSession, archiveSession as apiArchiveSession, unarchiveSession as apiUnarchiveSession, deleteArchivedSessions, type SessionRecord } from "../api/search";

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
  model: string | null;
  status: "running" | "waiting" | "stopped" | "error";
  pid: number | null;
  pinned: boolean;
  archived: boolean;
  createdAt: string;
  lastActiveAt: string;
  unread: boolean;
  /** True when session just finished generating and hasn't been opened yet */
  newlyCompleted: boolean;
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
    // After a page reload, no session is truly running/waiting — backend process is gone
    status: (record.status === 'running' || record.status === 'waiting') ? 'stopped' : record.status as Session["status"],
    pid: record.pid || null,
    pinned: record.pinned === 1,
    archived: record.archived === 1,
    createdAt: record.created_at,
    lastActiveAt: record.created_at,
    unread: false,
    newlyCompleted: false,
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

export const useWorkspaceStore = defineStore("workspace", () => {
  const directories = ref<Directory[]>([]);
  const sessions = ref<Session[]>([]);
  const activeSessionId = ref<string | null>(null);

  const activeSession = computed(() =>
    sessions.value.find((s) => s.id === activeSessionId.value) ?? null,
  );

  async function loadSessions() {
    try {
      const records = await getSessions();
      sessions.value = records.map(record => {
        const session = recordToSession(record);
        if (session.directoryId) {
          session.directoryId = ensureDirectory(session.directoryId);
        }
        return session;
      });

      
    } catch (err) {
      console.error("Failed to load sessions:", err);
    }
  }

  function ensureDirectory(dirPath: string): string | null {
    // Don't track .runjam internal directories in sidebar navigation
    if (dirPath.includes('.runjam')) return null;
    const existing = directories.value.find((d) => d.path === dirPath);
    if (existing) return existing.id;
    const id = generateId();
    directories.value.push({ id, path: dirPath });
    return id;
  }

  async function createSession(cli: Session["cli"], cliDisplayName: string, dirPath?: string, title?: string, model?: string, mode?: string, permissionMode?: string) {
    const directoryId = dirPath ? ensureDirectory(dirPath) : null;
    const sessionId = generateId();
    const now = new Date().toISOString();
    const session: Session = {
      id: sessionId, cli, cliDisplayName,
      title: title || cliDisplayName, directoryId, model: model || null, pinned: false, archived: false,
      status: "running", pid: null, createdAt: now, lastActiveAt: now, unread: false, newlyCompleted: false,
    };
    sessions.value.push(session);
    activeSessionId.value = session.id;

    try {
      await saveSession(sessionId, cli, cliDisplayName, session.title, dirPath || "", "running", null, 0, 0);
    } catch (err) {
      console.error("saveSession failed:", err);
    }

    tauriStartSession(cli, cliDisplayName, dirPath, sessionId, model, mode, permissionMode).then(info => {
      const s = sessions.value.find(s => s.id === sessionId);
      if (s) {
        s.status = info.status as Session["status"];
        s.pid = info.pid;
        s.createdAt = info.created_at;
        s.lastActiveAt = new Date().toISOString();
        if (info.status === 'stopped') {
          s.newlyCompleted = true;
        }
        saveSession(sessionId, cli, cliDisplayName, s.title, dirPath || "", s.status, s.pid, s.pinned ? 1 : 0, s.archived ? 1 : 0).catch(() => {});
      }
    }).catch(err => {
      console.error("Failed to start session:", err);
    });
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
      saveSession(s.id, s.cli, s.cliDisplayName, s.title, s.directoryId || "", s.status, s.pid, s.pinned ? 1 : 0, s.archived ? 1 : 0).catch(() => {});
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
    // New array reference triggers computed chain re-evaluation
    sessions.value = [...sessions.value];
  }

  function touchSession(id: string) {
    const s = sessions.value.find(s => s.id === id);
    if (s) s.lastActiveAt = new Date().toISOString();
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

  function batchDelete(ids: string[]) {
    sessions.value = sessions.value.filter(s => !ids.includes(s.id));
    if (activeSessionId.value && ids.includes(activeSessionId.value)) {
      activeSessionId.value = sessions.value[0]?.id ?? null;
    }
    ids.forEach(id => {
      useMessageStore().removeSession(id);
      deleteSession(id).catch(() => {});
    });
  }

  function batchPin(ids: string[]) {
    sessions.value.forEach(s => {
      if (ids.includes(s.id)) { s.pinned = true; }
    });
    ids.forEach(id => {
      const s = sessions.value.find(s => s.id === id);
      if (s) saveSession(s.id, s.cli, s.cliDisplayName, s.title, s.directoryId || "", s.status, s.pid, 1, s.archived ? 1 : 0).catch(() => {});
    });
  }

  async function stopSession(id: string) {
    try { await tauriStopSession(id); } catch (err) { console.error(err); }
    const s = sessions.value.find((s) => s.id === id);
    if (s) {
      s.status = "stopped";
      s.newlyCompleted = true;
      saveSession(id, s.cli, s.cliDisplayName, s.title, s.directoryId || "", "stopped", s.pid, s.pinned ? 1 : 0, s.archived ? 1 : 0).catch(() => {});
    }
  }

  async function removeSession(id: string) {
    sessions.value = sessions.value.filter((s) => s.id !== id);
    if (activeSessionId.value === id) {
      activeSessionId.value = sessions.value[0]?.id ?? null;
    }
    useMessageStore().removeSession(id);
    deleteSession(id).catch(err => console.error("Failed to delete session:", err));
  }

  return {
    directories,
    sessions,
    activeSessionId,
    activeSession,
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
