<script setup lang="ts">
import { ref, watch, onMounted } from "vue";
import { X, ChevronRight, Folder, FolderOpen, CornerDownRight } from "lucide-vue-next";
import { listDir } from "../api/fs";

const props = defineProps<{
  show: boolean;
  /** The path we're moving. Used to (a) pre-select the current parent and
   *  (b) forbid selecting this path or any of its descendants. */
  sourcePath: string;
  sourceIsDir: boolean;
  /** Workspace root — the dialog only browses within this tree. */
  rootPath: string;
}>();

const emit = defineEmits<{
  (e: "confirm", destDir: string): void;
  (e: "cancel"): void;
}>();

interface DirNode {  name: string;
  path: string;
  loaded: boolean;
  loading: boolean;
  children: DirNode[];
  expanded: boolean;
}

const root = ref<DirNode | null>(null);
const selected = ref<string | null>(null);
// Pre-expand the source's parent so the user can confirm they're moving
// "next to" the current location.
const initiallyExpanded = ref<Set<string>>(new Set());

// Track forbidden paths: the source itself plus all descendants if it's a
// directory. Loading children of a forbidden dir is wasteful, so we block it.
const forbiddenPaths = ref<Set<string>>(new Set());

watch(
  () => props.show,
  async (open) => {
    if (open) {
      forbiddenPaths.value = computeForbidden(props.sourcePath, props.sourceIsDir);
      selected.value = parentPath(props.sourcePath);
      initiallyExpanded.value = new Set();
      // Walk up from source so the user sees the surrounding context.
      let p = parentPath(props.sourcePath);
      while (p && p !== props.rootPath && p.startsWith(props.rootPath)) {
        initiallyExpanded.value.add(p);
        p = parentPath(p);
      }
      root.value = await loadDirNode(props.rootPath, true);
    }
  },
  { immediate: true }
);

function parentPath(p: string): string {
  const idx = p.lastIndexOf("/");
  return idx > 0 ? p.slice(0, idx) : "/";
}

function computeForbidden(source: string, isDir: boolean): Set<string> {
  const out = new Set<string>([source]);
  if (!isDir) return out;
  // We don't walk the whole subtree up front — instead, when the user expands
  // a forbidden dir, we refuse. The check is also enforced at submit time by
  // Rust's rename_path, so the frontend filter is just UX.
  return out;
}

async function loadDirNode(path: string, forceExpand = false): Promise<DirNode> {
  const entries = await listDir(path);
  const dirs: DirNode[] = entries
    .filter((e) => e.is_dir)
    .map((e) => ({
      name: e.name,
      path: e.path,
      loaded: false,
      loading: false,
      children: [],
      expanded: forceExpand && initiallyExpanded.value.has(e.path),
    }));
  // Auto-expand any node the user "should see" — the source's parent chain.
  const node: DirNode = {
    name: path === props.rootPath ? "" : path.split("/").pop() || path,
    path,
    loaded: true,
    loading: false,
    children: dirs,
    expanded: forceExpand,
  };
  // Eagerly load children of nodes that should start expanded.
  for (const child of dirs) {
    if (child.expanded) {
      child.children = (await loadDirNode(child.path)).children;
      child.loaded = true;
    }
  }
  return node;
}

async function toggle(node: DirNode) {
  if (forbiddenPaths.value.has(node.path)) return; // can't enter
  if (!node.loaded) {
    node.loading = true;
    try {
      const loaded = await loadDirNode(node.path);
      node.children = loaded.children;
      node.loaded = true;
    } finally {
      node.loading = false;
    }
  }
  node.expanded = !node.expanded;
}

function pick(node: DirNode) {
  if (forbiddenPaths.value.has(node.path)) return;
  selected.value = node.path;
}

// Allow clicking the root label to select the workspace root as destination.
const rootLabel = "(root)";

function isSelected(path: string): boolean {
  return selected.value === path;
}

function isForbidden(path: string): boolean {
  if (!forbiddenPaths.value.has(path)) return false;
  return true;
}

function onConfirm() {
  if (selected.value) emit("confirm", selected.value);
}

onMounted(() => {
  // nothing to do — initial load is in the watcher with immediate:true
});
</script>

