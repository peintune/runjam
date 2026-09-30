<script setup lang="ts">
/**
 * Desktop-pet Q&A popup — the minimal conversation surface opened by clicking
 * the floating pet icon. It is a separate top-level window anchored to the
 * right edge of the screen and floats above every application.
 *
 * Layout: a drag handle + transcript, a *persistent* settings toolbar (agent /
 * model / folder / permission mode / skills / reasoning) that mirrors the main
 * window's new-session page, and a single input. Everything is changeable at
 * any time: picking a value restarts the conversation with it. All the heavy
 * lifting (session lifecycle, streaming, config persistence) lives in
 * `usePetChat`.
 */
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  Send, Plus, X, Loader2, Sparkles, ChevronDown, Wand2, Brain,
  Folder, FolderOpen, ShieldCheck, Check, Maximize2,
} from "lucide-vue-next";
import AgentIcon from "../AgentIcon.vue";
import { usePetChat } from "../../composables/usePetChat";
import { t, type TranslationKey } from "../../i18n";

const {
  messages,
  input,
  sending,
  starting,
  agentName,
  modelName,
  canSend,
  send,
  newConversation,
  openInMainWindow,
  ensureSession,
  greet,
  dispose,
  // Toolbar state (agent / model / folder / permission / skills / reasoning).
  config,
  availableAgents,
  availableModels,
  availableSkills,
  recentDirs,
  loadOptions,
  applyConfig,
  toggleSkill,
  toggleReasoning,
  setDirectory,
  refreshRecentDirs,
} = usePetChat();

const scrollEl = ref<HTMLDivElement | null>(null);
const inputEl = ref<HTMLTextAreaElement | null>(null);
let unlistenOpened: UnlistenFn | null = null;

function scrollToBottom() {
  nextTick(() => {
    if (scrollEl.value) scrollEl.value.scrollTop = scrollEl.value.scrollHeight;
  });
}

watch(messages, scrollToBottom, { deep: true });

async function close() {
  await invoke("close_pet_chat").catch(() => {});
}

/**
 * Hand the conversation over to the main window: it focuses, switches to this
 * session, and this popup hides itself.
 *
 * The popup only renders plain text, so the full session view (tool calls,
 * thinking, permission prompts) is where a user needs to go for anything more
 * than a quick answer.
 */
async function openInMain() {
  await openInMainWindow();
}

/** The chat window is hidden (not destroyed) when dismissed, so the session
 *  keeps running. On re-open we refocus the input. */
async function focusInput() {
  await nextTick();
  inputEl.value?.focus();
}

/** Enter sends; Shift+Enter inserts a newline. */
function onKeydown(e: KeyboardEvent) {
  if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
    e.preventDefault();
    doSend();
  }
}

async function doSend() {
  if (!canSend.value) return;
  await send();
  scrollToBottom();
}

function onEsc(e: KeyboardEvent) {
  if (e.key === "Escape") {
    // Close whichever popover is open first; only then dismiss the window.
    if (openMenu.value) {
      e.preventDefault();
      openMenu.value = "";
      return;
    }
    e.preventDefault();
    close();
  }
}

// ---------------------------------------------------------------------------
// Toolbar
// ---------------------------------------------------------------------------

/** Which dropdown is open (only one at a time). */
const openMenu = ref<"" | "model" | "dir" | "perm" | "skills">("");
/** Anchor rect of the chip that opened the menu, in viewport coordinates.
 *  The menus are teleported to <body> and positioned `fixed`, so they escape
 *  the toolbar's clipping/scroll context entirely. */
const menuAnchor = ref<{ left: number; top: number; width: number } | null>(null);

/** Width of each menu panel (must match the classes below). */
const MENU_WIDTH: Record<"model" | "dir" | "perm" | "skills", number> = {
  model: 240,
  dir: 256,
  perm: 288,
  skills: 320,
};

