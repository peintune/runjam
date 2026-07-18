<script setup lang="ts">
import { computed, ref, onMounted, onBeforeUnmount } from "vue";
import { useRouter, useRoute } from "vue-router";
import { useWorkspaceStore, type Session } from "../stores/useWorkspaceStore";
import { Settings, BarChart3, Plus, Folder, ChevronRight, LayoutGrid, CheckSquare } from "lucide-vue-next";
import SessionItem from "./SessionItem.vue";

const store = useWorkspaceStore();
const router = useRouter();
const route = useRoute();

const viewMode = ref<"comfortable" | "compact">("comfortable");
const showArchived = ref(false);
const collapsedConversations = ref(false);
const confirmDeleteTimer = ref<ReturnType<typeof setTimeout> | null>(null);
const confirmDeleteMode = ref(false);
const batchMode = ref(false);
const selectedIds = ref<Set<string>>(new Set());

const activeSessions = computed(() => store.sessions.filter(s => !s.archived));
const archivedSessions = computed(() => store.sessions.filter(s => s.archived));

const grouped = computed(() => {
  const dirMap = new Map<string, { path: string; sessions: Session[] }>();
  const orphans: Session[] = [];
  for (const s of activeSessions.value) {
    if (s.directoryId) {
      const dir = store.directories.find((d) => d.id === s.directoryId);
      const key = dir?.path ?? s.directoryId;
      if (!dirMap.has(key)) dirMap.set(key, { path: key, sessions: [] });
      dirMap.get(key)!.sessions.push(s);
    } else { orphans.push(s); }
  }
  for (const [_, g] of dirMap) {
    g.sessions.sort((a, b) => (b.pinned ? 1 : 0) - (a.pinned ? 1 : 0) || new Date(b.lastActiveAt).getTime() - new Date(a.lastActiveAt).getTime());
  }
  orphans.sort((a, b) => (b.pinned ? 1 : 0) - (a.pinned ? 1 : 0) || new Date(b.lastActiveAt).getTime() - new Date(a.lastActiveAt).getTime());
  return { groups: [...dirMap.values()], orphans };
});

const expandedDirs = ref<Set<string>>(new Set());
function toggleDir(path: string) {
  if (expandedDirs.value.has(path)) expandedDirs.value.delete(path);
  else expandedDirs.value.add(path);
}

// context menu
const menuSessionId = ref<string | null>(null);
const menuX = ref(0);
const menuY = ref(0);
const renameId = ref<string | null>(null);

function showMenu(e: MouseEvent, id: string) {
  menuX.value = e.clientX;
  menuY.value = e.clientY;
  menuSessionId.value = id;
}
function closeMenu() { menuSessionId.value = null; }
function onGlobalClick(e: MouseEvent) {
  const target = e.target as HTMLElement;
  if (target.closest('[data-menu]')) return;
  closeMenu();
}
onMounted(() => document.addEventListener('click', onGlobalClick));
onBeforeUnmount(() => document.removeEventListener('click', onGlobalClick));

function startRename(id: string) { renameId.value = id; closeMenu(); }
function doRename(text: string) {
  if (renameId.value && text.trim()) { store.setSessionTitle(renameId.value, text.trim()); }
  renameId.value = null;
}

function toggleSelect(id: string) {
  const next = new Set(selectedIds.value);
  if (next.has(id)) next.delete(id); else next.add(id);
  selectedIds.value = next;
  if (next.size === 0) batchMode.value = false;
}
function exitBatch() { batchMode.value = false; selectedIds.value = new Set(); }
function batchDeleteSelected() { store.batchDelete([...selectedIds.value]); exitBatch(); }
function batchPinSelected() { store.batchPin([...selectedIds.value]); exitBatch(); }
function batchArchiveSelected() { [...selectedIds.value].forEach(id => store.archiveSession(id)); exitBatch(); }
function confirmDeleteAllArchived() {
  if (confirmDeleteMode.value) {
    store.batchDeleteAllArchived();
    confirmDeleteMode.value = false;
    if (confirmDeleteTimer.value) { clearTimeout(confirmDeleteTimer.value); confirmDeleteTimer.value = null; }
  } else {
    confirmDeleteMode.value = true;
    confirmDeleteTimer.value = setTimeout(() => { confirmDeleteMode.value = false; }, 3000);
  }
}
</script>

