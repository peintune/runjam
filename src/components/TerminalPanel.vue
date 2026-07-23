<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount, nextTick, watch } from "vue";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { X, Trash2, Plus, TerminalIcon } from "lucide-vue-next";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

const props = defineProps<{
  cwd?: string;
}>();

const emit = defineEmits<{
  (e: "close"): void;
}>();

// ---- Tab state ----
interface TabState {
  id: number;
  title: string;
  cwd: string;
  term: Terminal | null;
  fitAddon: FitAddon | null;
  unlisten: UnlistenFn | null;
  resizeObserver: ResizeObserver | null;
}

const tabs = ref<TabState[]>([]);
const activeTabIndex = ref(-1);
let tabCounter = 0;

// Container refs — keyed by terminal id
const containerEls = ref<Record<number, HTMLElement | null>>({});
const tabsScrollEl = ref<HTMLElement | null>(null);

// ═══════════════════════════════════════════════════
// Module-level terminal persistence (per directory)
// ═══════════════════════════════════════════════════

interface SavedTab {
  id: number;
  title: string;
  cwd: string;
}

interface SavedDirectoryState {
  tabs: SavedTab[];
  activeIndex: number;
  counter: number;
}

const directoryStates = new Map<string, SavedDirectoryState>();

/** Save current tabs metadata for a directory WITHOUT killing backend processes */
function saveDirectoryState(cwd: string) {
  if (!cwd || tabs.value.length === 0) return;
  directoryStates.set(cwd, {
    tabs: tabs.value.map((t) => ({ id: t.id, title: t.title, cwd: t.cwd })),
    activeIndex: activeTabIndex.value,
    counter: tabCounter,
  });
}

/** Restore tabs for a directory. Returns null if no saved state. */
async function restoreDirectoryState(cwd: string): Promise<{
  restoredTabs: TabState[];
  restoredIndex: number;
  restoredCounter: number;
} | null> {
  const saved = directoryStates.get(cwd);
  if (!saved || saved.tabs.length === 0) return null;

  const restoredTabs: TabState[] = [];
  for (const st of saved.tabs) {
    const tab: TabState = {
      id: st.id,
      title: st.title,
      cwd: st.cwd,
      term: null,
      fitAddon: null,
      unlisten: null,
      resizeObserver: null,
    };

    // Re-listen to backend terminal output
    tab.unlisten = await listen<number[] | string>(
      `terminal-data-${tab.id}`,
      (event) => {
        if (!tab.term) return;
        const payload = event.payload;
        if (typeof payload === "string") {
          tab.term.write(payload);
        } else if (Array.isArray(payload)) {
          tab.term.write(new Uint8Array(payload));
        }
      }
    );

    restoredTabs.push(tab);
  }

  return {
    restoredTabs,
    restoredIndex: saved.activeIndex,
    restoredCounter: saved.counter,
  };
}

/** Dispose xterm DOM resources for a tab (NOT the backend process, NOT the event listener) */
function disposeTabDOM(tab: TabState) {
  tab.resizeObserver?.disconnect();
  tab.resizeObserver = null;
  tab.term?.dispose();
  tab.term = null;
  tab.fitAddon = null;
}

/** Full dispose (including event listener) — used when tab is explicitly closed */
function disposeTabFull(tab: TabState) {
  tab.unlisten?.();
  tab.unlisten = null;
  disposeTabDOM(tab);
}

/** Save current tabs, then dispose all DOM + listeners */
function teardownCurrentTabs() {
  for (const tab of tabs.value) {
    disposeTabFull(tab);
  }
}

// ═══════════════════════════════════════════════════
// Terminal operations
// ═══════════════════════════════════════════════════

function handleClear() {
  activeTab()?.term?.clear();
}

function activeTab(): TabState | undefined {
  return tabs.value[activeTabIndex.value];
}

async function createTab(): Promise<TabState> {
  const workDir = props.cwd || null;
  const termId = await invoke<number>("spawn_terminal", { cwd: workDir });
  tabCounter++;

  const tab: TabState = {
    id: termId,
    title: `sh-${tabCounter}`,
    cwd: workDir || "",
    term: null,
    fitAddon: null,
    unlisten: null,
    resizeObserver: null,
  };

  tab.unlisten = await listen<number[] | string>(
    `terminal-data-${termId}`,
    (event) => {
      if (!tab.term) return;
      const payload = event.payload;
      if (typeof payload === "string") {
        tab.term.write(payload);
      } else if (Array.isArray(payload)) {
        tab.term.write(new Uint8Array(payload));
      }
    }
  );

  return tab;
}

