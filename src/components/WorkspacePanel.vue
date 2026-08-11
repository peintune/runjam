<script setup lang="ts">
import { computed, watch } from "vue";
import { useDragResize } from "../composables/useDragResize";
import { useSessionLayout } from "../composables/useSessionLayout";
import FileTree from "./FileTree.vue";
import FileEditor from "./FileEditor.vue";
import FilePreview from "./FilePreview.vue";
import TerminalPanel from "./TerminalPanel.vue";
import { FileText, X } from "lucide-vue-next";
import { openInFinder } from "../api/app";

const props = defineProps<{
  showTerminal: boolean;
  rootPath: string;
}>();

const emit = defineEmits<{
  (e: "update:showTerminal", value: boolean): void;
}>();

const { layout, saveLayout } = useSessionLayout();

// The working-directory root is now passed from the parent (WorkspaceLayout),
// which computes it with full fallback logic (bound directory → session.directory
// → default ~/.runjam/session/{id}). This ensures the file tree and terminal
// work for regular sessions too, not just project-bound ones.
const activeDirectory = computed(() => props.rootPath);

// ---- Multi-file tabs ----
const openFiles = computed({
  get: () => layout.openFiles,
  set: (val) => { layout.openFiles = val; },
});
const activeFileIndex = computed({
  get: () => layout.activeFileIndex,
  set: (val) => { layout.activeFileIndex = val; },
});

const activeFile = computed(() => {
  if (activeFileIndex.value < 0 || activeFileIndex.value >= openFiles.value.length) return null;
  return openFiles.value[activeFileIndex.value];
});

function isOfficeFile(ext: string) {
  return ["pptx", "ppt", "docx", "doc", "xlsx", "xls", "odt", "odp", "ods", "csv"].includes(ext);
}

function handleSelectFile(path: string) {
  const ext = path.split(".").pop()?.toLowerCase() || "";
  if (isOfficeFile(ext)) {
    openInFinder(path).catch((err) => console.error("Failed to open file:", err));
    return;
  }
  const idx = openFiles.value.indexOf(path);
  if (idx >= 0) {
    activeFileIndex.value = idx;
  } else {
    openFiles.value = [...openFiles.value, path];
    activeFileIndex.value = openFiles.value.length - 1;
  }
  saveLayout();
}

function closeTab(index: number) {
  const files = [...openFiles.value];
  files.splice(index, 1);
  openFiles.value = files;
  if (files.length === 0) {
    activeFileIndex.value = -1;
  } else if (activeFileIndex.value >= files.length) {
    activeFileIndex.value = files.length - 1;
  }
  saveLayout();
}

function handleCloseEditor() {
  if (activeFileIndex.value >= 0) {
    closeTab(activeFileIndex.value);
  }
}

function switchTab(index: number) {
  activeFileIndex.value = index;
}

function isImage(ext: string) {
  return ["png", "jpg", "jpeg", "gif", "svg", "webp", "ico"].includes(ext);
}

function isPdf(ext: string) {
  return ext === "pdf";
}

const fileMode = computed(() => {
  if (!activeFile.value) return null;
  const ext = activeFile.value.split(".").pop()?.toLowerCase() || "";
  if (isImage(ext) || isPdf(ext)) return "preview";
  return "editor";
});

// ---- Resizable panels ----
const fileTreeResize = useDragResize({
  direction: "horizontal",
  minSize: 120,
  defaultSize: 260,
  initialSize: layout.fileTreeWidth,
  onDragEnd: (size) => { layout.fileTreeWidth = size; },
});

const terminalResize = useDragResize({
  direction: "vertical",
  minSize: 80,
  defaultSize: 260,
  initialSize: layout.terminalHeight,
  onDragEnd: (size) => { layout.terminalHeight = size; },
});

// Sync resize sizes when layout changes (session switch)
watch(() => layout.fileTreeWidth, (w) => { fileTreeResize.size.value = w; });
watch(() => layout.terminalHeight, (h) => { terminalResize.size.value = h; });
</script>

