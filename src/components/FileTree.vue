<script setup lang="ts">
import { ref, watch, computed, onBeforeUnmount, onMounted, nextTick } from "vue";
import { listDir, searchFiles, createFile, createDir, type FileEntry, type FileSearchResult } from "../api/fs";
import FileTreeNode from "./FileTreeNode.vue";
import {
  Folder,
  RefreshCw,
  ExternalLink,
  Search,
  X,
  File,
  Plus,
  FilePlus,
  FolderPlus,
} from "lucide-vue-next";
import { openInFinder } from "../api/app";
import { useToast } from "../composables/useToast";
import FileContextMenu from "./FileContextMenu.vue";

const props = defineProps<{
  rootPath: string;
}>();

const emit = defineEmits<{
  (e: "select-file", path: string): void;
}>();

const { showSuccess } = useToast();

const entries = ref<FileEntry[]>([]);
const expanded = ref<Set<string>>(new Set());
const loading = ref(false);
const selectedPath = ref<string | null>(null);

// Per-directory tree state cache. Without this, switching sessions remounts the
// whole FileTree (via :key) and re-scans the directory from disk, collapsing
// every expanded folder. Caching lets us restore a previously-viewed directory
// instantly — folders stay expanded and already-loaded children are reused.
interface DirTreeState {
  entries: FileEntry[];
  expanded: Set<string>;
  selectedPath: string | null;
}
const dirCache = new Map<string, DirTreeState>();

// Cache of loaded subdirectory contents, keyed by directory path. FileTreeNode
// instances are recreated when switching directories (different :key paths), so
// their local `children` refs would otherwise be lost. This module-level cache
// lets a recreated expanded folder restore its already-loaded children instantly
// instead of re-scanning the subdirectory from disk.
const childrenCache = new Map<string, FileEntry[]>();

/** Bumped on every write operation (create/rename/move/delete). Passed down to
 *  FileTreeNode so an already-mounted expanded folder re-reads its children
 *  from disk and reflects the change immediately, instead of relying on the
 *  (now stale) cached listing. */
const mutationVersion = ref(0);

/** Return cached children for a directory, or load + cache them from disk.
 *  Pass `force = true` after a write operation (create/rename/move) so the
 *  stale cache is bypassed and the fresh listing is returned. */
async function resolveChildren(dirPath: string, force = false): Promise<FileEntry[]> {
  if (!force) {
    const cached = childrenCache.get(dirPath);
    if (cached) return cached;
  }
  const loaded = await listDir(dirPath);
  childrenCache.set(dirPath, loaded);
  return loaded;
}

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

// ── "+" dropdown state ──────────────────────────────────────────
const showNewMenu = ref(false);
const newMenuRef = ref<HTMLDivElement | null>(null);

function onNewPointerDown(e: PointerEvent) {
  if (newMenuRef.value && !newMenuRef.value.contains(e.target as Node)) {
    showNewMenu.value = false;
  }
}
onMounted(() => document.addEventListener("pointerdown", onNewPointerDown));
onBeforeUnmount(() => document.removeEventListener("pointerdown", onNewPointerDown));

// ── Root blank-area context menu ────────────────────────────────
// Right-clicking the empty area of the tree (not a specific entry) offers the
// same create actions as the toolbar "+" button, scoped to the workspace root.
const rootContextMenu = ref<{ x: number; y: number } | null>(null);

function onRootContextMenu(e: MouseEvent) {
  e.preventDefault();
  rootContextMenu.value = { x: e.clientX, y: e.clientY };
}

function closeRootContextMenu() {
  rootContextMenu.value = null;
}

