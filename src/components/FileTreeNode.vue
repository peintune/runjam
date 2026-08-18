<script setup lang="ts">
import { ref, watch, computed, type Component, nextTick, onBeforeUnmount } from "vue";
import {
  Folder, FolderOpen, File, FileText, FileCode, FileJson,
  FileSpreadsheet, FileImage, FileType, FileArchive,
  ChevronRight, Loader, FilePlus, FolderPlus,
} from "lucide-vue-next";
import type { FileEntry } from "../api/fs";
import { createDir, createFile, renamePath, deletePath } from "../api/fs";
import { openInFinder, revealPath } from "../api/app";
import { useToast } from "../composables/useToast";
import FileContextMenu from "./FileContextMenu.vue";
import MoveToDialog from "./MoveToDialog.vue";
import ConfirmDialog from "./ConfirmDialog.vue";

const props = defineProps<{
  entry: FileEntry;
  depth: number;
  expanded: Set<string>;
  selectedPath: string | null;
  isPreviewable: (ext: string) => boolean;
  getIconClass: (ext: string) => string;
  /** Loads (and caches) a directory's children — provided by FileTree so that
   *  recreated nodes for already-expanded folders restore cached children. */
  resolveChildren: (dirPath: string, force?: boolean) => Promise<FileEntry[]>;
  /** Optional: workspace root for move/rename safety checks. Falls back to
   *  `entry.path` parent for backward compatibility if not provided. */
  rootPath?: string;
  /** Bumped by FileTree after any write op. When it changes, an expanded
   *  folder re-reads its children from disk so creates/renames/moves show up
   *  immediately instead of relying on the stale cache. */
  mutationVersion?: number;
}>();

const emit = defineEmits<{
  (e: "toggle", path: string): void;
  (e: "select", entry: FileEntry): void;
  (e: "mutated", parentPath: string): void;
}>();

const { showError, showSuccess } = useToast();

const children = ref<FileEntry[]>([]);
const loadingChildren = ref(false);

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

// When FileTree reports a write elsewhere (create/rename/move), an expanded
// folder must re-read its children from disk. The cache may be stale, so force
// a fresh load. This is what makes a new/renamed/moved entry appear instantly
// in an already-open folder.
watch(
  () => props.mutationVersion,
  async () => {
    if (props.entry.is_dir && isExpanded()) {
      try {
        children.value = await props.resolveChildren(props.entry.path, true);
      } catch (err) {
        console.error("Failed to reload children after mutation:", err);
      }
    }
  }
);

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

// ── Right-click context menu ──────────────────────────────────────

interface ContextMenuState {
  x: number;
  y: number;
}
const contextMenu = ref<ContextMenuState | null>(null);

function onContextMenu(e: MouseEvent) {
  e.preventDefault();
  // Stop propagation so the tree container's root context menu doesn't also
  // fire — a right-click on an entry should show only the entry's menu.
  e.stopPropagation();
  // Only show menu for files/folders (not the inline edit/create rows).
  contextMenu.value = { x: e.clientX, y: e.clientY };
}

function closeContextMenu() {
  contextMenu.value = null;
}

// ── Inline rename ────────────────────────────────────────────────
const renaming = ref(false);
const renameValue = ref("");
const renameInputRef = ref<HTMLInputElement | null>(null);

function startRename() {
  contextMenu.value = null;
  renameValue.value = props.entry.name;
  renaming.value = true;
  nextTick(() => {
    renameInputRef.value?.focus();
    // Select the name without the extension for files — most renames change
    // the stem, not the extension. Falls back to selecting all if there's
    // no extension.
    const input = renameInputRef.value;
    if (input) {
      const dot = props.entry.name.lastIndexOf(".");
      if (dot > 0) {
        input.setSelectionRange(0, dot);
      } else {
        input.select();
      }
    }
  });
}

function parentOf(p: string): string {
  const idx = p.lastIndexOf("/");
  return idx > 0 ? p.slice(0, idx) : "/";
}

async function commitRename() {
  // Guard against double-invocation (Enter + blur firing together). Set
  // renaming false immediately so a second call is a no-op.
  if (!renaming.value) return;
  renaming.value = false;
  const next = renameValue.value.trim();
  if (!next || next === props.entry.name) {
    return;
  }
  if (next.includes("/") || next.includes("\\")) {
    showError(`Invalid name: "${next}" contains a path separator`);
    return;
  }
  const target = `${parentOf(props.entry.path)}/${next}`;
  try {
    await renamePath(props.entry.path, target, props.rootPath ?? parentOf(props.entry.path));
    emit("mutated", parentOf(props.entry.path));
    // If we're renaming a directory and it's currently expanded, also notify
    // the new path's parent (which is the same here, but expand state
    // keyed by old path will become stale — caller handles that by reload).
  } catch (err) {
    const msg = String(err);
    if (msg.includes("already exists")) {
      showError(`"${next}" already exists`);
    } else if (msg.includes("itself or its descendant")) {
      showError("Cannot move a folder into itself or its descendants");
    } else if (msg.includes("outside the workspace")) {
      showError("Path is outside the workspace");
    } else {
      showError(`Operation failed: ${msg}`);
    }
    // Keep the input open so the user can correct the name.
    renaming.value = true;
  }
}

