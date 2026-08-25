<script setup lang="ts">
import { ref, watch, computed } from "vue";
import { t } from "../i18n";
import type { Session } from "../stores/useWorkspaceStore";
import { useWorkspaceStore } from "../stores/useWorkspaceStore";
import { useMessageStore } from "../stores/useMessageStore";
import { useAgentStore } from "../stores/useAgentStore";
import { MoreHorizontal, Pin, Pencil, Trash2, Archive, RotateCcw, Terminal, FileText } from "lucide-vue-next";
import AgentIcon from "./AgentIcon.vue";
import { useSessionLayout } from "../composables/useSessionLayout";
import { computeContextStats } from "../composables/useContextSize";

const props = defineProps<{
  session: Session;
  active: boolean;
  compact: boolean;
  batch: boolean;
  selected: boolean;
  renaming: boolean;
  archived: boolean;
  menuOpen: boolean;
  menuX: number;
  menuY: number;
}>();

const emit = defineEmits<{
  select: [];
  'toggle-select': [];
  'show-menu': [e: MouseEvent];
  'start-rename': [];
  'do-rename': [text: string];
  pin: [];
  archive: [];
  unarchive: [];
  delete: [];
}>();

const editText = ref("");

const { peekLayout } = useSessionLayout();

const workspaceStore = useWorkspaceStore();
const messageStore = useMessageStore();
const agentStore = useAgentStore();

/** Directory-level layout info for this session's bound directory */
const dirLayout = computed(() => {
  if (!props.session.directoryId || props.archived) return null;
  return peekLayout(props.session.directoryId);
});

/** Context-size stats for every visible session, so the sidebar shows the
 *  same number as the session view's progress ring (message chars + input
 *  draft vs the model's context_window × 4). Hidden for archived rows.
 *
 *  For sessions whose messages aren't loaded into the frontend, the total
 *  comes from the DB-persisted `context_chars` (accurate right on load). For
 *  the active session, whose messages ARE in memory, we compute live from the
 *  message store so the number tracks streaming output the DB hasn't caught
 *  up with yet, plus the live input draft. */
const contextStats = computed(() => {
  if (props.archived) return null;
  const messages = messageStore.getMessages(props.session.id);
  const model = agentStore.models.find(m => m.id === props.session.model);
  // Include the live input draft (as a length-only placeholder string) so
  // the total, ratio and color match the session view exactly.
  const draft = props.active ? workspaceStore.activeDraftChars : 0;
  if (props.active && messages.length > 0) {
    // Active session: in-memory messages are the freshest source.
    return computeContextStats(
      messages,
      " ".repeat(draft),
      model?.context_window || undefined,
    );
  }
  // Inactive session (or messages not yet loaded): use the DB-persisted
  // char count so the row shows the context size immediately on load.
  return computeContextStats(
    [],
    " ".repeat(draft),
    model?.context_window || undefined,
    props.session.contextChars,
  );
});

/** Compact label like "12.3k/200k" for the sidebar row. */
const contextLabel = computed(() => {
  const s = contextStats.value;
  if (!s) return "";
  const fmt = (n: number) => {
    if (n < 1_000) return `${n}`;
    if (n < 1_000_000) return `${(n / 1_000).toFixed(n < 10_000 ? 1 : 0)}k`;
    return `${(n / 1_000_000).toFixed(2)}M`;
  };
  return `${fmt(s.totalChars)}/${fmt(s.maxChars)}`;
});

const contextTitle = computed(() => {
  const s = contextStats.value;
  if (!s) return "";
  return t("sessionItem.contextTitle", {
    used: s.totalChars.toLocaleString(),
    max: s.maxChars.toLocaleString(),
  });
});

function timeAgo(iso: string): string {
  const diff = Date.now() - new Date(iso).getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return t("sessionItem.justNow");
  if (mins < 60) return t("sessionItem.minutesAgo", { count: mins });
  const hours = Math.floor(mins / 60);
  if (hours < 24) return t("sessionItem.hoursAgo", { count: hours });
  const days = Math.floor(hours / 24);
  if (days < 7) return t("sessionItem.daysAgo", { count: days });
  return new Date(iso).toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
}
watch(() => props.renaming, (v) => { if (v) editText.value = props.session.title || props.session.cliDisplayName; });

function onContextMenu(e: MouseEvent) { e.preventDefault(); emit('show-menu', e); }
function onClick() { if (props.batch && !props.archived) emit('toggle-select'); else emit('select'); }
function submitRename() { if (editText.value.trim()) emit('do-rename', editText.value.trim()); }
</script>