function toggleMenu(name: "model" | "dir" | "perm" | "skills", e?: MouseEvent) {
  if (openMenu.value === name) {
    openMenu.value = "";
    return;
  }
  const el = (e?.currentTarget ?? null) as HTMLElement | null;
  if (el) {
    const r = el.getBoundingClientRect();
    // Keep the panel inside the window: shift it left when it would overflow
    // the right edge (the chips sit near the right side of a narrow popup).
    const width = MENU_WIDTH[name];
    const left = Math.max(4, Math.min(r.left, window.innerWidth - width - 4));
    menuAnchor.value = { left, top: r.bottom + 4, width };
  }
  openMenu.value = name;
}

/** Computed style for the currently open menu. */
const menuStyle = computed(() => {
  const a = menuAnchor.value;
  if (!a) return {};
  return { left: `${a.left}px`, top: `${a.top}px`, width: `${a.width}px` };
});

function onDocClick(e: MouseEvent) {
  const el = e.target as HTMLElement | null;
  if (!el || !el.closest("[data-pet-popover]")) openMenu.value = "";
}

// Same four modes the main window offers. Claude Code and Gemini CLI expose
// different names for them, so the label is agent-dependent — kept in sync with
// `SessionView.vue`.
const PERMISSION_LABELS: Record<string, Record<string, string>> = {
  "claude-code": {
    read_only: "perm.plan", ask_approval: "perm.acceptEdits",
    approve_for_me: "perm.auto", full_access: "perm.bypass",
  },
  "codex-cli": {
    read_only: "perm.readOnlyLabel", ask_approval: "perm.askApprovalLabel",
    approve_for_me: "perm.approveForMeLabel", full_access: "perm.fullAccessLabel",
  },
  "gemini-cli": {
    read_only: "plan", ask_approval: "auto_edit",
    approve_for_me: "auto", full_access: "yolo",
  },
};

const PERMISSION_DESCRIPTIONS: Record<string, TranslationKey> = {
  read_only: "perm.readOnly",
  ask_approval: "perm.askApproval",
  approve_for_me: "perm.approveForMe",
  full_access: "perm.fullAccess",
};

const permissionOptions = computed(() =>
  (["read_only", "ask_approval", "approve_for_me", "full_access"] as const).map((id) => {
    const raw = PERMISSION_LABELS[config.value.agentId]?.[id] || id;
    return { id, label: t(raw as TranslationKey), description: t(PERMISSION_DESCRIPTIONS[id]) };
  }),
);

const agentLabel = computed(
  () =>
    availableAgents.value.find((a) => a.id === config.value.agentId)?.display_name ||
    agentName.value ||
    config.value.agentId ||
    "",
);

const modelLabel = computed(
  () =>
    availableModels.value.find((m) => m.id === config.value.modelId)?.name ||
    modelName.value ||
    config.value.modelId ||
    "",
);

const permissionLabel = computed(
  () => permissionOptions.value.find((o) => o.id === config.value.permissionMode)?.label ||
    config.value.permissionMode ||
    "",
);

/** Last path segment — a full path is far too wide for a 460px popup. */
const dirLabel = computed(() => {
  const p = config.value.directory;
  if (!p) return "";
  return p.replace(/\/+$/, "").split("/").pop() || p;
});

/** Switching agents means the previous model/mode no longer apply — the user
 *  should not be able to pick a mode the new agent names differently. */
async function pickAgent(id: string) {
  openMenu.value = "";
  if (id === config.value.agentId) return;
  await applyConfig({ agentId: id });
}

async function pickModel(id: string) {
  openMenu.value = "";
  await applyConfig({ modelId: id });
}

async function pickPermission(id: string) {
  openMenu.value = "";
  await applyConfig({ permissionMode: id });
}

async function pickDir(path: string) {
  openMenu.value = "";
  await setDirectory(path);
}

/** Native folder picker — same `plugin-dialog` call the main window uses. */
async function browseDir() {
  try {
    const picked = await openDialog({ directory: true, multiple: false });
    if (typeof picked === "string" && picked) await pickDir(picked);
  } catch (err) {
    console.error("[pet] folder picker failed:", err);
  }
}

function openDirMenu(e?: MouseEvent) {
  refreshRecentDirs();
  toggleMenu("dir", e);
}

/** Skill chips share one compact class; the selected ones are tinted. */
function skillChipClass(name: string) {
  return config.value.skills.includes(name)
    ? "bg-indigo-500 text-white border-indigo-500"
    : "bg-white dark:bg-[#131318] border-gray-200 dark:border-[#2c2c36] text-gray-700 dark:text-gray-800 hover:border-gray-300";
}