/** Blur handler for the rename input. Deferred so a click on a sibling button
 *  (e.g. the context menu) can register first, and so Enter + blur don't both
 *  fire commitRename. */
function onRenameBlur() {
  setTimeout(() => {
    if (renaming.value) commitRename();
  }, 150);
}

function cancelRename() {
  renaming.value = false;
  renameValue.value = "";
}

function onRenameKey(e: KeyboardEvent) {
  if (e.key === "Enter") {
    e.preventDefault();
    commitRename();
  } else if (e.key === "Escape") {
    e.preventDefault();
    cancelRename();
  }
}

// ── Inline new file/folder (children of this directory) ──────────
const inlineNew = ref<{ kind: "file" | "folder"; value: string; error: string | null } | null>(null);

function startInlineNew(kind: "file" | "folder") {
  contextMenu.value = null;
  if (!props.entry.is_dir) return;
  if (!props.expanded.has(props.entry.path)) {
    emit("toggle", props.entry.path);
  }
  inlineNew.value = { kind, value: "", error: null };
  nextTick(() => {
    const input = document.getElementById(`inline-new-${props.entry.path}`) as HTMLInputElement | null;
    input?.focus();
  });
}

async function commitInlineNew() {
  if (!inlineNew.value) return;
  // Local handle — we null out `inlineNew.value` on success and the catch
  // block still wants to write `.error` on the original object.
  const ctx = inlineNew.value;
  const { kind, value } = ctx;
  const name = value.trim();
  if (!name) {
    inlineNew.value = null;
    return;
  }
  if (name.includes("/") || name.includes("\\")) {
    ctx.error = `Invalid name: "${name}" contains a path separator`;
    return;
  }
  const target = `${props.entry.path}/${name}`;
  try {
    if (kind === "file") {
      await createFile(target, props.rootPath ?? parentOf(props.entry.path));
    } else {
      await createDir(target, props.rootPath ?? parentOf(props.entry.path));
    }
    inlineNew.value = null;
    showSuccess(kind === "file" ? `Created ${name}` : `Created ${name}/`);
    // Reload the children of *this* dir with force=true so the newly created
    // entry appears immediately (the cache still holds the pre-create listing).
    children.value = await props.resolveChildren(props.entry.path, true);
    // Notify the parent that this directory's contents changed. We emit THIS
    // dir's path (not its parent) so FileTree refreshes the right cache and
    // doesn't collapse the whole tree.
    emit("mutated", props.entry.path);
  } catch (err) {
    const msg = String(err);
    if (msg.includes("already exists")) {
      ctx.error = `"${name}" already exists`;
    } else {
      ctx.error = msg;
    }
  }
}

function cancelInlineNew() {
  inlineNew.value = null;
}

function onInlineNewKey(e: KeyboardEvent) {
  if (e.key === "Enter") {
    e.preventDefault();
    commitInlineNew();
  } else if (e.key === "Escape") {
    e.preventDefault();
    cancelInlineNew();
  }
}

function onInlineNewBlur() {
  setTimeout(() => {
    if (inlineNew.value && inlineNew.value.value.trim()) {
      commitInlineNew();
    } else if (inlineNew.value) {
      cancelInlineNew();
    }
  }, 150);
}

// ── Move dialog ──────────────────────────────────────────────────
const moveDialogOpen = ref(false);

function openMoveDialog() {
  contextMenu.value = null;
  moveDialogOpen.value = true;
}

async function handleMoveConfirm(destDir: string) {
  // No-op: moving to the same parent it's already in. Close silently.
  if (destDir === parentOf(props.entry.path)) {
    moveDialogOpen.value = false;
    return;
  }
  const target = `${destDir}/${props.entry.name}`;
  try {
    await renamePath(props.entry.path, target, props.rootPath ?? destDir);
    moveDialogOpen.value = false;
    // Two parents changed: old parent and new parent. We can't emit two
    // events from here, so we emit for the new parent and let the caller
    // re-emit for the old parent by also calling mutate on this node's
    // former location. For simplicity, the FileTree root handles a full
    // children-cache invalidation on the next listDir call.
    emit("mutated", parentOf(props.entry.path));
    emit("mutated", destDir);
  } catch (err) {
    const msg = String(err);
    if (msg.includes("already exists")) {
      showError(`"${props.entry.name}" already exists`);
    } else if (msg.includes("itself or its descendant")) {
      showError("Cannot move a folder into itself or its descendants");
    } else if (msg.includes("outside the workspace")) {
      showError("Path is outside the workspace");
    } else {
      showError(`Operation failed: ${msg}`);
    }
  }
}

// ── Delete ───────────────────────────────────────────────────────
// Permanently deletes the file/folder. A confirmation dialog guards the
// irreversible action; the Rust side additionally refuses to delete the
// workspace root or anything outside it.
const deleteConfirmOpen = ref(false);

