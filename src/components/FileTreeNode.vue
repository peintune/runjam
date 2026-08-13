<script setup lang="ts">
import { ref, watch, type Component } from "vue";
import {
  Folder, FolderOpen, File, FileText, FileCode, FileJson,
  FileSpreadsheet, FileImage, FileType, FileArchive,
  ChevronRight, Loader,
} from "lucide-vue-next";
import type { FileEntry } from "../api/fs";

const props = defineProps<{
  entry: FileEntry;
  depth: number;
  expanded: Set<string>;
  selectedPath: string | null;
  isPreviewable: (ext: string) => boolean;
  getIconClass: (ext: string) => string;
  /** Loads (and caches) a directory's children — provided by FileTree so that
   *  recreated nodes for already-expanded folders restore cached children. */
  resolveChildren: (dirPath: string) => Promise<FileEntry[]>;
}>();

const emit = defineEmits<{
  (e: "toggle", path: string): void;
  (e: "select", entry: FileEntry): void;
}>();

const children = ref<FileEntry[]>([]);
const loadingChildren = ref(false);

async function loadChildren(dirPath: string) {
  loadingChildren.value = true;
  try {
    children.value = await props.resolveChildren(dirPath);
  } catch (err) {
    console.error("Failed to load children:", err);
  } finally {
    loadingChildren.value = false;
  }
}

function handleClick() {
  if (props.entry.is_dir) {
    emit("toggle", props.entry.path);
    if (!props.expanded.has(props.entry.path) && children.value.length === 0) {
      // Will expand, so load children
    } else if (props.expanded.has(props.entry.path)) {
      // Will collapse
    }
  } else {
    emit("select", props.entry);
  }
}

const isExpanded = () => props.expanded.has(props.entry.path);

// Load children when expanded. immediate:true so a recreated node for an
// already-expanded folder (e.g. after switching directories) loads its children
// right away — from the shared cache when available, avoiding a re-scan.
watch(
  () => props.expanded.has(props.entry.path),
  (nowExpanded) => {
    if (nowExpanded && children.value.length === 0 && props.entry.is_dir) {
      loadChildren(props.entry.path);
    }
  },
  { immediate: true }
);

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

// ── File-type → icon mapping ──────────────────────────────────────
const FILE_ICON_MAP: Record<string, Component> = {
  // Text / docs
  txt: FileText, md: FileText, log: FileText, env: FileText,
  rtf: FileText,
  // Code
  ts: FileCode, tsx: FileCode, js: FileCode, jsx: FileCode,
  vue: FileCode, rs: FileCode, py: FileCode, go: FileCode,
  java: FileCode, c: FileCode, cpp: FileCode, h: FileCode, hpp: FileCode,
  rb: FileCode, php: FileCode, swift: FileCode, kt: FileCode, scala: FileCode,
  sh: FileCode, bash: FileCode, zsh: FileCode,
  sql: FileCode, graphql: FileCode, prisma: FileCode, proto: FileCode,
  css: FileCode, scss: FileCode, less: FileCode, html: FileCode,
  dockerfile: FileCode, gitignore: FileCode, makefile: FileCode,
  // Data / config
  json: FileJson,
  yaml: FileText, yml: FileText, toml: FileText, xml: FileText, ini: FileText,
  // Office
  docx: FileType, doc: FileType, odt: FileType,
  xlsx: FileSpreadsheet, xls: FileSpreadsheet, ods: FileSpreadsheet,
  pptx: File, ppt: File, odp: File,
  csv: FileSpreadsheet,
  // Images
  png: FileImage, jpg: FileImage, jpeg: FileImage, gif: FileImage,
  svg: FileImage, webp: FileImage, ico: FileImage, bmp: FileImage,
  // Archives
  zip: FileArchive, tar: FileArchive, gz: FileArchive, bz2: FileArchive,
  xz: FileArchive, "7z": FileArchive, rar: FileArchive,
};

function getFileIcon(ext: string): Component {
  return FILE_ICON_MAP[ext.toLowerCase()] || File;
}
</script>

<template>
  <div>
    <button
      @click="handleClick"
      :class="[
        'w-full flex items-center gap-1 px-2 py-0.5 text-left transition-colors cursor-pointer group',
        selectedPath === entry.path
          ? 'bg-blue-50 text-blue-700'
          : 'hover:bg-gray-50 text-gray-700',
      ]"
      :style="{ paddingLeft: `${8 + depth * 16}px` }"
    >
      <!-- expand icon -->
      <span v-if="entry.is_dir" class="w-4 h-4 flex items-center justify-center flex-shrink-0">
        <Loader v-if="loadingChildren" :size="12" class="animate-spin text-gray-400" />
        <ChevronRight
          v-else
          :size="12"
          class="transition-transform duration-150 text-gray-400"
          :class="{ 'rotate-90': isExpanded() }"
        />
      </span>
      <span v-else class="w-4 flex-shrink-0" />

      <!-- icon -->
      <span v-if="entry.is_dir && isExpanded()">
        <FolderOpen :size="14" class="text-gray-700 flex-shrink-0" />
      </span>
      <span v-else-if="entry.is_dir">
        <Folder :size="14" class="text-gray-600 flex-shrink-0" />
      </span>
      <span v-else>
        <component :is="getFileIcon(entry.extension)" :size="14" :class="['flex-shrink-0', getIconClass(entry.extension)]" />
      </span>

      <!-- name -->
      <span class="text-[12px] truncate flex-1">{{ entry.name }}</span>

      <!-- size badge for files -->
      <span v-if="!entry.is_dir && entry.size > 0" class="text-[10px] text-gray-400 ml-1 flex-shrink-0 hidden group-hover:inline">
        {{ formatSize(entry.size) }}
      </span>
    </button>

    <!-- children -->
    <div v-if="entry.is_dir && isExpanded()">
      <FileTreeNode
        v-for="child in children"
        :key="child.path"
        :entry="child"
        :depth="depth + 1"
        :expanded="expanded"
        :selected-path="selectedPath"
        :is-previewable="isPreviewable"
        :get-icon-class="getIconClass"
        :resolve-children="resolveChildren"
        @toggle="emit('toggle', $event)"
        @select="emit('select', $event)"
      />
      <div v-if="children.length === 0 && !loadingChildren" class="text-[11px] text-gray-400 pl-2 py-1" :style="{ paddingLeft: `${8 + (depth + 1) * 16 + 16}px` }">
        (empty)
      </div>
    </div>
  </div>
</template>