async function addTab() {
  const tab = await createTab();
  tabs.value.push(tab);
  activeTabIndex.value = tabs.value.length - 1;
  await nextTick();
  mountTerminal(tab);
}

function closeTab(index: number) {
  const tab = tabs.value[index];
  if (!tab) return;

  // Kill the backend process — this is an explicit user action
  invoke("kill_terminal", { terminalId: tab.id }).catch(() => {});
  disposeTabFull(tab);

  tabs.value.splice(index, 1);

  if (tabs.value.length === 0) {
    activeTabIndex.value = -1;
  } else if (activeTabIndex.value >= tabs.value.length) {
    activeTabIndex.value = tabs.value.length - 1;
  }

  if (activeTabIndex.value >= 0) {
    nextTick(() => tabs.value[activeTabIndex.value]?.fitAddon?.fit());
  }
}

function switchTab(index: number) {
  if (index === activeTabIndex.value) return;
  activeTabIndex.value = index;
  nextTick(() => {
    const tab = tabs.value[index];
    if (tab) {
      setTimeout(() => tab.fitAddon?.fit(), 50);
    }
  });
}

function mountTerminal(tab: TabState) {
  const el = containerEls.value[tab.id];
  if (!el) return;

  const term = new Terminal({
    fontSize: 13,
    fontFamily:
      "JetBrains Mono, Fira Code, Cascadia Code, SF Mono, Menlo, monospace",
    theme: {
      background: "#0d1117",
      foreground: "#c9d1d9",
      cursor: "#58a6ff",
      cursorAccent: "#0d1117",
      selectionBackground: "#264f78",
      black: "#484f58",
      red: "#ff7b72",
      green: "#3fb950",
      yellow: "#d29922",
      blue: "#58a6ff",
      magenta: "#bc8cff",
      cyan: "#39c5cf",
      white: "#b1bac4",
      brightBlack: "#6e7681",
      brightRed: "#ffa198",
      brightGreen: "#56d364",
      brightYellow: "#e3b341",
      brightBlue: "#79c0ff",
      brightMagenta: "#d2a8ff",
      brightCyan: "#56d4dd",
      brightWhite: "#f0f6fc",
    },
    cursorBlink: true,
    cursorStyle: "bar",
    cursorWidth: 2,
    scrollback: 5000,
    allowProposedApi: true,
    smoothScrollDuration: 0,
    drawBoldTextInBrightColors: true,
    macOptionIsMeta: true,
  });

  const fitAddon = new FitAddon();
  term.loadAddon(fitAddon);
  term.open(el);

  term.onData((data) => {
    const bytes = new TextEncoder().encode(data);
    invoke("write_terminal", {
      terminalId: tab.id,
      data: Array.from(bytes),
    }).catch(() => {});
  });

  tab.term = term;
  tab.fitAddon = fitAddon;

  setTimeout(() => fitAddon.fit(), 150);

  tab.resizeObserver = new ResizeObserver(() => {
    fitAddon.fit();
  });
  tab.resizeObserver.observe(el);
}

// ═══════════════════════════════════════════════════
// Directory-switch logic — persist per-directory
// ═══════════════════════════════════════════════════

watch(
  () => props.cwd,
  async (newCwd, oldCwd) => {
    // Save old directory's terminal state
    if (oldCwd) {
      saveDirectoryState(oldCwd);
    }
    // Tear down current DOM/listeners
    teardownCurrentTabs();
    tabs.value = [];
    activeTabIndex.value = -1;

    if (newCwd) {
      const restored = await restoreDirectoryState(newCwd);
      if (restored) {
        tabs.value = restored.restoredTabs;
        activeTabIndex.value = restored.restoredIndex;
        tabCounter = restored.restoredCounter;
        await nextTick();
        const tab = tabs.value[activeTabIndex.value];
        if (tab) mountTerminal(tab);
      } else {
        await nextTick();
        await addTab();
      }
    }
  }
);

// ═══════════════════════════════════════════════════
// Lifecycle
// ═══════════════════════════════════════════════════