/** Dragging the header moves the undecorated window. */
async function startDrag(e: MouseEvent) {
  // Don't start a drag when the user clicks a button inside the header.
  if ((e.target as HTMLElement).closest("button")) return;
  try {
    await getCurrentWindow().startDragging();
  } catch {
    // ignore
  }
}

// Compact toolbar control: one line tall, icon + truncated value.
const CHIP =
  "inline-flex items-center gap-1 px-1.5 h-6 rounded-md text-[10px] leading-none " +
  "text-gray-600 dark:text-gray-500 hover:bg-gray-100 dark:hover:bg-[#26262f] " +
  "transition-colors cursor-pointer flex-shrink-0 max-w-[132px]";

/** Teleported to <body> and positioned `fixed`, so a dropdown is never clipped
 *  by the toolbar. Position comes from `menuStyle`. */
const MENU =
  "fixed rounded-lg border border-gray-200 dark:border-[#2c2c36] " +
  "bg-white dark:bg-[#1a1a21] shadow-xl z-[60] py-1 overflow-y-auto overflow-x-hidden " +
  // Explicit text color: this panel is Teleported to <body>, so it no longer
  // inherits the popup's `text-gray-900`. Without it the items fall back to the
  // body's dark `color: #1a1a2e` while sitting on a dark background. The gray
  // scale is inverted under `.dark`, so one class covers both themes.
  "text-gray-700";

onMounted(async () => {
  // Ensure a session exists (or a fresh one when the stored one went stale) so
  // the header can show which agent/model is about to answer.
  await ensureSession();
  greet();
  scrollToBottom();
  await focusInput();
  // Toolbar data (agents / models / skills / recent folders) loads in parallel
  // with the session so the popup is interactive immediately.
  void loadOptions();
  // Re-focus + greet whenever the window is reopened by the icon.
  unlistenOpened = await listen("pet:opened", () => {
    scrollToBottom();
    focusInput();
  });
  window.addEventListener("keydown", onEsc);
  document.addEventListener("click", onDocClick);
});

onBeforeUnmount(() => {
  if (unlistenOpened) {
    try { unlistenOpened(); } catch {}
    unlistenOpened = null;
  }
  window.removeEventListener("keydown", onEsc);
  document.removeEventListener("click", onDocClick);
  dispose();
});
</script>