<template>
  <aside class="w-full flex-shrink-0 flex flex-col select-none rounded-2xl bg-white border border-gray-100 shadow-sm" @click="menuSessionId = null">
    <!-- header -->
    <div class="px-5 pt-4 pb-2">
      <div class="flex items-center justify-between">
        <div class="flex items-center gap-3">
          <img src="/runjam-logo.svg" alt="RunJam" class="w-9 h-9 rounded-xl" />
          <span class="text-[17px] font-semibold text-gray-900 tracking-tight">Run<span style="color: #10b981">Jam</span></span>
        </div>
        <div class="flex items-center gap-1">
          <button v-if="batchMode" @click="exitBatch" class="p-1.5 rounded-lg text-[11px] font-medium text-gray-500 hover:bg-gray-100 transition-colors cursor-pointer">Cancel</button>
          <button v-else @click="batchMode = true" class="p-1.5 rounded-lg text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors cursor-pointer" title="Select sessions">
            <CheckSquare :size="15" />
          </button>
          <button @click="viewMode = viewMode === 'comfortable' ? 'compact' : 'comfortable'" class="p-1.5 rounded-lg text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors cursor-pointer" :title="viewMode === 'comfortable' ? 'Compact view' : 'Comfortable view'">
            <LayoutGrid :size="15" />
          </button>
        </div>
      </div>
    </div>

    <!-- batch action bar -->
    <div v-if="batchMode && selectedIds.size > 0" class="px-3 pb-2">
      <div class="flex items-center gap-1 bg-gray-100 rounded-xl px-2 py-1.5">
        <span class="text-[12px] text-gray-500 ml-2 mr-auto">{{ selectedIds.size }} selected</span>
        <button @click="batchPinSelected" class="px-2.5 py-1 rounded-lg text-[12px] font-medium text-gray-600 hover:bg-gray-200 transition-colors cursor-pointer">Pin</button>
        <button @click="batchArchiveSelected" class="px-2.5 py-1 rounded-lg text-[12px] font-medium text-gray-600 hover:bg-gray-200 transition-colors cursor-pointer">Archive</button>
        <button @click="batchDeleteSelected" class="px-2.5 py-1 rounded-lg text-[12px] font-medium text-red-500 hover:bg-red-100 transition-colors cursor-pointer">Delete</button>
      </div>
    </div>

    <!-- new session -->
    <div class="px-3 pb-3">
      <button
        @click="store.activeSessionId = null"
        class="w-full flex items-center justify-center gap-2 px-4 py-2.5 rounded-xl text-[13px] font-semibold text-white bg-gray-800 hover:bg-gray-900 active:scale-[0.98] transition-all duration-150 cursor-pointer shadow-sm"
      >
        <Plus :size="17" /> New Session
      </button>
    </div>

    <div class="mx-5 border-t border-gray-100" />

    <!-- tree -->
    <div class="flex-1 overflow-y-auto px-2.5 py-3 space-y-4" @click="menuSessionId = null">
      <!-- batch mode hint -->
      <p v-if="batchMode && selectedIds.size === 0" class="px-3 py-8 text-center text-[12px] text-gray-400">
        Click sessions to select
      </p>

      <template v-for="g in grouped.groups" :key="g.path" v-if="!batchMode || true">
        <div>
          <button
            @click="toggleDir(g.path)"
            class="w-full flex items-center gap-2 px-3 py-1.5 rounded-lg text-[12px] text-gray-400 hover:text-gray-600 hover:bg-gray-50 transition-colors cursor-pointer"
          >
            <ChevronRight :size="13" class="transition-transform duration-150 flex-shrink-0" :class="{ 'rotate-90': expandedDirs.has(g.path) }" />
            <Folder :size="14" class="text-gray-700 flex-shrink-0" />
            <span class="truncate font-medium">{{ g.path.split('/').pop() || g.path }}</span>
            <span class="text-[11px] text-gray-400 ml-auto">{{ g.sessions.length }}</span>
          </button>
          <div v-if="expandedDirs.has(g.path)" class="mt-0.5">
            <SessionItem
              v-for="s in g.sessions" :key="s.id" :session="s"
              :active="store.activeSessionId === s.id"
              :compact="viewMode === 'compact'"
              :batch="batchMode" :selected="selectedIds.has(s.id)"
              :renaming="renameId === s.id"
              :archived="false" :menuOpen="menuSessionId === s.id" :menuX="menuX" :menuY="menuY"
              @select="store.selectSession(s.id)"
              @toggle-select="toggleSelect(s.id)"
              @show-menu="(e: MouseEvent) => showMenu(e, s.id)"
              @start-rename="startRename(s.id)"
              @do-rename="doRename"
              @pin="store.togglePin(s.id); menuSessionId = null"
              @archive="store.archiveSession(s.id); menuSessionId = null"
              @delete="store.removeSession(s.id); menuSessionId = null"
            />
          </div>
        </div>
      </template>

      <div v-if="grouped.orphans.length > 0">
        <button @click="collapsedConversations = !collapsedConversations" class="w-full flex items-center gap-2 px-3 py-1.5 rounded-lg text-[12px] text-gray-400 hover:text-gray-600 hover:bg-gray-50 transition-colors cursor-pointer">
          <ChevronRight :size="13" class="transition-transform duration-150 flex-shrink-0" :class="{ 'rotate-90': !collapsedConversations }" />
          <span class="text-[11px] font-semibold text-gray-400 uppercase tracking-wider">Conversations</span>
          <span class="text-[11px] text-gray-400 ml-auto">{{ grouped.orphans.length }}</span>
        </button>
        <div v-if="!collapsedConversations" :class="viewMode === 'compact' ? 'space-y-0 mt-0.5' : 'space-y-0.5 mt-0.5'">
          <SessionItem
            v-for="s in grouped.orphans" :key="s.id" :session="s"
            :active="store.activeSessionId === s.id"
            :compact="viewMode === 'compact'"
            :batch="batchMode" :selected="selectedIds.has(s.id)"
            :renaming="renameId === s.id"
            :archived="false" :menuOpen="menuSessionId === s.id" :menuX="menuX" :menuY="menuY"
            @select="store.selectSession(s.id)"
            @toggle-select="toggleSelect(s.id)"
            @show-menu="(e: MouseEvent) => showMenu(e, s.id)"
            @start-rename="startRename(s.id)"
            @do-rename="doRename"
            @pin="store.togglePin(s.id); menuSessionId = null"
            @archive="store.archiveSession(s.id); menuSessionId = null"
            @delete="store.removeSession(s.id); menuSessionId = null"
          />
        </div>
      </div>

      <!-- Archived section -->
      <div v-if="archivedSessions.length > 0" class="pt-2">
        <button
          @click="showArchived = !showArchived"
          class="w-full flex items-center gap-2 px-3 py-1.5 rounded-lg text-[12px] text-gray-400 hover:text-gray-600 hover:bg-gray-50 transition-colors cursor-pointer"
        >
          <ChevronRight :size="13" class="transition-transform duration-150 flex-shrink-0" :class="{ 'rotate-90': showArchived }" />
          <Archive :size="13" class="flex-shrink-0" />
          <span class="font-medium">Archived</span>
          <span class="text-[11px] text-gray-400 ml-auto mr-2">{{ archivedSessions.length }}</span>
          <button v-if="showArchived" @click.stop="confirmDeleteAllArchived" :class="confirmDeleteMode ? 'rounded-md text-[11px] bg-red-500 text-white hover:bg-red-600 cursor-pointer px-2 py-0.5' : 'p-1 rounded-md text-[11px] text-red-400 hover:bg-red-50 hover:text-red-500 transition-colors cursor-pointer'" :title="confirmDeleteMode ? 'Click again to confirm' : 'Delete all archived'">{{ confirmDeleteMode ? 'Sure?' : '🗑' }}</button>
        </button>
        <div v-if="showArchived" class="mt-0.5">
          <SessionItem
            v-for="s in archivedSessions" :key="s.id" :session="s"
            :active="false" :compact="true" :archived="true"
            :batch="batchMode" :selected="selectedIds.has(s.id)"
            :renaming="false"
            :menuOpen="menuSessionId === s.id" :menuX="menuX" :menuY="menuY"
            @select="batchMode ? toggleSelect(s.id) : store.selectSession(s.id)"
            @toggle-select="toggleSelect(s.id)"
            @show-menu="(e: MouseEvent) => showMenu(e, s.id)"
            @unarchive="store.unarchiveSession(s.id); menuSessionId = null"
            @delete="store.removeSession(s.id); menuSessionId = null"
          />
        </div>
      </div>

      <div v-if="store.sessions.length === 0" class="flex flex-col items-center justify-center py-16 text-gray-300">
        <Folder :size="32" class="mb-3 opacity-30" />
        <p class="text-[13px] text-gray-400">No sessions yet</p>
        <p class="text-[12px] text-gray-300 mt-1">Start a conversation to begin</p>
      </div>
    </div>

    <!-- footer -->
    <div class="border-t border-gray-100 px-3 py-3 space-y-0.5">
      <button @click="router.push('/costs')" :class="['w-full flex items-center gap-3 px-3.5 py-2.5 rounded-xl text-[13px] transition-all duration-150 cursor-pointer', route.path === '/costs' ? 'bg-gray-100 text-gray-900 font-medium' : 'text-gray-500 hover:bg-gray-50 hover:text-gray-700']">
        <BarChart3 :size="17" /> Costs
      </button>
      <button @click="router.push('/settings')" :class="['w-full flex items-center gap-3 px-3.5 py-2.5 rounded-xl text-[13px] transition-all duration-150 cursor-pointer', route.path === '/settings' ? 'bg-gray-100 text-gray-900 font-medium' : 'text-gray-500 hover:bg-gray-50 hover:text-gray-700']">
        <Settings :size="17" /> Settings
      </button>
    </div>
  </aside>
</template>