onMounted(async () => {
  await nextTick();
  if (props.cwd) {
    const restored = await restoreDirectoryState(props.cwd);
    if (restored) {
      tabs.value = restored.restoredTabs;
      activeTabIndex.value = restored.restoredIndex;
      tabCounter = restored.restoredCounter;
      await nextTick();
      const tab = tabs.value[activeTabIndex.value];
      if (tab) mountTerminal(tab);
    } else {
      await addTab();
    }
  } else {
    await addTab();
  }
});

onBeforeUnmount(() => {
  // Save state (keep backend processes alive)
  if (props.cwd) {
    saveDirectoryState(props.cwd);
  }
  // Tear down DOM + listeners
  teardownCurrentTabs();
  // Note: backend terminal processes are NOT killed here.
  // They persist until the user explicitly closes a tab or the app exits.
});
</script>

<template>
  <div class="flex flex-col h-full bg-[#0d1117]">
    <!-- Header bar: single row with tabs inline -->
    <div
      class="flex items-center h-[36px] flex-shrink-0 select-none border-b border-white/[0.06]"
      style="background: linear-gradient(180deg, #161b22 0%, #0d1117 100%)"
    >
      <!-- Left: status indicator -->
      <div class="flex items-center gap-1.5 pl-3 pr-1.5 shrink-0">
        <span class="w-[5px] h-[5px] rounded-full bg-[#3fb950] ring-1 ring-[#3fb950]/30" />
        <span class="text-[10px] font-semibold text-[#8b949e] tracking-[0.04em] uppercase">TERMINAL</span>
      </div>

      <!-- Tabs: horizontal scroll -->
      <div
        class="flex items-center gap-0.5 min-w-0 max-w-[420px] overflow-x-auto [&::-webkit-scrollbar]:hidden"
        style="scrollbar-width: none;"
        ref="tabsScrollEl"
      >
        <div
          v-for="(tab, i) in tabs"
          :key="tab.id"
          @click="switchTab(i)"
          @click.middle.prevent="closeTab(i)"
          class="flex items-center gap-1 px-2 h-[22px] rounded text-[10px] whitespace-nowrap shrink-0 cursor-pointer transition-colors select-none"
          :class="
            i === activeTabIndex
              ? 'bg-[#0d1117] text-[#c9d1d9] border border-white/[0.08]'
              : 'text-[#484f58] hover:text-[#8b949e] hover:bg-white/[0.04]'
          "
        >
          <TerminalIcon :size="9" />
          <span>{{ tab.title }}</span>
          <button
            @click.stop="closeTab(i)"
            class="w-[14px] h-[14px] flex items-center justify-center rounded hover:bg-white/[0.1] text-[#484f58] hover:text-[#c9d1d9]"
            title="Close"
          >
            <X :size="8" />
          </button>
        </div>
        <button
          @click="addTab"
          class="w-[22px] h-[22px] flex items-center justify-center rounded text-[#484f58] hover:text-[#8b949e] hover:bg-white/[0.06] transition-colors shrink-0"
          title="New Terminal"
        >
          <Plus :size="13" />
        </button>
      </div>

      <!-- Right: actions -->
      <div class="flex items-center gap-px pr-2 shrink-0 ml-auto">
        <button
          @click="handleClear"
          class="w-[22px] h-[22px] flex items-center justify-center rounded text-[#8b949e] hover:text-[#c9d1d9] hover:bg-white/[0.08] transition-colors cursor-pointer"
          title="Clear"
        >
          <Trash2 :size="11.5" />
        </button>
        <button
          @click="emit('close')"
          class="w-[22px] h-[22px] flex items-center justify-center rounded text-[#8b949e] hover:text-[#c9d1d9] hover:bg-white/[0.08] transition-colors cursor-pointer"
          title="Close"
        >
          <X :size="11.5" />
        </button>
      </div>
    </div>

    <!-- Terminal containers -->
    <div class="flex-1 relative overflow-hidden">
      <div
        v-for="(tab, i) in tabs"
        :key="tab.id"
        :ref="(el) => { if (el) containerEls[tab.id] = el as HTMLElement }"
        class="absolute inset-0"
        :class="{ 'hidden': i !== activeTabIndex }"
      />
      <!-- Empty state when no tabs -->
      <div
        v-if="tabs.length === 0"
        class="absolute inset-0 flex items-center justify-center text-[#30363d]"
      >
        <TerminalIcon :size="28" class="opacity-20" />
      </div>
    </div>
  </div>
</template>
