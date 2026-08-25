<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount } from "vue";
import { useRouter } from "vue-router";
import { homeDir } from "@tauri-apps/api/path";
import { Columns3, FolderTree, ChevronRight, Folder, Clock, CalendarClock } from "lucide-vue-next";
import type { TranslationKey } from "../i18n";
import { useWorkspaceStore, type Session } from "../stores/useWorkspaceStore";
import { getCostBySession, type SessionCost } from "../api/costs";
import TaskBoardCard from "../components/TaskBoardCard.vue";

const store = useWorkspaceStore();
const router = useRouter();

const viewMode = ref<"kanban" | "folder">("kanban");
const sortKey = ref<"lastActive" | "created">("lastActive");
const collapsedCols = ref<Set<string>>(new Set());
const collapsedDirs = ref<Set<string>>(new Set());
const costMap = ref<Map<string, SessionCost>>(new Map());

// Cached home directory for detecting the app's default session dir
// (~/.runjam/session/{id}) — such sessions should not be grouped as
// independent directory rows; they belong under "Default directory".
const cachedHomeDir = ref("");

/** True when `path` is the default session dir (~/.runjam/session/{id}),
 *  i.e. the session was created without a user-chosen project folder. */
function isDefaultDir(path: string): boolean {
  const home = cachedHomeDir.value;
  if (home) {
    return path.startsWith(`${home}/.runjam/session/`) || path === `${home}/.runjam/session`;
  }
  // Home unknown (rare): fall back to a path-shape match.
  return /\/\.runjam\/session(?:\/|$)/.test(path);
}

const SORT_OPTIONS = [
  { key: "lastActive", labelKey: "board.sortRecent" as TranslationKey, icon: Clock },
  { key: "created", labelKey: "board.sortCreated" as TranslationKey, icon: CalendarClock },
] as const;

const sorted = computed(() => {
  const arr = [...store.sessions];
  arr.sort((a, b) => {
    const pinned = (b.pinned ? 1 : 0) - (a.pinned ? 1 : 0);
    if (pinned) return pinned;
    if (sortKey.value === "created") return new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime();
    return new Date(b.lastActiveAt).getTime() - new Date(a.lastActiveAt).getTime();
  });
  return arr;
});

interface ColumnDef {
  key: string;
  labelKey: TranslationKey;
  predicate: (s: Session) => boolean;
  dot: string;
  archivedOnly?: boolean;
}

// 统一色板：running=蓝（进行中）、completed=绿（完成未读）、history=紫（已读）、
// failed=红、archived=灰。
// 各列 predicate 必须严格互斥——同一会话只能出现在一个列。若 running 分支与
// completed/history 分支都命中（例如"用户正在查看的 idle 会话"），点击 history
// 卡片后该卡片会同时出现在 running 列与 history 列。
// running 列额外包含"用户当前正在查看的 idle 会话"，但仅限未读（刚完成一轮、
// 进程还活着、用户仍在它的上下文里，可继续对话），语义上就是"进行中"——
// 否则用户点开生成中的会话，等它完成后回到看板，会看到它掉进已读 History，
// 与"还在 running"的直觉不符。已读 idle 会话属于历史：用户点开它只是查看
// 历史，不应被划进 running，故 activeSessionId 命中但已读时仍留在 history 列。
const COLUMN_DEFS: ColumnDef[] = [
  {
    key: "running",
    labelKey: "board.columnRunning",
    predicate: s => s.status === "running" || (s.status === "idle" && s.unread && store.activeSessionId === s.id),
    dot: "bg-blue-400",
  },
  { key: "completed", labelKey: "board.columnCompleted", predicate: s => s.status === "idle" && s.unread && store.activeSessionId !== s.id, dot: "bg-emerald-400" },
  { key: "history", labelKey: "board.columnHistory", predicate: s => s.status === "stopped" || (s.status === "idle" && !s.unread), dot: "bg-violet-400" },
  { key: "failed", labelKey: "board.columnFailed", predicate: s => s.status === "error", dot: "bg-red-400" },
  { key: "archived", labelKey: "board.columnArchived", predicate: s => !!s.archived, dot: "bg-gray-300", archivedOnly: true },
];

const columns = computed(() =>
  COLUMN_DEFS.map(d => ({
    ...d,
    collapsed: collapsedCols.value.has(d.key),
    sessions: sorted.value.filter(s =>
      d.archivedOnly ? !!s.archived : !s.archived && d.predicate(s),
    ),
  })),
);

