import { reactive, ref } from "vue";

export interface SessionLayout {
  openFiles: string[];
  activeFileIndex: number;
  /** Viewer kind for files opened FROM A SESSION (a path link in message text or
   *  thinking), keyed by path.
   *
   *  The file tree keeps its own, simpler behaviour (images/PDF preview,
   *  everything else in the editor), so a file opened there is deliberately NOT
   *  listed here. Persisted with the rest of the layout: `openFiles` survives a
   *  restart, and without this a restored `.docx` tab would fall back to the
   *  editor and render its bytes as text. */
  fileViewers: Record<string, string>;
  showTerminal: boolean;
  /** 文件树是否展开。与 showTerminal 相互独立：只开终端时文件树保持关闭，
   *  终端独占整个面板宽度。 */
  showFileTree: boolean;
  fileTreeWidth: number;
  terminalHeight: number;
  sidebarWidth: number;
  /** 右侧工作区（文件树 + 编辑器）的宽度。对话区占剩余空间，所以只需要
   *  记住工作区这一侧。 */
  workspaceWidth: number;
}

export interface LayoutSnapshot {
  hasTerminal: boolean;
  hasOpenFiles: boolean;
  openFileCount: number;
}

const defaults: SessionLayout = {
  openFiles: [],
  activeFileIndex: -1,
  fileViewers: {},
  showTerminal: false,
  showFileTree: false,
  fileTreeWidth: 260,
  terminalHeight: 130,
  sidebarWidth: 270,
  workspaceWidth: 420,
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

/**
 * Bumped every time a file is opened from a session.
 *
 * This exists because the ACTION of opening a file is not always visible in the
 * state: re-opening a file that is already open, while the panel is hidden,
 * leaves `openFiles` and `activeFileIndex` exactly as they were. A watcher on
 * those fields never fires, so the panel stays hidden and the click looks like it
 * did nothing — which is exactly the bug this replaces.
 *
 * A counter has no such blind spot: every open bumps it, so the listener can
 * react to "the user asked to open a file" rather than trying to infer it from a
 * state difference.
 *
 * Deliberately NOT persisted — it is a transient signal, not layout, and
 * restoring a stale value at startup would pop the panel open unbidden.
 */
const openFileSignal = ref(0);

function saveLayout() {
  if (state.key) {
    updateSnapshot(state.key, state.layout);
  }
}

function loadLayout(key: string) {
  // A fresh copy of the defaults every time: `defaults` is a module constant, so
  // its `openFiles` array and `fileViewers` object would otherwise be SHARED by
  // every bucket — an edit in one session would appear in all of them.
  const fresh = (): SessionLayout => ({
    ...defaults,
    openFiles: [...defaults.openFiles],
    fileViewers: { ...defaults.fileViewers },
  });
  try {
    const raw = localStorage.getItem(storageKey(key));
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<SessionLayout>;
      Object.assign(state.layout, fresh(), parsed);
    } else {
      Object.assign(state.layout, fresh());
    }
  } catch {
    Object.assign(state.layout, fresh());
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

/**
 * Whether the right-hand workspace panel (file tree + editor) should be shown.
 *
 * The rule is the FILE TREE toggle alone — that button is the master switch for
 * the whole workspace, not just the tree. Opening a file expands the tree (see
 * the `openFileSignal` watcher in `WorkspaceLayout`), so the two stay in step;
 * keeping the editor on screen after the tree is closed leaves an orphaned panel
 * eating the chat's width, which is a bug we actually shipped.
 *
 * Deliberately independent of whether files are open (`openFiles`), and of the
 * terminal: the terminal is a full-width strip along the bottom and must not
 * reveal the editor, and a persisted `openFiles` from earlier in the session must
 * not either.
 *
 * Extracted as a pure function so the rule has a test: a component-local
 * `computed` would be unreachable without a DOM test harness.
 */
export function isWorkspaceAreaVisible(o: {
  isBoard: boolean;
  hasDirectory: boolean;
  showFileTree: boolean;
}): boolean {
  return !o.isBoard && o.hasDirectory && o.showFileTree;
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

    /** Transient signal, bumped on every "open a file from a session" action.
     *  See `openFileSignal` for why a state diff is not enough. */
    openFileSignal,

    /** Announce that a file was just opened, so the workspace panel shows itself.
     *  Does not touch `openFiles` — the caller has already done that. */
    signalFileOpened() {
      openFileSignal.value++;
    },

    /** Peek at a bucket's layout reactively (used for sidebar/task-board badges) */
    peekLayout(key: string): LayoutSnapshot | null {
      return layoutSnapshots.get(key) ?? null;
    },
  };
}