<template>
  <div
    :class="[
      'w-full flex items-center gap-3 rounded-xl text-[13px] transition-all duration-150 text-left cursor-pointer group',
      compact ? 'px-2.5 py-1.5' : 'px-3 py-2',
      archived ? 'opacity-50 hover:opacity-80' : '',
      active && !archived ? 'bg-gray-100 text-gray-900 font-medium dark:bg-gray-200' : 'text-gray-600 hover:bg-gray-50',
    ]"
    @click="onClick"
    @contextmenu="onContextMenu"
  >
    <!-- Checkbox in batch mode -->
    <input v-if="batch && !archived" type="checkbox" :checked="selected" @click.stop="emit('toggle-select')" class="w-3.5 h-3.5 rounded border-gray-300 text-gray-700 flex-shrink-0 cursor-pointer" />
    <!-- Agent icon (not in compact mode) -->
    <AgentIcon v-else-if="!compact || archived" :agent-id="session.cli" />

    <!-- Title area -->
    <div class="flex-1 min-w-0">
      <div v-if="renaming" class="flex-1">
        <input v-model="editText" @blur="submitRename" @keydown.enter="submitRename" @click.stop
          class="w-full bg-transparent border-b border-gray-300 outline-none text-[13px]" />
      </div>
      <template v-else>
        <div class="flex items-center gap-1.5">
          <Pin v-if="session.pinned && !compact" :size="10" class="text-amber-400 flex-shrink-0" />
          <span class="truncate">{{ session.title || session.cliDisplayName }}</span>
        </div>
        <p v-if="!compact" class="text-[11px] text-gray-400 mt-0.5 truncate flex items-center gap-1.5">
          {{ timeAgo(session.lastActiveAt) }}
          <span
            v-if="contextStats"
            :title="contextTitle"
            class="inline-flex items-center gap-1 flex-shrink-0 ml-auto pl-1.5"
          >
            <span class="tabular-nums" :style="{ color: contextStats.ringColor }">{{ contextLabel }}</span>
          </span>
        </p>
      </template>
    </div>

    <!-- Status indicators -->
    <div v-if="!batch && !compact" class="flex items-center gap-1 flex-shrink-0">
      <!-- Running → blinking blue dot -->
      <span
        v-if="session.status === 'running'"
        class="w-1.5 h-1.5 rounded-full bg-blue-400 flex-shrink-0 animate-blink"
        :title="$t('sessionItem.running')"
      />
      <!-- Completed (reply done, not yet viewed) → solid green dot -->
      <span
        v-else-if="session.status === 'idle' && session.unread"
        class="w-1.5 h-1.5 rounded-full bg-emerald-400 flex-shrink-0"
        :title="$t('sessionItem.completed')"
      />
      <!-- Completed but not yet opened → solid green dot -->
      <span
        v-else-if="session.newlyCompleted && !active"
        class="w-1.5 h-1.5 rounded-full bg-emerald-400 flex-shrink-0"
        :title="$t('sessionItem.new')"
      />
      <!-- No dot for stopped/error — just show terminal/file icons if any -->
      <!-- Explorer icon -->
      <span
        v-if="dirLayout?.hasOpenFiles"
        class="p-0.5 rounded bg-gray-200 text-gray-700"
        :title="$t('sessionItem.explorer')"
      >
        <FileText :size="11" />
      </span>
      <!-- Terminal icon -->
      <span
        v-if="dirLayout?.hasTerminal"
        class="p-0.5 rounded bg-gray-200 text-gray-700"
        :title="$t('sessionItem.terminal')"
      >
        <Terminal :size="11" />
      </span>
    </div>

    <!-- Context menu trigger + dropdown -->
    <div v-if="!batch" class="relative flex-shrink-0" @click.stop @mousedown.stop>
      <button data-menu @click="emit('show-menu', $event)" class="p-1 rounded-lg opacity-0 group-hover:opacity-100 hover:bg-gray-200/60 transition-all">
        <MoreHorizontal :size="13" class="text-gray-400" />
      </button>
      <div v-if="menuOpen" data-menu class="fixed z-50 w-40 bg-white rounded-xl border border-gray-100 shadow-xl py-1.5" :style="{ left: menuX + 'px', top: menuY + 'px' }">
        <template v-if="!archived">
          <button @click="emit('pin')" class="w-full text-left px-3.5 py-2 text-[12px] text-gray-600 hover:bg-gray-50 flex items-center gap-2.5 transition-colors">
            <Pin :size="13" class="text-gray-400" /> {{ session.pinned ? $t("sessionItem.unpin") : $t("sessionItem.pinToTop") }}
          </button>
          <button @click="emit('start-rename')" class="w-full text-left px-3.5 py-2 text-[12px] text-gray-600 hover:bg-gray-50 flex items-center gap-2.5 transition-colors">
            <Pencil :size="13" class="text-gray-400" /> {{ $t("sessionItem.rename") }}
          </button>
          <button @click="emit('archive')" class="w-full text-left px-3.5 py-2 text-[12px] text-gray-600 hover:bg-gray-50 flex items-center gap-2.5 transition-colors">
            <Archive :size="13" class="text-gray-400" /> {{ $t("sessionItem.archive") }}
          </button>
          <div class="mx-3 my-1 border-t border-gray-100" />
          <button @click="emit('delete')" class="w-full text-left px-3.5 py-2 text-[12px] text-red-500 hover:bg-red-50 flex items-center gap-2.5 transition-colors">
            <Trash2 :size="13" /> {{ $t("sessionItem.delete") }}
          </button>
        </template>
        <template v-else>
          <button @click="emit('unarchive')" class="w-full text-left px-3.5 py-2 text-[12px] text-gray-600 hover:bg-gray-50 flex items-center gap-2.5 transition-colors">
            <RotateCcw :size="13" class="text-gray-400" /> {{ $t("sessionItem.unarchive") }}
          </button>
          <div class="mx-3 my-1 border-t border-gray-100" />
          <button @click="emit('delete')" class="w-full text-left px-3.5 py-2 text-[12px] text-red-500 hover:bg-red-50 flex items-center gap-2.5 transition-colors">
            <Trash2 :size="13" /> {{ $t("sessionItem.delete") }}
          </button>
        </template>
      </div>
    </div>
  </div>
</template>