const dirGroups = computed(() => {
  const map = new Map<string, Session[]>();
  const orphans: Session[] = [];
  for (const s of sorted.value) {
    const key = s.directory || "";
    if (key && !isDefaultDir(key)) {
      if (!map.has(key)) map.set(key, []);
      map.get(key)!.push(s);
    } else {
      // 无目录或位于默认会话目录（~/.runjam/session）的会话统一归入
      // "Default directory"，不按各自路径分成独立组。
      orphans.push(s);
    }
  }
  const groups = [...map.entries()].map(([path, sessions]) => ({ path, sessions }));
  groups.sort(
    (a, b) =>
      new Date(b.sessions[0].lastActiveAt).getTime() - new Date(a.sessions[0].lastActiveAt).getTime(),
  );
  return { groups, orphans };
});

async function refreshCosts() {
  try {
    const list = await getCostBySession(100_000);
    costMap.value = new Map(list.map(c => [c.session_id, c]));
  } catch (err) {
    console.error("Failed to load session costs:", err);
  }
}

let timer: ReturnType<typeof setInterval> | null = null;
onMounted(async () => {
  if (store.sessions.length === 0) await store.loadSessions();
  homeDir()
    .then(h => {
      cachedHomeDir.value = h;
    })
    .catch(() => {});
  await refreshCosts();
  timer = setInterval(refreshCosts, 30_000);
});
onBeforeUnmount(() => {
  if (timer) clearInterval(timer);
});

/** Toggle collapse state of a folder group in folder view. */
function toggleDirGroup(path: string) {
  const next = new Set(collapsedDirs.value);
  if (next.has(path)) next.delete(path);
  else next.add(path);
  collapsedDirs.value = next;
}

function openSession(id: string) {
  store.selectSession(id);
  router.push("/");
}

function toggleCol(key: string) {
  const next = new Set(collapsedCols.value);
  if (next.has(key)) next.delete(key);
  else next.add(key);
  collapsedCols.value = next;
}

function handleDelete(id: string) {
  store.removeSession(id);
  costMap.value.delete(id);
}
</script>

