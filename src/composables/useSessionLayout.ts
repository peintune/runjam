import { reactive } from "vue";

export interface SessionLayout {
  openFiles: string[];
  activeFileIndex: number;
  showTerminal: boolean;
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
  fileTreeWidth: 260,
  terminalHeight: 260,
  sidebarWidth: 270,
  chatWidth: 420,
};

function storageKey(directoryId: string) {
  return `dir-layout-${directoryId}`;
}

/** Reactive snapshots of directory layouts — SessionItem depends on this.
 *  Using Map instead of plain object for reliable dynamic-key reactivity in Vue 3. */
const layoutSnapshots = reactive(new Map<string, LayoutSnapshot | null>());

/** Update the reactive snapshot for a directory (also writes to localStorage) */
function updateSnapshot(directoryId: string, layout: SessionLayout) {
  layoutSnapshots.set(directoryId, {
    hasTerminal: layout.showTerminal,
    hasOpenFiles: layout.openFiles.length > 0,
    openFileCount: layout.openFiles.length,
  });
  localStorage.setItem(storageKey(directoryId), JSON.stringify(layout));
}

// Shared module-level reactive state
const state = reactive({
  directoryId: null as string | null,
  layout: { ...defaults } as SessionLayout,
});

function saveLayout() {
  if (state.directoryId) {
    updateSnapshot(state.directoryId, state.layout);
  }
}

function loadLayout(directoryId: string) {
  try {
    const raw = localStorage.getItem(storageKey(directoryId));
    if (raw) {
      Object.assign(state.layout, { ...defaults, ...JSON.parse(raw) });
    } else {
      Object.assign(state.layout, { ...defaults });
    }
  } catch {
    Object.assign(state.layout, { ...defaults });
  }
  state.directoryId = directoryId;
  // Ensure reactive snapshot exists for this directory
  if (!layoutSnapshots.has(directoryId)) {
    layoutSnapshots.set(directoryId, {
      hasTerminal: state.layout.showTerminal,
      hasOpenFiles: state.layout.openFiles.length > 0,
      openFileCount: state.layout.openFiles.length,
    });
  }
}

export function useSessionLayout() {
  return {
    layout: state.layout,
    directoryId: state.directoryId,

    /** Switch layout to another directory. Same directory = keep current layout. */
    switchDirectory(directoryId: string | null) {
      // Same directory: do nothing, keep current reactive state
      if (directoryId && directoryId === state.directoryId) return;

      // Save current layout before switching away
      if (state.directoryId) {
        saveLayout();
      }
      if (directoryId) {
        loadLayout(directoryId);
      } else {
        Object.assign(state.layout, { ...defaults });
        state.directoryId = null;
      }
    },

    saveLayout,

    /** Peek at a directory's layout reactively */
    peekLayout(directoryId: string): LayoutSnapshot | null {
      return layoutSnapshots.get(directoryId) ?? null;
    },
  };
}
