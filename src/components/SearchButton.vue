<script setup lang="ts">
import { ref, watch, nextTick } from "vue";
import { Search, X, Clock } from "lucide-vue-next";
import { searchConversations, type SearchResult } from "../api/search";
import { useWorkspaceStore } from "../stores/useWorkspaceStore";

const wsStore = useWorkspaceStore();
const open = ref(false);
const query = ref("");
const results = ref<SearchResult[]>([]);
const searching = ref(false);
const inputRef = ref<HTMLInputElement | null>(null);
let timer: ReturnType<typeof setTimeout> | null = null;

watch(open, async (val) => {
  if (val) {
    await nextTick();
    inputRef.value?.focus();
  }
});

function doSearch() {
  if (timer) clearTimeout(timer);
  const q = query.value.trim();
  if (!q) { results.value = []; return; }
  searching.value = true;
  timer = setTimeout(async () => {
    try { results.value = await searchConversations(q); } catch { results.value = []; }
    searching.value = false;
  }, 200);
}

function selectSession(id: string) {
  wsStore.selectSession(id);
  open.value = false;
  query.value = "";
  results.value = [];
}
</script>

<template>
  <div class="relative" style="-webkit-app-region: no-drag">
    <button @click="open = !open" class="p-1.5 rounded-lg text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors duration-150">
      <Search :size="16" />
    </button>

    <!-- centered overlay -->
    <Teleport to="body">
      <div v-if="open" class="fixed inset-0 z-50 flex items-start justify-center pt-[15vh]" @click.self="open = false; query = ''; results = []">
        <div class="w-[560px] bg-white rounded-2xl border border-gray-200 shadow-xl">
          <div class="flex items-center gap-3 px-4 py-3 border-b border-gray-100">
            <Search :size="17" class="text-gray-400 flex-shrink-0" />
            <input ref="inputRef" v-model="query" @input="doSearch" placeholder="Search conversations..."
              class="flex-1 bg-transparent border-none outline-none text-[15px] text-gray-900 placeholder-gray-400" />
            <button @click="open = false; query = ''; results = []" class="text-gray-400 hover:text-gray-600 flex-shrink-0 p-1">
              <X :size="16" />
            </button>
          </div>

          <div class="max-h-[50vh] overflow-y-auto">
            <div v-if="searching" class="px-4 py-12 text-center text-[13px] text-gray-400">Searching...</div>

            <template v-else-if="query && results.length > 0">
              <button v-for="r in results" :key="r.session_id + r.content.substring(0,20)"
                @click="selectSession(r.session_id)"
                class="w-full text-left px-4 py-3 hover:bg-gray-50 transition-colors border-b border-gray-50 last:border-b-0">
                <div class="flex items-center gap-2 mb-1">
                  <span class="text-[11px] font-medium text-gray-400 bg-gray-100 rounded px-1.5 py-0.5">{{ r.role === 'user' ? 'You' : 'Agent' }}</span>
                  <span class="text-[11px] text-gray-400">{{ wsStore.sessions.find(s => s.id === r.session_id)?.cliDisplayName || 'Session' }}</span>
                </div>
                <p class="text-[13px] text-gray-700 leading-relaxed line-clamp-3">{{ r.content }}</p>
              </button>
            </template>

            <div v-else-if="query && results.length === 0" class="px-4 py-12 text-center text-[13px] text-gray-400">No results</div>

            <template v-else>
              <p class="px-4 pt-3 pb-1 text-[11px] font-semibold text-gray-400 uppercase tracking-wider">Recent</p>
              <button v-for="s in wsStore.sessions.slice().reverse().slice(0, 12)" :key="s.id"
                @click="selectSession(s.id)"
                class="w-full text-left px-4 py-2.5 hover:bg-gray-50 transition-colors flex items-center gap-3">
                <Clock :size="14" class="text-gray-400 flex-shrink-0" />
                <span class="text-[14px] text-gray-700 truncate flex-1">{{ s.cliDisplayName }}</span>
                <span class="text-[11px] text-gray-400 flex-shrink-0">{{ s.createdAt ? new Date(s.createdAt).toLocaleDateString() : '' }}</span>
              </button>
              <div v-if="wsStore.sessions.length === 0" class="px-4 py-12 text-center text-[13px] text-gray-400">
                No sessions yet
              </div>
            </template>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>