<template>
  <div class="w-full h-full flex flex-col bg-gray-50 text-gray-900">
    <!-- slim toolbar -->
    <div class="flex items-center gap-3 px-4 py-3 flex-shrink-0">
      <h1 class="text-[14px] font-semibold tracking-tight text-gray-800">{{ $t("sidebar.sessionBoard") }}</h1>
      <div class="ml-auto flex items-center gap-2">
        <!-- view mode switch -->
        <div class="flex items-center bg-gray-100 rounded-xl p-1">
          <button
            :class="[
              'px-3 py-1.5 rounded-lg text-[12px] font-medium flex items-center gap-1.5 transition-colors cursor-pointer',
              viewMode === 'kanban' ? 'bg-white shadow-sm text-gray-900 dark:bg-gray-200 dark:shadow-none' : 'text-gray-500 hover:text-gray-700',
            ]"
            @click="viewMode = 'kanban'"
          >
            <Columns3 :size="14" />
            {{ $t("board.kanban") }}
          </button>
          <button
            :class="[
              'px-3 py-1.5 rounded-lg text-[12px] font-medium flex items-center gap-1.5 transition-colors cursor-pointer',
              viewMode === 'folder' ? 'bg-white shadow-sm text-gray-900 dark:bg-gray-200 dark:shadow-none' : 'text-gray-500 hover:text-gray-700',
            ]"
            @click="viewMode = 'folder'"
          >
            <FolderTree :size="14" />
            {{ $t("board.folder") }}
          </button>
        </div>
        <!-- sort switch -->
        <div class="flex items-center bg-gray-100 rounded-xl p-1">
          <button
            v-for="opt in SORT_OPTIONS"
            :key="opt.key"
            :class="[
              'px-3 py-1.5 rounded-lg text-[12px] font-medium flex items-center gap-1.5 transition-colors cursor-pointer',
              sortKey === opt.key ? 'bg-white shadow-sm text-gray-900 dark:bg-gray-200 dark:shadow-none' : 'text-gray-500 hover:text-gray-700',
            ]"
            @click="sortKey = opt.key"
          >
            <component :is="opt.icon" :size="14" />
            {{ $t(opt.labelKey) }}
          </button>
        </div>
      </div>
    </div>

    <!-- kanban view -->
    <div v-if="viewMode === 'kanban'" class="flex-1 overflow-hidden px-4 pb-4 min-h-0">
      <div class="flex gap-4 h-full overflow-x-auto">
        <div
          v-for="col in columns"
          :key="col.key"
          class="flex flex-col w-72 min-w-[17rem] max-w-xs flex-shrink-0 bg-gray-100/80 rounded-2xl border border-gray-100"
        >
          <div class="flex items-center gap-2 px-3.5 py-3 flex-shrink-0">
            <span class="w-2 h-2 rounded-full flex-shrink-0" :class="col.dot" />
            <span class="text-[12px] font-semibold text-gray-700">{{ $t(col.labelKey) }}</span>
            <span class="text-[11px] text-gray-400 ml-auto tabular-nums">{{ col.sessions.length }}</span>
            <button
              class="p-0.5 rounded hover:bg-gray-200/70 text-gray-400 transition-colors cursor-pointer"
              @click="toggleCol(col.key)"
            >
              <ChevronRight :size="13" :class="{ 'rotate-90': col.collapsed }" class="transition-transform duration-150" />
            </button>
          </div>
          <div v-if="!col.collapsed" class="flex-1 overflow-y-auto px-2.5 pb-3 space-y-2.5">
            <TaskBoardCard
              v-for="s in col.sessions"
              :key="s.id"
              :session="s"
              :message-count="costMap.get(s.id)?.message_count ?? 0"
              @open="openSession(s.id)"
              @pin="store.togglePin(s.id)"
              @archive="store.archiveSession(s.id)"
              @unarchive="store.unarchiveSession(s.id)"
              @delete="handleDelete(s.id)"
            />
            <p v-if="col.sessions.length === 0" class="text-center text-[11px] text-gray-400 py-6">{{ $t("board.empty") }}</p>
          </div>
        </div>
      </div>
    </div>

    <!-- folder view -->
    <div v-else class="flex-1 overflow-y-auto px-6 pb-6 min-h-0">
      <div v-if="dirGroups.groups.length === 0 && dirGroups.orphans.length === 0" class="pt-24 text-center">
        <Folder :size="36" class="mx-auto mb-3 text-gray-300" />
        <p class="text-[13px] text-gray-400">{{ $t("sidebar.emptyState") }}</p>
        <button
          class="mt-3 px-4 py-2 rounded-xl bg-emerald-500 text-white text-[12px] font-medium hover:bg-emerald-600 transition-colors cursor-pointer"
          @click="router.push('/')"
        >
          {{ $t("board.newSession") }}
        </button>
      </div>

      <div v-for="g in dirGroups.groups" :key="g.path" class="bg-white rounded-2xl border border-gray-100 shadow-sm mb-4">
        <button
          class="w-full flex items-center gap-2.5 px-4 py-3 border-b border-gray-50 text-left cursor-pointer hover:bg-gray-50/70 transition-colors"
          @click="toggleDirGroup(g.path)"
        >
          <ChevronRight
            :size="14"
            class="text-gray-400 transition-transform duration-150 flex-shrink-0"
            :class="{ 'rotate-90': !collapsedDirs.has(g.path) }"
          />
          <Folder :size="15" class="text-gray-500 flex-shrink-0" />
          <span class="text-[13px] font-semibold text-gray-800 flex-shrink-0">{{ g.path.split("/").pop() || g.path }}</span>
          <span class="text-[11px] text-gray-400 truncate min-w-0" :title="g.path">{{ g.path }}</span>
          <span class="text-[11px] text-gray-400 ml-auto flex-shrink-0 tabular-nums">
            {{ $t("board.sessions", { count: g.sessions.length }) }}
          </span>
        </button>
        <div v-if="!collapsedDirs.has(g.path)" class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4 gap-3 p-4">
          <TaskBoardCard
            v-for="s in g.sessions"
            :key="s.id"
            :session="s"
            :message-count="costMap.get(s.id)?.message_count ?? 0"
            @open="openSession(s.id)"
            @pin="store.togglePin(s.id)"
            @archive="store.archiveSession(s.id)"
            @unarchive="store.unarchiveSession(s.id)"
            @delete="handleDelete(s.id)"
          />
        </div>
      </div>

      <div v-if="dirGroups.orphans.length > 0" class="bg-white rounded-2xl border border-gray-100 shadow-sm">
        <button
          class="w-full flex items-center gap-2.5 px-4 py-3 border-b border-gray-50 text-left cursor-pointer hover:bg-gray-50/70 transition-colors"
          @click="toggleDirGroup('__default__')"
        >
          <ChevronRight
            :size="14"
            class="text-gray-400 transition-transform duration-150 flex-shrink-0"
            :class="{ 'rotate-90': !collapsedDirs.has('__default__') }"
          />
          <Folder :size="15" class="text-gray-500 flex-shrink-0" />
          <span class="text-[13px] font-semibold text-gray-800">{{ $t("board.defaultDirectory") }}</span>
          <span class="text-[11px] text-gray-400 ml-auto flex-shrink-0 tabular-nums">{{ $t("board.sessions", { count: dirGroups.orphans.length }) }}</span>
        </button>
        <div v-if="!collapsedDirs.has('__default__')" class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4 gap-3 p-4">
          <TaskBoardCard
            v-for="s in dirGroups.orphans"
            :key="s.id"
            :session="s"
            :message-count="costMap.get(s.id)?.message_count ?? 0"
            @open="openSession(s.id)"
            @pin="store.togglePin(s.id)"
            @archive="store.archiveSession(s.id)"
            @unarchive="store.unarchiveSession(s.id)"
            @delete="handleDelete(s.id)"
          />
        </div>
      </div>
    </div>
  </div>
</template>
