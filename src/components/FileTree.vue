<script setup lang="ts">
import { ref, watch, onMounted, computed } from "vue";
import { listDir, searchFiles, type FileEntry, type FileSearchResult } from "../api/fs";
import FileTreeNode from "./FileTreeNode.vue";
import {
  Folder,
  RefreshCw,
  ExternalLink,
  Search,
  X,
  File,
} from "lucide-vue-next";
import { openInFinder } from "../api/app";

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

// Search state
const searchQuery = ref("");
const searchResults = ref<FileSearchResult[]>([]);
const searching = ref(false);
let searchTimer: ReturnType<typeof setTimeout> | null = null;

const isSearching = computed(() => searchQuery.value.trim().length > 0);

const filenameResults = computed(() =>
  searchResults.value.filter((r) => r.match_type === "filename")
);
const contentResults = computed(() =>
  searchResults.value.filter((r) => r.match_type === "content")
);

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

function handleSearchResultClick(result: FileSearchResult) {
  selectedPath.value = result.path;
  emit("select-file", result.path);
}

function doSearch() {
  if (searchTimer) clearTimeout(searchTimer);
  const q = searchQuery.value.trim();
  if (!q) {
    searchResults.value = [];
    return;
  }
  searching.value = true;
  searchTimer = setTimeout(async () => {
    try {
      searchResults.value = await searchFiles(props.rootPath, q, 100);
    } catch (err) {
      console.error("Search failed:", err);
      searchResults.value = [];
    } finally {
      searching.value = false;
    }
  }, 200);
}

function clearSearch() {
  searchQuery.value = "";
  searchResults.value = [];
  searching.value = false;
}

function isPreviewable(ext: string) {
  return isImageFile(ext) || isPdfFile(ext) || isExcelFile(ext) || isTextFile(ext);
}

function getExt(name: string): string {
  const parts = name.split(".");
  if (parts.length > 1) return parts.pop()!.toLowerCase();
  return "";
}

function highlightMatch(text: string, query: string): { before: string; match: string; after: string } {
  const lower = text.toLowerCase();
  const idx = lower.indexOf(query.toLowerCase());
  if (idx === -1) return { before: text, match: "", after: "" };
  return {
    before: text.slice(0, idx),
    match: text.slice(idx, idx + query.length),
    after: text.slice(idx + query.length),
  };
}

watch(() => props.rootPath, () => {
  expanded.value.clear();
  selectedPath.value = null;
  clearSearch();
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
      <div class="flex items-center gap-1">
        <button
          @click="openInFinder(props.rootPath)"
          class="p-1 rounded-md text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors flex-shrink-0"
          title="Open in Finder"
        >
          <ExternalLink :size="13" />
        </button>
        <button
          @click="loadEntries"
          class="p-1 rounded-md text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors flex-shrink-0"
          title="Refresh"
        >
          <RefreshCw :size="13" :class="{ 'animate-spin': loading }" />
        </button>
      </div>
    </div>

    <!-- search input -->
    <div class="px-2.5 py-2 border-b border-gray-100 flex-shrink-0">
      <div class="relative">
        <Search :size="13" class="absolute left-2.5 top-1/2 -translate-y-1/2 text-gray-400" />
        <input
          v-model="searchQuery"
          @input="doSearch"
          placeholder="Search files..."
          class="w-full pl-8 pr-7 py-1.5 text-[12px] bg-gray-50 border border-gray-200 rounded-lg outline-none focus:border-blue-300 focus:bg-white transition-colors placeholder-gray-400"
        />
        <button
          v-if="searchQuery"
          @click="clearSearch"
          class="absolute right-2 top-1/2 -translate-y-1/2 text-gray-400 hover:text-gray-600 p-0.5"
        >
          <X :size="12" />
        </button>
      </div>
    </div>

    <!-- tree / search results -->
    <div class="flex-1 overflow-y-auto py-1">
      <!-- Loading -->
      <div v-if="isSearching && searching" class="flex items-center justify-center py-8">
        <div class="w-4 h-4 border-2 border-gray-300 border-t-gray-600 rounded-full animate-spin"></div>
      </div>

      <!-- Search results -->
      <template v-else-if="isSearching">
        <div v-if="searchResults.length === 0" class="flex flex-col items-center justify-center py-12 text-gray-300">
          <Search :size="28" class="mb-2 opacity-30" />
          <p class="text-[12px] text-gray-400">No results</p>
        </div>

        <template v-else>
          <!-- Filename matches -->
          <div v-if="filenameResults.length > 0">
            <p class="px-3 pt-2 pb-1 text-[10px] font-semibold text-gray-400 uppercase tracking-wider">
              Files ({{ filenameResults.length }})
            </p>
            <button
              v-for="r in filenameResults"
              :key="r.path"
              @click="handleSearchResultClick(r)"
              :class="[
                'w-full flex items-center gap-1.5 px-3 py-1 text-left transition-colors',
                selectedPath === r.path ? 'bg-blue-50 text-blue-700' : 'hover:bg-gray-50 text-gray-700'
              ]"
            >
              <File :size="13" :class="['flex-shrink-0', getIconClass(getExt(r.name))]" />
              <span class="text-[12px] truncate flex-shrink-0">{{ r.name }}</span>
              <span class="text-[10px] text-gray-400 truncate flex-1">{{ r.relative_path }}</span>
            </button>
          </div>

          <!-- Content matches -->
          <div v-if="contentResults.length > 0">
            <p class="px-3 pt-2 pb-1 text-[10px] font-semibold text-gray-400 uppercase tracking-wider">
              In Files ({{ contentResults.length }})
            </p>
            <button
              v-for="r in contentResults"
              :key="r.path + ':' + r.line_number"
              @click="handleSearchResultClick(r)"
              :class="[
                'w-full text-left px-3 py-1.5 transition-colors',
                selectedPath === r.path ? 'bg-blue-50' : 'hover:bg-gray-50'
              ]"
            >
              <div class="flex items-center gap-1.5 mb-0.5">
                <File :size="12" :class="['flex-shrink-0', getIconClass(getExt(r.name))]" />
                <span class="text-[12px] text-gray-700 truncate">{{ r.name }}</span>
                <span class="text-[10px] text-gray-400 flex-shrink-0">:{{ r.line_number }}</span>
              </div>
              <div v-if="r.line_content" class="text-[11px] text-gray-500 truncate pl-5">
                <span>{{ highlightMatch(r.line_content, searchQuery).before }}</span><mark class="bg-yellow-200 text-gray-700 rounded px-0.5">{{ highlightMatch(r.line_content, searchQuery).match }}</mark><span>{{ highlightMatch(r.line_content, searchQuery).after }}</span>
              </div>
            </button>
          </div>
        </template>
      </template>

      <!-- File tree (default view) -->
      <template v-else>
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
      </template>
    </div>
  </div>
</template>
