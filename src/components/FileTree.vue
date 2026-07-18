<script setup lang="ts">
import { ref, watch, onMounted } from "vue";
import { listDir, type FileEntry } from "../api/fs";
import FileTreeNode from "./FileTreeNode.vue";
import {
  Folder,
  RefreshCw,
} from "lucide-vue-next";

const props = defineProps<{
  rootPath: string;
}>();

const emit = defineEmits<{
  (e: "select-file", path: string): void;
}>();

const entries = ref<FileEntry[]>([]);
const expanded = ref<Set<string>>(new Set());
const loading = ref(false);
const selectedPath = ref<string | null>(null);

function getIconClass(ext: string) {
  const map: Record<string, string> = {
    ts: "text-blue-500",
    tsx: "text-cyan-500",
    js: "text-yellow-500",
    jsx: "text-cyan-500",
    vue: "text-emerald-500",
    rs: "text-orange-500",
    py: "text-blue-400",
    go: "text-cyan-400",
    json: "text-yellow-400",
    md: "text-gray-500",
    css: "text-pink-500",
    html: "text-orange-400",
    svg: "text-purple-500",
    yaml: "text-red-400",
    yml: "text-red-400",
    toml: "text-gray-400",
  };
  return map[ext] || "text-gray-400";
}

function isImageFile(ext: string) {
  return ["png", "jpg", "jpeg", "gif", "svg", "webp", "ico"].includes(ext);
}

function isPdfFile(ext: string) {
  return ext === "pdf";
}

function isExcelFile(ext: string) {
  return ["xlsx", "xls", "csv"].includes(ext);
}

function isTextFile(ext: string) {
  const textExts = [
    "ts", "tsx", "js", "jsx", "vue", "rs", "py", "go", "java", "c", "cpp",
    "h", "hpp", "rb", "php", "swift", "kt", "scala", "sh", "bash", "zsh",
    "json", "yaml", "yml", "toml", "xml", "md", "txt", "log", "env",
    "css", "scss", "less", "html", "svg", "gitignore", "dockerfile",
    "sql", "graphql", "prisma", "proto",
  ];
  return textExts.includes(ext) || ext === "";
}

async function loadEntries() {
  if (!props.rootPath) return;
  loading.value = true;
  try {
    entries.value = await listDir(props.rootPath);
  } catch (err) {
    console.error("Failed to list directory:", err);
  } finally {
    loading.value = false;
  }
}

function toggleExpand(path: string) {
  if (expanded.value.has(path)) {
    expanded.value.delete(path);
  } else {
    expanded.value.add(path);
  }
}

function handleFileClick(entry: FileEntry) {
  if (entry.is_dir) {
    toggleExpand(entry.path);
  } else {
    selectedPath.value = entry.path;
    emit("select-file", entry.path);
  }
}

function isPreviewable(ext: string) {
  return isImageFile(ext) || isPdfFile(ext) || isExcelFile(ext) || isTextFile(ext);
}

watch(() => props.rootPath, () => {
  expanded.value.clear();
  selectedPath.value = null;
  loadEntries();
}, { immediate: true });

onMounted(() => {
  if (props.rootPath) loadEntries();
});
</script>

<template>
  <div class="h-full flex flex-col bg-white border-r border-gray-100">
    <!-- header -->
    <div class="flex items-center justify-between px-3 py-2.5 border-b border-gray-100 flex-shrink-0">
      <div class="flex items-center gap-1.5 min-w-0">
        <Folder :size="14" class="text-gray-600 flex-shrink-0" />
        <span class="text-[12px] font-medium text-gray-700 truncate" :title="rootPath">
          {{ rootPath.split('/').pop() || rootPath }}
        </span>
      </div>
      <button
        @click="loadEntries"
        class="p-1 rounded-md text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors flex-shrink-0"
        title="Refresh"
      >
        <RefreshCw :size="13" :class="{ 'animate-spin': loading }" />
      </button>
    </div>

    <!-- tree -->
    <div class="flex-1 overflow-y-auto py-1">
      <div v-if="loading" class="flex items-center justify-center py-8">
        <div class="w-4 h-4 border-2 border-gray-300 border-t-gray-600 rounded-full animate-spin"></div>
      </div>
      <div v-else-if="entries.length === 0" class="flex flex-col items-center justify-center py-12 text-gray-300">
        <Folder :size="28" class="mb-2 opacity-30" />
        <p class="text-[12px] text-gray-400">Empty directory</p>
      </div>
      <template v-else>
        <FileTreeNode
          v-for="entry in entries"
          :key="entry.path"
          :entry="entry"
          :depth="0"
          :expanded="expanded"
          :selected-path="selectedPath"
          :is-previewable="isPreviewable"
          :get-icon-class="getIconClass"
          @toggle="toggleExpand"
          @select="handleFileClick"
        />
      </template>
    </div>
  </div>
</template>