function getIconClass(ext: string) {
  const map: Record<string, string> = {
    // Code
    ts: "text-blue-500",
    tsx: "text-cyan-500",
    js: "text-yellow-500",
    jsx: "text-cyan-500",
    vue: "text-emerald-500",
    rs: "text-orange-500",
    py: "text-blue-400",
    go: "text-cyan-400",
    java: "text-red-500",
    c: "text-blue-600",
    cpp: "text-blue-600",
    rb: "text-red-500",
    php: "text-indigo-500",
    swift: "text-orange-500",
    kt: "text-purple-500",
    scala: "text-red-500",
    sh: "text-green-600",
    bash: "text-green-600",
    zsh: "text-green-600",
    css: "text-pink-500",
    scss: "text-pink-500",
    less: "text-pink-500",
    html: "text-orange-400",
    sql: "text-blue-400",
    // Data / config
    json: "text-yellow-400",
    yaml: "text-red-400",
    yml: "text-red-400",
    toml: "text-gray-400",
    xml: "text-green-500",
    ini: "text-gray-500",
    // Text / docs
    md: "text-blue-400",
    txt: "text-gray-500",
    log: "text-gray-400",
    env: "text-yellow-600",
    // Office
    docx: "text-blue-600",
    doc: "text-blue-600",
    xlsx: "text-green-600",
    xls: "text-green-600",
    csv: "text-green-600",
    pptx: "text-orange-600",
    ppt: "text-orange-600",
    // Images
    svg: "text-purple-500",
    png: "text-pink-400",
    jpg: "text-pink-400",
    jpeg: "text-pink-400",
    gif: "text-pink-400",
    webp: "text-pink-400",
    ico: "text-pink-400",
    // Archives
    zip: "text-amber-500",
    tar: "text-amber-500",
    gz: "text-amber-500",
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

/** Manual refresh: drop the cached tree for the current directory and reload
 *  from disk so newly added/removed files show up. */
function refreshTree() {
  if (!props.rootPath) return;
  dirCache.delete(props.rootPath);
  childrenCache.clear();
  expanded.value = new Set();
  selectedPath.value = null;
  loadEntries();
}

/** Drop cached state for `dirPath` (and the root if it matches) and reload
 *  the relevant listing. Used after write operations so the UI shows the
 *  new/renamed/moved entry without a full refresh. */
async function invalidateAndReload(dirPath: string) {
  childrenCache.delete(dirPath);
  // If we touch the root, also drop the per-root snapshot so the new entry
  // shows up in the top-level listing.
  if (dirPath === props.rootPath) {
    dirCache.delete(props.rootPath);
    expanded.value = new Set();
    selectedPath.value = null;
    await loadEntries();
    return;
  }
  // If this dir is currently expanded, force a reload of its children.
  if (expanded.value.has(dirPath)) {
    try {
      const reloaded = await listDir(dirPath);
      childrenCache.set(dirPath, reloaded);
    } catch (err) {
      console.error("Failed to reload dir after mutation:", err);
    }
  }
}

function toggleExpand(path: string) {
  if (expanded.value.has(path)) {
    expanded.value.delete(path);
  } else {
    expanded.value.add(path);
  }
}

/** Child node mutated (renamed/moved/created/deleted). Refresh the right
 *  directory's cache. Path = the parent directory that was affected. */
function onNodeMutated(parentPath: string) {
  mutationVersion.value++;
  invalidateAndReload(parentPath);
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

// ── "+" menu: create at root ─────────────────────────────────────

/** Prompt the user for a name with a small inline input. Returns the entered
 *  name or null if cancelled. The prompt is appended as a transient row at
 *  the bottom of the entry list. */
const inlineCreate = ref<{
  kind: "file" | "folder";
  parentPath: string;
  value: string;
  error: string | null;
} | null>(null);

function startInlineCreate(kind: "file" | "folder", parentPath: string) {
  inlineCreate.value = { kind, parentPath, value: "", error: null };
  showNewMenu.value = false;
  nextTick(() => {
    const input = document.getElementById("inline-create-input") as HTMLInputElement | null;
    input?.focus();
  });
}

async function commitInlineCreate() {
  if (!inlineCreate.value) return;
  // Local handle — we null out `inlineCreate.value` on success and the
  // catch block still wants to write to `.error` on the original object.
  const ctx = inlineCreate.value;
  const { kind, parentPath, value } = ctx;
  const name = value.trim();
  if (!name) {
    inlineCreate.value = null;
    return;
  }
  // Reject path separators — these would create nested paths we didn't intend.
  if (name.includes("/") || name.includes("\\")) {
    ctx.error = `Invalid name: "${name}" contains a path separator`;
    return;
  }
  const target = parentPath === "/" ? `/${name}` : `${parentPath}/${name}`;
  try {
    if (kind === "file") {
      await createFile(target, props.rootPath);
    } else {
      await createDir(target, props.rootPath);
    }
    showSuccess(kind === "file" ? `Created ${name}` : `Created ${name}/`);
    inlineCreate.value = null;
    await invalidateAndReload(parentPath);
  } catch (err) {
    const msg = String(err);
    if (msg.includes("already exists")) {
      ctx.error = `"${name}" already exists`;
    } else {
      ctx.error = `Operation failed: ${msg}`;
    }
  }
}

function cancelInlineCreate() {
  inlineCreate.value = null;
}

function onInlineKey(e: KeyboardEvent) {
  if (e.key === "Enter") {
    e.preventDefault();
    commitInlineCreate();
  } else if (e.key === "Escape") {
    e.preventDefault();
    cancelInlineCreate();
  }
}

/** If the user clicks away, commit (treat as "OK"). Esc cancels. We commit
 *  on blur rather than cancel because the typed name is usually intentional
 *  — losing it on a stray click would be frustrating. */
function onInlineBlur() {
  // Defer so a click on a sibling button can register first.
  setTimeout(() => {
    if (inlineCreate.value && inlineCreate.value.value.trim()) {
      commitInlineCreate();
    } else if (inlineCreate.value) {
      cancelInlineCreate();
    }
  }, 150);
}

watch(() => props.rootPath, (newPath, oldPath) => {
  // Save current state to cache before switching away
  if (oldPath) {
    dirCache.set(oldPath, {
      entries: entries.value,
      expanded: expanded.value,
      selectedPath: selectedPath.value,
    });
  }
  // Search results belong to the previous directory — always clear on switch.
  clearSearch();
  inlineCreate.value = null;
  // Restore cached state if we've seen this dir before — instant, folders stay
  // expanded and already-loaded children are reused (no re-scan).
  const cached = newPath ? dirCache.get(newPath) : undefined;
  if (cached) {
    entries.value = cached.entries;
    expanded.value = cached.expanded;
    selectedPath.value = cached.selectedPath;
    loading.value = false;
  } else {
    entries.value = [];
    expanded.value = new Set();
    selectedPath.value = null;
    loadEntries();
  }
}, { immediate: true });
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
        <!-- "+" dropdown -->
        <div ref="newMenuRef" class="relative">
          <button
            @click="showNewMenu = !showNewMenu"
            class="p-1 rounded-md text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors flex-shrink-0"
            :title="'New File'"
          >
            <Plus :size="13" />
          </button>
          <div
            v-if="showNewMenu"
            class="absolute right-0 top-full mt-1 z-30 min-w-[160px] bg-white rounded-lg shadow-lg border border-gray-200 py-1"
          >
            <button
              class="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-gray-50 text-[12px] text-gray-700 text-left"
              @click="startInlineCreate('file', rootPath)"
            >
              <FilePlus :size="13" class="text-gray-500" />
              <span>New File</span>
            </button>
            <button
              class="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-gray-50 text-[12px] text-gray-700 text-left"
              @click="startInlineCreate('folder', rootPath)"
            >
              <FolderPlus :size="13" class="text-gray-500" />
              <span>New Folder</span>
            </button>
          </div>
        </div>
        <button
          @click="openInFinder(props.rootPath)"
          class="p-1 rounded-md text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors flex-shrink-0"
          :title="'Open in Finder'"
        >
          <ExternalLink :size="13" />
        </button>
        <button
          @click="refreshTree"
          class="p-1 rounded-md text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors flex-shrink-0"
          :title="'Refresh'"
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
          :placeholder="'Search files...'"
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
    <div class="flex-1 overflow-y-auto py-1" @contextmenu="onRootContextMenu">
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
        <div v-else-if="entries.length === 0 && !inlineCreate" class="flex flex-col items-center justify-center py-12 text-gray-300">
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
            :resolve-children="resolveChildren"
            :root-path="rootPath"
            :mutation-version="mutationVersion"
            @toggle="toggleExpand"
            @select="handleFileClick"
            @mutated="onNodeMutated"
          />
          <!-- Inline create row (only at root level for the toolbar "+" entry point) -->
          <div
            v-if="inlineCreate && inlineCreate.parentPath === rootPath"
            class="flex items-center gap-1 px-2 py-0.5"
            :style="{ paddingLeft: '8px' }"
          >
            <span class="w-4 h-4 flex items-center justify-center flex-shrink-0">
              <FolderPlus v-if="inlineCreate.kind === 'folder'" :size="12" class="text-gray-400" />
              <FilePlus v-else :size="12" class="text-gray-400" />
            </span>
            <input
              id="inline-create-input"
              v-model="inlineCreate.value"
              @keydown="onInlineKey"
              @blur="onInlineBlur"
              :placeholder="'Enter name'"
              class="flex-1 text-[12px] px-1 py-0.5 border border-blue-300 rounded outline-none focus:border-blue-500"
            />
          </div>
          <p
            v-if="inlineCreate && inlineCreate.error && inlineCreate.parentPath === rootPath"
            class="px-3 py-0.5 text-[10px] text-red-500"
          >
            {{ inlineCreate.error }}
          </p>
        </template>
      </template>
    </div>

    <!-- Root blank-area context menu: New File / New Folder at the root -->
    <FileContextMenu
      v-if="rootContextMenu"
      :x="rootContextMenu.x"
      :y="rootContextMenu.y"
      :is-dir="true"
      :root-mode="true"
      @new-file="startInlineCreate('file', rootPath)"
      @new-folder="startInlineCreate('folder', rootPath)"
      @close="closeRootContextMenu"
    />
  </div>
</template>