<template>
  <!--
    `shadow` is deliberately NOT a CSS class here: the window is transparent and
    borderless, so a CSS box-shadow gets clipped by the square window bounds and
    shows up as dark square corners below the rounded card. The native window
    shadow (`shadow(true)` in pet_cmd.rs) already follows the rounded shape.
  -->
  <div
    class="w-screen h-screen flex flex-col overflow-hidden rounded-2xl border border-gray-200 dark:border-[#2c2c36]
           bg-white dark:bg-[#1a1a21] text-gray-900"
  >
    <!-- Header: drag handle + active agent + actions -->
    <div
      class="flex items-center gap-2 px-3 py-2 border-b border-gray-100 dark:border-[#26262f] flex-shrink-0 cursor-default"
      @mousedown="startDrag"
    >
      <Sparkles :size="14" class="text-indigo-500 flex-shrink-0" />
      <div class="min-w-0 flex-1 text-[12px] font-medium truncate">
        {{ agentLabel || t("pet.title") }}
      </div>
      <!-- Hand the conversation over to the main window, where the full
           transcript (tool calls, thinking, permission prompts) is visible. -->
      <button
        class="p-1.5 rounded-md text-gray-400 hover:text-gray-700 hover:bg-gray-100
               dark:hover:text-gray-800 dark:hover:bg-[#26262f] transition-colors flex-shrink-0"
        :title="t('pet.openInMain')"
        @click="openInMain"
      >
        <Maximize2 :size="14" />
      </button>
      <button
        class="p-1.5 rounded-md text-gray-400 hover:text-gray-700 hover:bg-gray-100
               dark:hover:text-gray-800 dark:hover:bg-[#26262f] transition-colors flex-shrink-0"
        :title="t('pet.newConversation')"
        @click="newConversation"
      >
        <Plus :size="14" />
      </button>
      <button
        class="p-1.5 rounded-md text-gray-400 hover:text-gray-700 hover:bg-gray-100
               dark:hover:text-gray-800 dark:hover:bg-[#26262f] transition-colors flex-shrink-0"
        :title="t('pet.close')"
        @click="close"
      >
        <X :size="14" />
      </button>
    </div>

    <!-- Settings toolbar: always visible, one line, any change restarts nothing
         unless the session went stale.
         `flex-nowrap` keeps it on a single row (with `flex-wrap` the chips
         spilled onto a second row on a narrow popup, pushing the transcript
         down). It is deliberately NOT `overflow-x-auto`: a scroll container
         would clip the dropdown panels. Those are teleported to <body> instead,
         so they escape this box entirely. -->
    <div
      class="flex flex-nowrap items-center gap-1 px-2.5 py-1.5 border-b border-gray-100 dark:border-[#26262f] flex-shrink-0"
    >
      <!-- Agent: icon pills, one per installed agent -->
      <div class="flex items-center gap-0.5 flex-shrink-0" :title="t('pet.agent')">
        <button
          v-for="a in availableAgents"
          :key="a.id"
          class="inline-flex items-center justify-center w-6 h-6 rounded-md transition-all cursor-pointer"
          :class="a.id === config.agentId
            ? 'bg-indigo-50 dark:bg-[#26262f] ring-1 ring-indigo-300 dark:ring-indigo-500/50'
            : 'opacity-50 hover:opacity-100 hover:bg-gray-100 dark:hover:bg-[#26262f]'"
          :title="a.display_name"
          @click="pickAgent(a.id)"
        >
          <AgentIcon :agent-id="a.id" :size="14" />
        </button>
      </div>

      <!-- Model -->
      <div class="relative" data-pet-popover>
        <button :class="CHIP" :title="modelLabel || t('pet.model')" @click="toggleMenu('model', $event)">
          <span class="truncate">{{ modelLabel || t("pet.model") }}</span>
          <ChevronDown :size="10" class="flex-shrink-0 opacity-60" />
        </button>
        <Teleport to="body">
          <div v-if="openMenu === 'model'" :class="MENU" :style="menuStyle" class="max-h-[220px]">
          <button
            v-for="m in availableModels"
            :key="m.id"
            class="w-full flex items-center gap-1.5 px-2 py-1.5 text-left hover:bg-gray-50
                   dark:hover:bg-[#26262f] transition-colors"
            @click="pickModel(m.id)"
          >
            <Check
              :size="11"
              class="flex-shrink-0"
              :class="m.id === config.modelId ? 'text-indigo-500' : 'opacity-0'"
            />
            <span class="min-w-0 flex-1">
              <span class="block text-[11px] truncate">{{ m.name }}</span>
              <span class="block text-[9px] text-gray-500 truncate">{{ m.provider_name }}</span>
            </span>
          </button>
          <div v-if="availableModels.length === 0" class="px-2 py-2 text-[10px] text-gray-500">
            {{ t("pet.model") }} —
          </div>
          </div>
        </Teleport>
      </div>

      <!-- Project folder -->
      <div class="relative" data-pet-popover>
        <button :class="CHIP" :title="config.directory || t('pet.noProject')" @click="openDirMenu($event)">
          <Folder :size="11" class="flex-shrink-0 opacity-70" />
          <span class="truncate">{{ dirLabel || t("pet.noProject") }}</span>
          <ChevronDown :size="10" class="flex-shrink-0 opacity-60" />
        </button>
        <Teleport to="body">
          <div v-if="openMenu === 'dir'" :class="MENU" :style="menuStyle" class="max-h-[240px]">
          <button
            class="w-full flex items-center gap-2 px-2 py-1.5 text-left hover:bg-gray-50
                   dark:hover:bg-[#26262f] transition-colors"
            @click="pickDir('')"
          >
            <Check :size="11" class="flex-shrink-0" :class="!config.directory ? 'text-indigo-500' : 'opacity-0'" />
            <span class="text-[11px] text-gray-500">{{ t("pet.noProject") }}</span>
          </button>
          <div v-if="recentDirs.length > 0" class="px-2 pt-1.5 pb-0.5 text-[9px] uppercase tracking-wide text-gray-500">
            {{ t("pet.recentProjects") }}
          </div>
          <button
            v-for="d in recentDirs"
            :key="d"
            class="w-full flex items-center gap-2 px-2 py-1.5 text-left hover:bg-gray-50
                   dark:hover:bg-[#26262f] transition-colors"
            :title="d"
            @click="pickDir(d)"
          >
            <Check :size="11" class="flex-shrink-0" :class="d === config.directory ? 'text-indigo-500' : 'opacity-0'" />
            <span class="text-[11px] truncate">{{ d.replace(/\/+$/, "").split("/").pop() || d }}</span>
          </button>
          <div class="my-1 border-t border-gray-100 dark:border-[#26262f]" />
          <button
            class="w-full flex items-center gap-2 px-2 py-1.5 text-left hover:bg-gray-50
                   dark:hover:bg-[#26262f] transition-colors"
            @click="browseDir"
          >
            <FolderOpen :size="11" class="flex-shrink-0 text-gray-500" />
            <span class="text-[11px] text-gray-600 dark:text-gray-500">{{ t("pet.openFolder") }}</span>
          </button>
          </div>
        </Teleport>
      </div>

      <!-- Permission mode -->
      <div class="relative" data-pet-popover>
        <button :class="CHIP" :title="t('pet.permission')" @click="toggleMenu('perm', $event)">
          <ShieldCheck :size="11" class="flex-shrink-0 opacity-70" />
          <span class="truncate">{{ permissionLabel || t("pet.permission") }}</span>
          <ChevronDown :size="10" class="flex-shrink-0 opacity-60" />
        </button>
        <Teleport to="body">
          <div v-if="openMenu === 'perm'" :class="MENU" :style="menuStyle" class="max-h-[260px]">
          <button
            v-for="o in permissionOptions"
            :key="o.id"
            class="w-full flex items-start gap-2 px-2 py-1.5 text-left hover:bg-gray-50
                   dark:hover:bg-[#26262f] transition-colors"
            @click="pickPermission(o.id)"
          >
            <Check
              :size="11"
              class="flex-shrink-0 mt-px"
              :class="o.id === config.permissionMode ? 'text-indigo-500' : 'opacity-0'"
            />
            <span class="min-w-0 flex-1">
              <span class="block text-[11px]">{{ o.label }}</span>
              <span class="block text-[9px] text-gray-500 leading-snug">{{ o.description }}</span>
            </span>
          </button>
          <!-- The popup has no permission dialog, so a mode that asks for
               approval can only be answered by auto-denying. Say so up front. -->
          <div
            v-if="config.permissionMode === 'ask_approval'"
            class="mx-2 mt-1 mb-0.5 px-2 py-1.5 rounded-md bg-amber-50 dark:bg-amber-500/10
                   text-[9px] leading-snug text-amber-600 dark:text-amber-400"
          >
            {{ t("pet.deniedNote") }}
          </div>
          </div>
        </Teleport>
      </div>

      <!-- Skills -->
      <div class="relative" data-pet-popover>
        <button :class="CHIP" :title="t('pet.skills')" @click="toggleMenu('skills', $event)">
          <Wand2 :size="11" class="flex-shrink-0 opacity-70" />
          <span class="truncate">
            {{ t("pet.skills") }}<template v-if="config.skills.length"> · {{ config.skills.length }}</template>
          </span>
          <ChevronDown :size="10" class="flex-shrink-0 opacity-60" />
        </button>
        <Teleport to="body">
          <div v-if="openMenu === 'skills'" :class="MENU" :style="menuStyle" class="max-h-[240px]">
          <div class="p-1.5 grid grid-cols-2 gap-1">
            <button
              v-for="s in availableSkills"
              :key="s.name"
              class="px-2 py-1.5 rounded-md border text-left transition-all cursor-pointer"
              :class="skillChipClass(s.name)"
              :title="s.description"
              @click="toggleSkill(s.name)"
            >
              <span class="block text-[11px] font-medium leading-tight truncate">{{ s.name }}</span>
              <span class="block text-[9px] leading-snug line-clamp-2 opacity-70">{{ s.description }}</span>
            </button>
          </div>
          <div v-if="availableSkills.length === 0" class="px-2 py-2 text-[10px] text-gray-500">
            {{ t("pet.skills") }} —
          </div>
          </div>
        </Teleport>
      </div>

      <!-- Reasoning toggle -->
      <button
        :class="[
          CHIP,
          config.noThinking
            ? 'bg-gray-200 dark:bg-[#26262f] text-gray-500'
            : 'text-indigo-600 dark:text-indigo-400 bg-indigo-50 dark:bg-indigo-500/10',
        ]"
        :title="t('pet.reasoning')"
        @click="toggleReasoning"
      >
        <Brain :size="11" class="flex-shrink-0" />
        <span class="truncate">{{ t("pet.reasoning") }}</span>
      </button>
    </div>

    <!-- Transcript -->
    <div ref="scrollEl" class="flex-1 overflow-y-auto px-3 py-3 space-y-3 no-scrollbar">
      <div
        v-if="starting && messages.length === 0"
        class="h-full flex flex-col items-center justify-center gap-2 text-gray-500"
      >
        <Loader2 :size="18" class="animate-spin" />
        <span class="text-[12px]">{{ t("pet.starting") }}</span>
      </div>

      <!-- Empty state: the popup is tiny and has no other affordance, so spell
           out how to use it (and that Enter sends). -->
      <div
        v-else-if="messages.length === 0"
        class="h-full flex flex-col items-center justify-center gap-2 text-gray-500 px-6 text-center"
      >
        <Sparkles :size="20" class="text-indigo-400" />
        <span class="text-[12px]">{{ t("pet.emptyQuestion") }}</span>
        <span class="text-[10px] text-gray-500/80">{{ t("pet.emptyHint") }}</span>
      </div>

      <template v-for="(m, i) in messages" :key="i">
        <!-- User question: right-aligned bubble -->
        <div v-if="m.role === 'user'" class="flex justify-end">
          <div
            class="max-w-[85%] px-3 py-2 rounded-2xl rounded-br-md bg-indigo-500 text-white
                   text-[12px] leading-relaxed whitespace-pre-wrap break-words"
          >
            {{ m.content }}
          </div>
        </div>
        <!-- Agent answer: left-aligned, plain (no bubble chrome) -->
        <div v-else class="flex justify-start">
          <div
            class="max-w-[92%] text-[12px] leading-relaxed whitespace-pre-wrap break-words"
            :class="m.error ? 'text-red-500' : 'text-gray-700 dark:text-gray-800'"
          >
            {{ m.content }}
            <span
              v-if="m.streaming"
              class="inline-block w-1.5 h-3.5 ml-0.5 align-middle bg-indigo-400 animate-pulse"
            />
          </div>
        </div>
      </template>
    </div>

    <!-- Composer -->
    <div class="flex-shrink-0 border-t border-gray-100 dark:border-[#26262f] p-2.5">
      <div
        class="flex items-end gap-2 rounded-xl border border-gray-200 dark:border-[#2c2c36]
               bg-gray-50 dark:bg-[#131318] px-2.5 py-2 focus-within:border-indigo-300"
      >
        <textarea
          ref="inputEl"
          v-model="input"
          rows="1"
          :placeholder="sending ? t('pet.answering') : t('pet.askSomething')"
          class="flex-1 bg-transparent border-none outline-none resize-none text-[12px] text-gray-800
                 placeholder-gray-400 dark:placeholder-gray-600 leading-relaxed max-h-28 no-scrollbar"
          @keydown="onKeydown"
        />
        <button
          class="p-1.5 rounded-lg flex-shrink-0 transition-colors"
          :class="canSend
            ? 'bg-indigo-500 text-white hover:bg-indigo-600'
            : 'bg-gray-200 dark:bg-[#26262f] text-gray-400 cursor-not-allowed'"
          :disabled="!canSend"
          @click="doSend"
        >
          <Loader2 v-if="sending" :size="14" class="animate-spin" />
          <Send v-else :size="14" />
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
:global(html),
:global(body),
:global(#app) {
  background: transparent !important;
}
</style>