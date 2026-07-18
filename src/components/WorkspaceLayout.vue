<script setup lang="ts">
import { ref, computed, watch, onMounted } from "vue";
import Sidebar from "./Sidebar.vue";
import SessionView from "./SessionView.vue";
import WorkspacePanel from "./WorkspacePanel.vue";
import SearchButton from "./SearchButton.vue";
import { useWorkspaceStore } from "../stores/useWorkspaceStore";
import { useDragResize } from "../composables/useDragResize";
import { useSessionLayout } from "../composables/useSessionLayout";
import {
  PanelLeftOpen, PanelLeftClose,
  Files, Terminal,
} from "lucide-vue-next";

// ── State ────────────────────────────────────────
const sidebarPinned = ref(true);
const sidebarHover = ref(false);
const isWorkspaceMode = ref(false);

const store = useWorkspaceStore();
const { layout, switchDirectory, saveLayout } = useSessionLayout();

/** Get directoryId from current active session */
function currentDirectoryId(): string | null {
  const session = store.activeSession;
  if (!session?.directoryId) return null;
  return session.directoryId;
}

// Terminal visibility comes from persisted layout
const showTerminal = ref(false);

const activeDirectory = computed(() => {
  if (!store.activeSession?.directoryId) return "";
  const dir = store.directories.find((d) => d.id === store.activeSession!.directoryId);
  return dir?.path || "";
});

// Enable workspace mode only when there's a bound directory
const canEnableWorkspace = computed(() => !!activeDirectory.value);

function toggleWorkspace() {
  if (!canEnableWorkspace.value) return;
  isWorkspaceMode.value = !isWorkspaceMode.value;
  if (isWorkspaceMode.value) {
    sidebarPinned.value = false;
  }
}

function toggleTerminal() {
  if (!isWorkspaceMode.value) {
    isWorkspaceMode.value = true;
    sidebarPinned.value = false;
  }
  showTerminal.value = !showTerminal.value;
  layout.showTerminal = showTerminal.value;
  saveLayout();
}

// ---- Session persistence (keyed by directoryId) ----
watch(() => store.activeSessionId, (newId) => {
  const dirId = currentDirectoryId();
  switchDirectory(dirId);
  if (newId) {
    showTerminal.value = layout.showTerminal;
  }
});

onMounted(() => {
  if (store.activeSessionId) {
    const dirId = currentDirectoryId();
    switchDirectory(dirId);
    showTerminal.value = layout.showTerminal;
  }
});

// ---- Resizable sidebar ----
const sidebarResize = useDragResize({
  direction: "horizontal",
  minSize: 180,
  defaultSize: 270,
  initialSize: layout.sidebarWidth,
  onDragEnd: (size) => { layout.sidebarWidth = size; },
});

// ---- Resizable chat panel ----
const chatResize = useDragResize({
  direction: "horizontal",
  minSize: 300,
  defaultSize: 420,
  reversed: true,
  initialSize: layout.chatWidth,
  onDragEnd: (size) => { layout.chatWidth = size; },
});

// Sync resize sizes when layout changes (session switch)
watch(() => layout.sidebarWidth, (w) => { sidebarResize.size.value = w; });
watch(() => layout.chatWidth, (w) => { chatResize.size.value = w; });
</script>

