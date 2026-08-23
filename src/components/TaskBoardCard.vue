<script setup lang="ts">
import { computed } from "vue";
import { Pin, Archive, Trash2, RotateCcw, Folder, FileText, Terminal, MessageSquare } from "lucide-vue-next";
import type { Session } from "../stores/useWorkspaceStore";
import { useSessionLayout } from "../composables/useSessionLayout";
import { computeContextStats } from "../composables/useContextSize";
import { useAgentStore } from "../stores/useAgentStore";
import AgentIcon from "./AgentIcon.vue";

const props = defineProps<{
  session: Session;
  /** Number of stored conversation messages (0 when not tracked). */
  messageCount: number;
}>();

const emit = defineEmits<{
  open: [];
  pin: [];
  archive: [];
  unarchive: [];
  delete: [];
}>();

const { peekLayout } = useSessionLayout();
const agentStore = useAgentStore();

/** Directory-level layout info (file tree / terminal usage) for this session's
 *  bound directory. Comes from the localStorage layout snapshot (dir-level,
 *  best-effort) — the same source the sidebar rows use. */
const dirLayout = computed(() => {
  if (!props.session.directoryId || props.session.archived) return null;
  return peekLayout(props.session.directoryId);
});

const model = computed(() => agentStore.models.find(m => m.id === props.session.model));

/** Context-size stats, identical to the sidebar rows (DB-persisted
 *  context_chars + the model's context window). */
const contextStats = computed(() =>
  computeContextStats([], "", model.value?.context_window || undefined, props.session.contextChars),
);

const fmt = (n: number) => {
  if (n < 1_000) return `${n}`;
  if (n < 1_000_000) return `${(n / 1_000).toFixed(n < 10_000 ? 1 : 0)}k`;
  return `${(n / 1_000_000).toFixed(2)}M`;
};

const contextLabel = computed(() => `${fmt(contextStats.value.totalChars)}/${fmt(contextStats.value.maxChars)}`);

const dirBase = computed(() => props.session.directory?.split("/").pop() || "Default directory");

const statusMeta: Record<string, { label: string; dot: string; text: string; bg: string }> = {
  running: { label: "Running", dot: "bg-emerald-400 animate-blink", text: "text-emerald-700", bg: "bg-emerald-50" },
  waiting: { label: "Waiting", dot: "bg-amber-400", text: "text-amber-700", bg: "bg-amber-50" },
  idle: { label: "Idle", dot: "bg-emerald-400", text: "text-emerald-700", bg: "bg-emerald-50" },
  stopped: { label: "Completed", dot: "bg-sky-400", text: "text-sky-700", bg: "bg-sky-50" },
  error: { label: "Failed", dot: "bg-red-400", text: "text-red-700", bg: "bg-red-50" },
};

const badge = computed(() => {
  if (props.session.archived) return { label: "Archived", dot: "bg-gray-300", text: "text-gray-500", bg: "bg-gray-100" };
  return statusMeta[props.session.status] ?? { label: props.session.status, dot: "bg-gray-400", text: "text-gray-600", bg: "bg-gray-100" };
});

function timeAgo(iso: string): string {
  const diff = Date.now() - new Date(iso).getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return "Just now";
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d ago`;
  return new Date(iso).toLocaleDateString("en-US", { month: "short", day: "numeric" });
}
</script>

<template>
  <div
    class="group bg-white rounded-xl border border-gray-100 shadow-sm hover:shadow-md hover:border-gray-200 transition-all duration-150 p-3.5 cursor-pointer"
    @click="emit('open')"
  >
    <!-- title row -->
    <div class="flex items-start gap-2.5">
      <AgentIcon :agent-id="session.cli" class="mt-0.5 flex-shrink-0" />
      <div class="flex-1 min-w-0">
        <div class="flex items-center gap-1.5">
          <Pin v-if="session.pinned" :size="11" class="text-amber-400 flex-shrink-0" />
          <span class="truncate text-[13px] font-medium text-gray-800 leading-snug">{{ session.title || session.cliDisplayName }}</span>
        </div>
        <p class="flex items-center gap-1 text-[11px] text-gray-400 truncate mt-1" :title="session.directory || undefined">
          <Folder :size="11" class="flex-shrink-0" />
          <span class="truncate">{{ dirBase }}</span>
        </p>
      </div>
      <!-- hover actions -->
      <div class="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0" @click.stop>
        <template v-if="!session.archived">
          <button
            :title="session.pinned ? 'Unpin' : 'Pin to top'"
            class="p-1 rounded-md text-gray-400 hover:text-amber-500 hover:bg-gray-100 transition-colors cursor-pointer"
            @click="emit('pin')"
          >
            <Pin :size="12" />
          </button>
          <button
            title="Archive"
            class="p-1 rounded-md text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors cursor-pointer"
            @click="emit('archive')"
          >
            <Archive :size="12" />
          </button>
        </template>
        <template v-else>
          <button
            title="Unarchive"
            class="p-1 rounded-md text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors cursor-pointer"
            @click="emit('unarchive')"
          >
            <RotateCcw :size="12" />
          </button>
        </template>
        <button
          title="Delete"
          class="p-1 rounded-md text-gray-400 hover:text-red-500 hover:bg-red-50 transition-colors cursor-pointer"
          @click="emit('delete')"
        >
          <Trash2 :size="12" />
        </button>
      </div>
    </div>

    <!-- status row -->
    <div class="flex items-center gap-2 mt-3">
      <span :class="['inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md text-[10px] font-semibold', badge.bg, badge.text]">
        <span class="w-1.5 h-1.5 rounded-full flex-shrink-0" :class="badge.dot" />
        {{ badge.label }}
      </span>
      <span v-if="session.model" class="text-[11px] text-gray-400 truncate">{{ session.model }}</span>
      <span class="text-[11px] text-gray-400 ml-auto flex-shrink-0">{{ timeAgo(session.lastActiveAt) }}</span>
    </div>

    <!-- stats row -->
    <div class="flex items-center gap-3 mt-2.5 pt-2.5 border-t border-gray-50 text-[11px] text-gray-500">
      <span
        class="inline-flex items-center gap-1 tabular-nums"
        title="Context (chars, same as sidebar)"
        :style="{ color: contextStats.ringColor }"
      >
        <FileText :size="11" class="flex-shrink-0" />
        {{ contextLabel }}
      </span>
      <span v-if="messageCount > 0" class="inline-flex items-center gap-1 text-gray-400" title="Messages">
        <MessageSquare :size="11" class="flex-shrink-0" />
        {{ messageCount }}
      </span>
      <span class="flex items-center gap-1 ml-auto">
        <span
          v-if="dirLayout?.hasOpenFiles"
          class="p-0.5 rounded bg-gray-100 text-gray-500"
          title="Explorer"
        >
          <FileText :size="11" />
        </span>
        <span
          v-if="dirLayout?.hasTerminal"
          class="p-0.5 rounded bg-gray-100 text-gray-500"
          title="Terminal"
        >
          <Terminal :size="11" />
        </span>
      </span>
    </div>
  </div>
</template>
