<script setup lang="ts">
import { ref, computed, watch, nextTick } from "vue";
import { Search, File, Folder, Clock } from "lucide-vue-next";
import { listMentionEntries, searchMentionFiles, type FileEntry } from "../api/fs";

const props = defineProps<{
  cwd: string;
  anchorRect: DOMRect | null;
  /** Live search query synced from the parent textarea (text after @).
   *  Lets the user type the search directly in the textarea without
   *  needing focus to jump to the popup's input. */
  externalQuery?: string;
}>();

const emit = defineEmits<{
  (e: "select", entry: FileEntry): void;
  (e: "close"): void;
}>();

const query = ref("");

// Sync external query (from textarea) → internal query.
watch(
  () => props.externalQuery,
  (val) => {
    if (val !== undefined) {
      query.value = val;
    }
  },
  { immediate: true }
);

const activeTab = ref<"file" | "folder">("file");
const recent = ref<FileEntry[]>([]);
const rootEntries = ref<FileEntry[]>([]);
const searchResults = ref<FileEntry[]>([]);
const loading = ref(false);
const searching = ref(false);
const activeIndex = ref(0);
const inputRef = ref<HTMLInputElement | null>(null);
const listContainer = ref<HTMLElement | null>(null);

// ── Filtering ────────────────────────────────────────────────
const q = computed(() => query.value.trim().toLowerCase());

// ── Data loading with per-cwd cache (60s TTL) ────────────────
const cache = new Map<string, { entries: { recent: FileEntry[]; root: FileEntry[] }; ts: number }>();
const TTL = 60_000;

async function loadData() {
  if (!props.cwd) return;
  const hit = cache.get(props.cwd);
  if (hit && Date.now() - hit.ts < TTL) {
    recent.value = hit.entries.recent;
    rootEntries.value = hit.entries.root;
    return;
  }
  loading.value = true;
  try {
    const r = await listMentionEntries(props.cwd, 5);
    cache.set(props.cwd, { entries: { recent: r.recent, root: r.root }, ts: Date.now() });
    recent.value = r.recent;
    rootEntries.value = r.root;
  } catch (err) {
    console.error("[MentionPicker] Failed to load entries:", err);
  } finally {
    loading.value = false;
  }
}

watch(() => props.cwd, loadData, { immediate: true });

// ── On-demand search (debounced) ─────────────────────────────
// When the user types a search query, call the backend to recursively
// search the project tree. This avoids loading all 2800+ files upfront
// (which hit the old 2000-entry cap and missed files like build.rs).
let searchTimer: ReturnType<typeof setTimeout> | null = null;

watch(q, (queryVal) => {
  if (searchTimer) clearTimeout(searchTimer);
  if (!queryVal) {
    searchResults.value = [];
    searching.value = false;
    return;
  }
  searching.value = true;
  searchTimer = setTimeout(async () => {
    try {
      const results = await searchMentionFiles(props.cwd, queryVal, 100);
      searchResults.value = results;
    } catch (err) {
      console.error("[MentionPicker] Search failed:", err);
      searchResults.value = [];
    } finally {
      searching.value = false;
    }
  }, 200); // 200ms debounce
});

// ── Relative path display ────────────────────────────────────
// Precompute relative directory for each entry so duplicate filenames
// can be disambiguated (e.g. "index.ts" in src/ vs test/).
function relDir(entry: FileEntry): string {
  const cwd = props.cwd;
  if (!cwd) return "";
  let rel = entry.path;
  if (entry.path.startsWith(cwd)) {
    rel = entry.path.slice(cwd.length).replace(/^\/+/, "");
  } else {
    const slash = entry.path.lastIndexOf("/");
    rel = slash > 0 ? entry.path.slice(0, slash) : "";
  }
  const idx = rel.lastIndexOf("/");
  return idx > 0 ? rel.slice(0, idx) : "";
}

// Auto-focus search input when popup opens
watch(
  () => props.anchorRect,
  async (rect) => {
    if (rect) {
      await nextTick();
      inputRef.value?.focus();
    }
  },
  { immediate: true }
);

const filteredRecent = computed(() => {
  if (!q.value) return recent.value;
  return recent.value.filter(
    (e) => e.name.toLowerCase().includes(q.value) || e.path.toLowerCase().includes(q.value)
  );
});