<template>
  <div class="flex flex-col h-screen bg-[#f2f3f5] relative overflow-hidden">

    <!-- ═══ Top Nav Bar (always visible) ═══ -->
    <div
      data-tauri-drag-region
      class="flex-shrink-0 h-8 flex items-center px-4 w-full border-b border-gray-200/60 bg-white/90 backdrop-blur-sm z-30"
      style="-webkit-app-region: drag"
    >
      <div class="w-[70px] flex-shrink-0" />

      <!-- Sidebar toggle: hide when pinned, show when hidden -->
      <button
        v-if="sidebarPinned"
        @click="sidebarPinned = !sidebarPinned"
        class="p-1.5 rounded-lg text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors duration-150"
        style="-webkit-app-region: no-drag"
        title="Hide sidebar"
      >
        <PanelLeftOpen :size="18" />
      </button>
      <button
        v-if="!sidebarPinned"
        @click="sidebarPinned = true"
        @mouseenter="sidebarHover = true"
        class="p-1.5 rounded-lg text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors duration-150"
        style="-webkit-app-region: no-drag"
        title="Show sidebar"
      >
        <PanelLeftClose :size="18" />
      </button>

      <SearchButton />
      <div class="flex-1" />

      <!-- File explorer toggle -->
      <button
        v-if="canEnableWorkspace"
        @click="toggleWorkspace"
        class="p-1.5 rounded-lg transition-colors duration-150 ml-1"
        :class="isWorkspaceMode ? 'text-gray-700 bg-gray-200 hover:bg-gray-300' : 'text-gray-400 hover:text-gray-600 hover:bg-gray-100'"
        style="-webkit-app-region: no-drag"
        :title="isWorkspaceMode ? 'Close explorer' : 'Open explorer'"
      >
        <Files :size="18" />
      </button>

      <!-- Terminal toggle -->
      <button
        v-if="canEnableWorkspace"
        @click="toggleTerminal"
        class="p-1.5 rounded-lg transition-colors duration-150 ml-0.5"
        :class="showTerminal ? 'text-gray-700 bg-gray-200 hover:bg-gray-300' : 'text-gray-400 hover:text-gray-600 hover:bg-gray-100'"
        style="-webkit-app-region: no-drag"
        title="Toggle terminal"
      >
        <Terminal :size="18" />
      </button>
    </div>

    <!-- ═══ Sidebar Overlay: slides in from left when hovering toggle ═══ -->
    <Transition name="sidebar-slide">
      <div
        v-if="sidebarHover && !sidebarPinned"
        class="fixed top-8 left-0 z-40 h-[calc(100vh-32px)] flex flex-col py-0 pl-[3px] pr-[3px] pb-[3px]"
        :style="{ width: (sidebarResize.size.value + 6) + 'px' }"
        @mouseleave="sidebarHover = false"
      >
        <div class="flex-1 min-h-0">
          <Sidebar class="h-full" />
        </div>
      </div>
    </Transition>

    <!-- ═══ Body ═══ -->
    <div class="flex flex-1 min-h-0">
      <!-- Pinned Sidebar (with smooth width transition) -->
      <div
        class="transition-all duration-200 ease-out flex-shrink-0 flex flex-col py-0 px-[3px] pb-[3px]"
        :class="sidebarPinned ? '' : 'w-0 overflow-hidden'"
        :style="{ width: sidebarPinned ? (sidebarResize.size.value + 6) + 'px' : undefined }"
      >
        <div class="flex-1 min-h-0">
          <Sidebar class="h-full" />
        </div>
      </div>

      <!-- Resize handle between sidebar and main content -->
      <div
        v-if="sidebarPinned"
        class="w-px flex-shrink-0 cursor-col-resize transition-colors rounded-full hover:bg-blue-400/40"
        :class="sidebarResize.isDragging.value ? 'bg-blue-400' : 'bg-transparent'"
        @mousedown="sidebarResize.startDrag"
      />

      <!-- Main content area -->
      <div class="flex-1 flex min-w-0 gap-[3px]">
        <!-- Workspace Panel -->
        <WorkspacePanel
          v-if="isWorkspaceMode && activeDirectory"
          :show-terminal="showTerminal"
          @update:show-terminal="(val: boolean) => { showTerminal = val; layout.showTerminal = val; saveLayout(); }"
        />

        <!-- Resize handle between workspace and chat -->
        <div
          v-if="isWorkspaceMode && activeDirectory"
          class="w-px flex-shrink-0 cursor-col-resize transition-colors rounded-full hover:bg-blue-400/40"
          :class="chatResize.isDragging.value ? 'bg-blue-400' : 'bg-transparent'"
          @mousedown="chatResize.startDrag"
        />

        <!-- Chat / Session View -->
        <div
          class="flex-shrink-0 rounded-lg overflow-hidden bg-white shadow-[0_0_0_1px_rgba(0,0,0,0.04)] flex flex-col min-h-0"
          :style="isWorkspaceMode && activeDirectory
            ? { width: chatResize.size.value + 'px' }
            : {}"
          :class="!isWorkspaceMode || !activeDirectory ? 'flex-1' : ''"
        >
          <SessionView />
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.sidebar-slide-enter-active,
.sidebar-slide-leave-active {
  transition: transform 0.2s ease, opacity 0.2s ease;
}
.sidebar-slide-enter-from,
.sidebar-slide-leave-to {
  transform: translateX(-100%);
  opacity: 0;
}
</style>