const deleteConfirmMessage = computed(() =>
  props.entry.is_dir
    ? `Delete folder "${props.entry.name}" and all its contents? This cannot be undone.`
    : `Delete "${props.entry.name}"? This cannot be undone.`
);

function openDeleteConfirm() {
  contextMenu.value = null;
  deleteConfirmOpen.value = true;
}

async function performDelete() {
  deleteConfirmOpen.value = false;
  try {
    await deletePath(props.entry.path, props.rootPath ?? parentOf(props.entry.path));
    showSuccess(`Deleted "${props.entry.name}"`);
    // The parent listing changed — tell FileTree to reload it.
    emit("mutated", parentOf(props.entry.path));
  } catch (err) {
    const msg = String(err);
    if (msg.includes("workspace root")) {
      showError("Cannot delete the workspace root");
    } else if (msg.includes("outside the workspace")) {
      showError("Path is outside the workspace");
    } else {
      showError(`Operation failed: ${msg}`);
    }
  }
}

// ── Open in Finder ───────────────────────────────────────────────
// Folders: open the folder itself. Files: reveal the file in its parent
// folder (macOS `open -R` / Windows `explorer /select`).
function openInFinderHere() {
  contextMenu.value = null;
  const action = props.entry.is_dir
    ? openInFinder(props.entry.path)
    : revealPath(props.entry.path);
  action.catch((err) => {
    showError(`Operation failed: ${String(err)}`);
  });
}

// Cleanup any open menu when this node unmounts (e.g. parent reload).
onBeforeUnmount(() => {
  contextMenu.value = null;
});
</script>

<template>
  <div>
    <button
      @click="handleClick"
      @contextmenu="onContextMenu"
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

      <!-- name (or rename input) -->
      <input
        v-if="renaming"
        ref="renameInputRef"
        v-model="renameValue"
        @keydown="onRenameKey"
        @blur="onRenameBlur"
        class="flex-1 text-[12px] px-1 py-0.5 border border-blue-300 rounded outline-none focus:border-blue-500 bg-white"
      />
      <span v-else class="text-[12px] truncate flex-1">{{ entry.name }}</span>

      <!-- size badge for files -->
      <span v-if="!entry.is_dir && entry.size > 0 && !renaming" class="text-[10px] text-gray-400 ml-1 flex-shrink-0 hidden group-hover:inline">
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
        :root-path="rootPath"
        :mutation-version="mutationVersion"
        @toggle="emit('toggle', $event)"
        @select="emit('select', $event)"
        @mutated="(p) => emit('mutated', p)"
      />

      <!-- Inline new file/folder (children of THIS directory) -->
      <div
        v-if="inlineNew"
        class="flex items-center gap-1 px-2 py-0.5"
        :style="{ paddingLeft: `${8 + (depth + 1) * 16}px` }"
      >
        <span class="w-4 h-4 flex items-center justify-center flex-shrink-0">
          <FolderPlus v-if="inlineNew.kind === 'folder'" :size="11" class="text-gray-400" />
          <FilePlus v-else :size="11" class="text-gray-400" />
        </span>
        <input
          :id="`inline-new-${entry.path}`"
          v-model="inlineNew.value"
          @keydown="onInlineNewKey"
          @blur="onInlineNewBlur"
          :placeholder="'Enter name'"
          class="flex-1 text-[12px] px-1 py-0.5 border border-blue-300 rounded outline-none focus:border-blue-500"
        />
      </div>
      <p
        v-if="inlineNew && inlineNew.error"
        class="text-[10px] text-red-500 py-0.5"
        :style="{ paddingLeft: `${8 + (depth + 1) * 16 + 16}px` }"
      >
        {{ inlineNew.error }}
      </p>

      <div v-if="children.length === 0 && !loadingChildren && !inlineNew" class="text-[11px] text-gray-400 pl-2 py-1" :style="{ paddingLeft: `${8 + (depth + 1) * 16 + 16}px` }">
        (empty)
      </div>
    </div>

    <!-- Context menu (rendered at body via Teleport) -->
    <FileContextMenu
      v-if="contextMenu"
      :x="contextMenu.x"
      :y="contextMenu.y"
      :is-dir="entry.is_dir"
      @new-file="startInlineNew('file')"
      @new-folder="startInlineNew('folder')"
      @rename="startRename"
      @move-to="openMoveDialog"
      @open-in-finder="openInFinderHere"
      @delete="openDeleteConfirm"
      @close="closeContextMenu"
    />

    <!-- Move dialog -->
    <MoveToDialog
      :show="moveDialogOpen"
      :source-path="entry.path"
      :source-is-dir="entry.is_dir"
      :root-path="rootPath ?? parentOf(entry.path)"
      @confirm="handleMoveConfirm"
      @cancel="moveDialogOpen = false"
    />

    <!-- Delete confirm -->
    <ConfirmDialog
      :show="deleteConfirmOpen"
      :title="'Delete file?'"
      :message="deleteConfirmMessage"
      :cancel-text="'Cancel'"
      :confirm-text="'Delete'"
      @confirm="performDelete"
      @cancel="deleteConfirmOpen = false"
    />
  </div>
</template>