const filteredTab = computed(() => {
  // Browse mode (no query): root-level entries.
  // Search mode (with query): backend search results (recursive).
  const source = q.value ? searchResults.value : rootEntries.value;
  return source.filter((e) => (activeTab.value === "file" ? !e.is_dir : e.is_dir));
});

// Flat list for keyboard navigation: [recent items...] + [tab items...]
// Deduplicate by path
const flatList = computed(() => {
  const seen = new Set<string>();
  const result: FileEntry[] = [];
  for (const e of filteredRecent.value) {
    if (!seen.has(e.path)) {
      seen.add(e.path);
      result.push(e);
    }
  }
  for (const e of filteredTab.value) {
    if (!seen.has(e.path)) {
      seen.add(e.path);
      result.push(e);
    }
  }
  return result;
});

// Reset active index when query or tab changes
watch([q, activeTab], () => {
  activeIndex.value = 0;
});

// ── Keyboard navigation ───────────────────────────────────────
function handleKeydown(e: KeyboardEvent) {
  const list = flatList.value;
  if (e.key === "ArrowDown") {
    e.preventDefault();
    e.stopPropagation();
    activeIndex.value = Math.min(activeIndex.value + 1, list.length - 1);
    scrollIntoView();
  } else if (e.key === "ArrowUp") {
    e.preventDefault();
    e.stopPropagation();
    activeIndex.value = Math.max(activeIndex.value - 1, 0);
    scrollIntoView();
  } else if (e.key === "Enter" || e.key === "Tab") {
    e.preventDefault();
    e.stopPropagation();
    if (list[activeIndex.value]) {
      emit("select", list[activeIndex.value]);
    }
  } else if (e.key === "Escape") {
    e.preventDefault();
    e.stopPropagation();
    emit("close");
  }
}

function scrollIntoView() {
  nextTick(() => {
    const container = listContainer.value;
    if (!container) return;
    const active = container.querySelector("[data-active='true']") as HTMLElement | null;
    if (active) {
      active.scrollIntoView({ block: "nearest" });
    }
  });
}

function selectEntry(entry: FileEntry) {
  emit("select", entry);
}

function close() {
  emit("close");
}

// ── Popup positioning ────────────────────────────────────────
const popupStyle = computed(() => {
  if (!props.anchorRect) return { display: "none" };
  const spaceAbove = props.anchorRect.top;
  const showBelow = spaceAbove < 340;
  const width = Math.min(props.anchorRect.width, 480);
  const left = props.anchorRect.left + (props.anchorRect.width - width) / 2;
  if (showBelow) {
    return {
      position: "fixed" as const,
      left: `${Math.max(8, left)}px`,
      top: `${props.anchorRect.bottom + 6}px`,
      width: `${width}px`,
      maxHeight: "340px",
      zIndex: 60,
    };
  }
  return {
    position: "fixed" as const,
    left: `${Math.max(8, left)}px`,
    bottom: `${window.innerHeight - props.anchorRect.top + 6}px`,
    width: `${width}px`,
    maxHeight: "340px",
    zIndex: 60,
  };
});

defineExpose({ handleKeydown });
</script>