<template>
  <Teleport to="body">
    <div v-if="show" class="fixed inset-0 z-50 flex items-center justify-center p-4">
      <!-- Backdrop -->
      <div class="absolute inset-0 bg-black/20 backdrop-blur-[2px]" @click="emit('cancel')"></div>

      <div class="relative w-full max-w-sm bg-white rounded-2xl shadow-2xl border border-gray-100 overflow-hidden">
        <!-- Header -->
        <div class="flex items-center justify-between px-5 pt-4 pb-3 border-b border-gray-100">
          <h3 class="text-[14px] font-semibold text-gray-900 flex items-center gap-2">
            <CornerDownRight :size="15" class="text-gray-500" />
            Move to...
          </h3>
          <button
            @click="emit('cancel')"
            class="p-1 rounded-md text-gray-400 hover:text-gray-600 hover:bg-gray-100"
          >
            <X :size="14" />
          </button>
        </div>

        <!-- Tree -->
        <div class="px-3 py-2 max-h-[280px] overflow-y-auto">
          <p class="px-2 py-1 text-[10px] font-semibold text-gray-400 uppercase tracking-wider">
            Select destination folder
          </p>
          <div v-if="root" class="text-[12px]">
            <button
              class="w-full flex items-center gap-1.5 px-2 py-1 rounded text-left hover:bg-blue-50"
              :class="isSelected(root.path) ? 'bg-blue-50 text-blue-700' : 'text-gray-700'"
              @click="pick(root)"
            >
              <Folder :size="13" class="text-gray-500 flex-shrink-0" />
              <span class="truncate">{{ rootLabel }}</span>
            </button>
            <div v-if="root.children.length" class="ml-3 border-l border-gray-100 pl-1">
              <div v-for="child in root.children" :key="child.path">
                <button
                  class="w-full flex items-center gap-1.5 px-2 py-1 rounded text-left hover:bg-gray-50"
                  :class="[
                    isSelected(child.path) ? 'bg-blue-50 text-blue-700' : 'text-gray-700',
                    isForbidden(child.path) ? 'opacity-40 cursor-not-allowed' : '',
                  ]"
                  @click="!isForbidden(child.path) && pick(child)"
                >
                  <span
                    v-if="!isForbidden(child.path)"
                    class="w-3.5 h-3.5 flex items-center justify-center"
                    @click.stop="toggle(child)"
                  >
                    <ChevronRight
                      :size="11"
                      class="transition-transform duration-100 text-gray-400"
                      :class="{ 'rotate-90': child.expanded }"
                    />
                  </span>
                  <span v-else class="w-3.5 h-3.5"></span>
                  <Folder v-if="!child.expanded" :size="12" class="text-gray-500 flex-shrink-0" />
                  <FolderOpen v-else :size="12" class="text-gray-600 flex-shrink-0" />
                  <span class="truncate">{{ child.name }}</span>
                </button>
                <div v-if="child.expanded && child.children.length" class="ml-4 border-l border-gray-100 pl-1">
                  <button
                    v-for="grand in child.children"
                    :key="grand.path"
                    class="w-full flex items-center gap-1.5 px-2 py-1 rounded text-left hover:bg-gray-50"
                    :class="[
                      isSelected(grand.path) ? 'bg-blue-50 text-blue-700' : 'text-gray-700',
                      isForbidden(grand.path) ? 'opacity-40 cursor-not-allowed' : '',
                    ]"
                    @click="!isForbidden(grand.path) && pick(grand)"
                  >
                    <span class="w-3.5 h-3.5"></span>
                    <Folder :size="11" class="text-gray-500 flex-shrink-0" />
                    <span class="truncate">{{ grand.name }}</span>
                  </button>
                  <p v-if="!child.children.length" class="px-2 py-0.5 text-[11px] text-gray-400">(empty)</p>
                </div>
              </div>
            </div>
          </div>
        </div>

        <!-- Footer -->
        <div class="px-5 py-3 bg-gray-50 flex gap-2 border-t border-gray-100">
          <button
            @click="emit('cancel')"
            class="flex-1 px-4 py-2 rounded-lg text-[12px] font-medium text-gray-600 bg-white border border-gray-200 hover:bg-gray-100 transition-colors"
          >
            Cancel
          </button>
          <button
            @click="onConfirm"
            :disabled="!selected"
            class="flex-1 px-4 py-2 rounded-lg text-[12px] font-medium text-white bg-blue-500 hover:bg-blue-600 disabled:bg-gray-300 disabled:cursor-not-allowed transition-colors"
          >
            Move
          </button>
        </div>
      </div>
    </div>
  </Teleport>
</template>
