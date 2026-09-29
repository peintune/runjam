import { reactive } from "vue";

export interface SessionLayout {
  openFiles: string[];
  activeFileIndex: number;
  showTerminal: boolean;
  /** 文件树是否展开。与 showTerminal 相互独立：只开终端时文件树保持关闭，
   *  终端独占整个面板宽度。 */
  showFileTree: boolean;
  fileTreeWidth: number;
  terminalHeight: number;
  sidebarWidth: number;
  chatWidth: number;
}

export interface LayoutSnapshot {
  hasTerminal: boolean;
  hasOpenFiles: boolean;
  openFileCount: number;
}

const defaults: SessionLayout = {
  openFiles: [],
  activeFileIndex: -1,
  showTerminal: false,
  showFileTree: false,
  fileTreeWidth: 260,
  terminalHeight: 130,
  sidebarWidth: 270,
  chatWidth: 420,
};

/** Layout bucket for the *new-session page* — the one place with neither a
 *  directory nor a session. Everything else gets a real bucket. */
export const DEFAULT_LAYOUT_KEY = "__default__";

/** Prefix for the per-session fallback bucket. Keeps those keys from being
 *  mistaken for a directory path (which always contains `/`). */
export const SESSION_LAYOUT_PREFIX = "session:";

/** Layout bucket key for a session.
 *
 *  Keyed by the session's working-directory *path*, NOT by the store's
 *  `directoryId`: that id is generated fresh on every launch (the `directories`
 *  array is never persisted), so keying by it silently lost every per-directory
 *  layout — and the sidebar/task-board badges — on restart.
 *
 *  A project directory path is the natural bucket: sessions running in that
 *  folder share their open files/terminal, and the path is stable across
 *  restarts. Default-directory sessions get `~/.runjam/session/{id}`, already
 *  unique per session, so they are isolated without extra handling.
 *
 *  `sessionId` is the fallback for legacy rows whose directory is empty — each
 *  gets its own bucket, where they used to share a single one (which made a
 *  terminal opened in one session appear in every other one). */
export function layoutKeyFor(
  directoryPath: string | null | undefined,
  sessionId: string | null | undefined,
): string | null {
  if (directoryPath) return directoryPath;
  if (sessionId) return SESSION_LAYOUT_PREFIX + sessionId;
  return null;
}

function storageKey(key: string) {
  return `dir-layout-${key}`;
}

/** Reactive snapshots of layout buckets — SessionItem/TaskBoardCard depend on
 *  this. Using a Map instead of a plain object for reliable dynamic-key
 *  reactivity in Vue 3. */
const layoutSnapshots = reactive(new Map<string, LayoutSnapshot | null>());

/** Seed the snapshot map from localStorage on cold start.
 *
 *  The map otherwise only holds buckets touched during the current run — i.e.
 *  those the user actively switched to. Since the sidebar/task-board badges read
 *  snapshots for *every* visible session, a restart would show no badges until
 *  each session had been opened once. Reading them once up front keeps the
 *  badges correct immediately. */
function hydrateSnapshots() {
  const PREFIX = "dir-layout-";
  try {
    for (let i = 0; i < localStorage.length; i++) {
      const rawKey = localStorage.key(i);
      if (!rawKey?.startsWith(PREFIX)) continue;
      const key = rawKey.slice(PREFIX.length);
      if (layoutSnapshots.has(key)) continue;
      const raw = localStorage.getItem(rawKey);
      if (!raw) continue;
      const parsed = JSON.parse(raw) as Partial<SessionLayout>;
      const openFiles = Array.isArray(parsed.openFiles) ? parsed.openFiles : [];
      layoutSnapshots.set(key, {
        hasTerminal: !!parsed.showTerminal,
        hasOpenFiles: openFiles.length > 0,
        openFileCount: openFiles.length,
      });
    }
  } catch {
    // localStorage unavailable — badges fall back to nothing.
  }
}
hydrateSnapshots();

/** Update the reactive snapshot for a bucket (also writes to localStorage) */
function updateSnapshot(key: string, layout: SessionLayout) {
  layoutSnapshots.set(key, {
    hasTerminal: layout.showTerminal,
    hasOpenFiles: layout.openFiles.length > 0,
    openFileCount: layout.openFiles.length,
  });
  localStorage.setItem(storageKey(key), JSON.stringify(layout));
}

// Shared module-level reactive state
const state = reactive({
  /** Currently loaded bucket key (already defaulted — never `null` once loaded) */
  key: null as string | null,
  layout: { ...defaults } as SessionLayout,
});

function saveLayout() {
  if (state.key) {
    updateSnapshot(state.key, state.layout);
  }
}

function loadLayout(key: string) {
  try {
    const raw = localStorage.getItem(storageKey(key));
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<SessionLayout>;
      Object.assign(state.layout, { ...defaults, ...parsed });
    } else {
      Object.assign(state.layout, { ...defaults });
    }
  } catch {
    Object.assign(state.layout, { ...defaults });
  }
  state.key = key;
  // Ensure reactive snapshot exists for this bucket
  if (!layoutSnapshots.has(key)) {
    layoutSnapshots.set(key, {
      hasTerminal: state.layout.showTerminal,
      hasOpenFiles: state.layout.openFiles.length > 0,
      openFileCount: state.layout.openFiles.length,
    });
  }
}

export function useSessionLayout() {
  return {
    layout: state.layout,

    /** Switch to another layout bucket. Same bucket = keep current state.
     *
     *  `key` comes from `layoutKeyFor` (`null` → the new-session page bucket).
     *  Each bucket is fully independent, so a terminal opened in one session or
     *  directory never leaks into another — the previous behaviour mapped every
     *  project-less session to one shared bucket, and resetting on every switch
     *  is what silently closed an open terminal. */
    switchDirectory(key: string | null) {
      const k = key ?? DEFAULT_LAYOUT_KEY;
      // Same bucket: do nothing, keep current reactive state
      if (k === state.key) return;

      // Save current layout before switching away
      if (state.key) {
        saveLayout();
      }
      loadLayout(k);
    },

    saveLayout,

    /** Peek at a bucket's layout reactively (used for sidebar/task-board badges) */
    peekLayout(key: string): LayoutSnapshot | null {
      return layoutSnapshots.get(key) ?? null;
    },
  };
}