<template>
  <Teleport to="body">
    <div v-if="anchorRect" :style="popupStyle" data-mention-root="true"
      class="bg-white rounded-xl border border-gray-200 shadow-2xl flex flex-col overflow-hidden"
      @click.stop>
      <!-- Search input -->
      <div class="flex items-center gap-2 px-3 py-2 border-b border-gray-100">
        <Search :size="14" class="text-gray-400 flex-shrink-0" />
        <input
          ref="inputRef"
          v-model="query"
          @keydown="handleKeydown"
          placeholder="Search files and folders..."
          class="flex-1 text-[13px] bg-transparent outline-none placeholder:text-gray-400 text-gray-700"
        />
        <button
          @click="close"
          class="text-gray-400 hover:text-gray-600 text-[11px] flex-shrink-0 cursor-pointer"
          title="Close (Esc)"
        >
          Esc
        </button>
      </div>

      <!-- Loading state -->
      <div v-if="loading" class="px-3 py-8 text-center text-[12px] text-gray-400">
        Loading files...
      </div>

      <!-- Content -->
      <template v-else>
        <!-- Scrollable list area -->
        <div ref="listContainer" class="overflow-y-auto flex-1 min-h-0 max-h-[280px]">
          <!-- Recently Modified (shown when no search or filtered matches exist) -->
          <div v-if="filteredRecent.length > 0">
            <div class="flex items-center gap-1.5 px-3 py-1.5 text-[10px] font-semibold text-gray-400 uppercase tracking-wide bg-gray-50/50">
              <Clock :size="10" />
              Recently Modified
            </div>
            <button
              v-for="(entry, i) in filteredRecent"
              :key="'r-' + entry.path"
              @click="selectEntry(entry)"
              @mouseenter="activeIndex = i"
              :data-active="i === activeIndex ? 'true' : 'false'"
              class="w-full flex items-center gap-2 px-3 py-1.5 text-[12px] text-left transition-colors cursor-pointer"
              :class="i === activeIndex ? 'bg-blue-50 text-blue-700' : 'hover:bg-gray-50 text-gray-700'"
            >
              <File v-if="!entry.is_dir" :size="12" class="flex-shrink-0 text-gray-400" />
              <Folder v-else :size="12" class="flex-shrink-0 text-amber-500" />
              <span class="truncate flex-1">{{ entry.name }}</span>
              <span v-if="relDir(entry)" class="text-[10px] text-gray-400 flex-shrink-0 truncate max-w-[110px]" :class="i === activeIndex ? 'text-blue-300' : ''">{{ relDir(entry) }}</span>
              <span class="text-[10px] text-gray-400 flex-shrink-0">{{ entry.modified }}</span>
            </button>
          </div>

          <!-- Divider -->
          <div v-if="filteredRecent.length > 0 && filteredTab.length > 0" class="border-t border-gray-100"></div>

          <!-- File / Folder tabs -->
          <div v-if="filteredTab.length > 0 || !q" class="flex items-center gap-1 px-2 py-1.5 bg-gray-50/50 sticky top-0 z-10 border-b border-gray-100">
            <button
              @click="activeTab = 'file'"
              class="flex items-center gap-1 px-2.5 py-1 rounded-md text-[11px] font-medium transition-colors cursor-pointer"
              :class="activeTab === 'file' ? 'bg-gray-200 text-gray-800' : 'text-gray-400 hover:text-gray-600 hover:bg-gray-100'"
            >
              <File :size="11" /> Files
            </button>
            <button
              @click="activeTab = 'folder'"
              class="flex items-center gap-1 px-2.5 py-1 rounded-md text-[11px] font-medium transition-colors cursor-pointer"
              :class="activeTab === 'folder' ? 'bg-gray-200 text-gray-800' : 'text-gray-400 hover:text-gray-600 hover:bg-gray-100'"
            >
              <Folder :size="11" /> Folders
            </button>
            <span v-if="q" class="ml-auto text-[10px] text-gray-400">
              {{ searching ? 'Searching...' : `${filteredTab.length} matches` }}
            </span>
          </div>

          <!-- Tab list -->
          <div v-if="filteredTab.length > 0">
            <button
              v-for="(entry, i) in filteredTab"
              :key="'t-' + entry.path"
              @click="selectEntry(entry)"
              @mouseenter="activeIndex = filteredRecent.length + i"
              :data-active="filteredRecent.length + i === activeIndex ? 'true' : 'false'"
              class="w-full flex items-center gap-2 px-3 py-1.5 text-[12px] text-left transition-colors cursor-pointer"
              :class="filteredRecent.length + i === activeIndex ? 'bg-blue-50 text-blue-700' : 'hover:bg-gray-50 text-gray-700'"
            >
              <File v-if="!entry.is_dir" :size="12" class="flex-shrink-0 text-gray-400" />
              <Folder v-else :size="12" class="flex-shrink-0 text-amber-500" />
              <span class="truncate flex-1">{{ entry.name }}</span>
              <span v-if="relDir(entry)" class="text-[10px] flex-shrink-0 truncate max-w-[140px]" :class="filteredRecent.length + i === activeIndex ? 'text-blue-300' : 'text-gray-400'">{{ relDir(entry) }}</span>
            </button>
          </div>

          <!-- Browse hint: tell users they can search for deeper files -->
          <div v-if="!q && filteredTab.length > 0" class="px-3 py-1.5 text-[10px] text-gray-400 text-center border-t border-gray-50">
            Type to search all files and folders...
          </div>

          <!-- Empty state -->
          <div v-if="flatList.length === 0 && !loading" class="px-3 py-8 text-center text-[12px] text-gray-400">
            {{ q ? `No matches for "${query}"` : "No files found in this directory" }}
          </div>
        </div>
      </template>
    </div>
  </Teleport>
</template>