<template>
  <div class="flex-1 flex min-h-0 min-w-0 gap-[3px]">
    <!-- File Tree panel -->
    <div
      class="flex-shrink-0 rounded-lg overflow-hidden bg-white shadow-[0_0_0_1px_rgba(0,0,0,0.04)]"
      :style="{ width: fileTreeResize.size.value + 'px' }"
    >
      <FileTree
        :key="activeDirectory"
        :root-path="activeDirectory"
        @select-file="handleSelectFile"
      />
    </div>

    <!-- File tree resize handle -->
    <div
      class="w-px flex-shrink-0 cursor-col-resize transition-colors rounded-full hover:bg-blue-400/40"
      :class="fileTreeResize.isDragging.value ? 'bg-blue-400' : 'bg-transparent'"
      @mousedown="fileTreeResize.startDrag"
    />

    <!-- Center: Tab bar + Editor + Terminal -->
    <div class="flex-1 flex flex-col min-w-0 gap-[3px]">
      <!-- Tab bar -->
      <div
        v-if="openFiles.length > 0"
        class="flex-shrink-0 min-w-0 overflow-hidden h-[30px] bg-[#fafbfc] border-b border-gray-200"
      >
        <div
          class="flex items-center h-full overflow-x-auto [&::-webkit-scrollbar]:hidden"
          style="scrollbar-width: none;"
        >
          <button
            v-for="(filePath, i) in openFiles"
            :key="filePath"
            @click="switchTab(i)"
            @click.middle.prevent="closeTab(i)"
            class="flex items-center gap-1 px-3 h-full text-[11px] whitespace-nowrap shrink-0 cursor-pointer transition-colors select-none"
            :class="
              i === activeFileIndex
                ? 'bg-white text-gray-900 font-semibold border-b-[2px] border-gray-800'
                : 'text-gray-400 hover:text-gray-600 hover:bg-gray-100/40 border-b-[2px] border-transparent'
            "
          >
            <span>{{ filePath.split('/').pop() }}</span>
            <button
              @click.stop="closeTab(i)"
              class="w-[15px] h-[15px] flex items-center justify-center rounded hover:bg-gray-200 text-gray-400 hover:text-gray-600 ml-0.5"
            >
              <X :size="10" />
            </button>
          </button>
        </div>
      </div>

      <!-- Editor / Preview area -->
      <div class="flex-1 min-h-0 rounded-lg overflow-hidden bg-white shadow-[0_0_0_1px_rgba(0,0,0,0.04)]">
        <FileEditor
          v-if="activeFile && fileMode === 'editor'"
          :file-path="activeFile"
          @close="handleCloseEditor"
        />
        <FilePreview
          v-else-if="activeFile && fileMode === 'preview'"
          :key="activeFile"
          :file-path="activeFile"
        />
        <!-- Empty state -->
        <div v-else class="h-full flex items-center justify-center bg-[#fafbfc]">
          <div class="flex flex-col items-center gap-2 text-gray-300">
            <FileText :size="36" class="opacity-30" />
            <p class="text-[12px] text-gray-400">Select a file from the explorer to start editing</p>
          </div>
        </div>
      </div>

      <!-- Terminal resize handle -->
      <div
        v-show="showTerminal"
        class="h-px flex-shrink-0 cursor-row-resize transition-colors rounded-full hover:bg-blue-400/40"
        :class="terminalResize.isDragging.value ? 'bg-blue-400' : 'bg-transparent'"
        @mousedown="terminalResize.startDrag"
      />

      <!-- Terminal panel (v-show keeps terminal processes alive when hidden) -->
      <div
        v-show="showTerminal"
        class="flex-shrink-0 rounded-lg overflow-hidden shadow-[0_0_0_1px_rgba(0,0,0,0.04)]"
        :style="{ height: terminalResize.size.value + 'px' }"
      >
        <TerminalPanel
          :cwd="activeDirectory"
          :active="showTerminal"
          @close="emit('update:showTerminal', false)"
        />
      </div>
    </div>
  </div>
</template>
