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
import {
  computed, nextTick, onBeforeUnmount, onMounted, ref, watch,
} from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  Send, Plus, X, Loader2, Sparkles, ChevronDown, Wand2,
  Folder, FolderOpen, ShieldCheck, Check, Maximize2, Paperclip, Brain, Square,
} from "lucide-vue-next";
import AgentIcon from "../AgentIcon.vue";
import ChatMessages from "../ChatMessages.vue";
import { usePetChat } from "../../composables/usePetChat";
import { formatFileSize } from "../../composables/useAttachments";
import { t, type TranslationKey } from "../../i18n";

const {
  messages,
  input,
  busy,
  starting,
  agentName,
  modelName,
  canSend,
  send,
  stop,
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
  isUnavailableLocalModel,
  refreshLocalServer,
  openLocalModelsSettings,
  // Attachments.
  attachedFiles,
  attachFiles,
  removeAttachedFile,
} = usePetChat();

const chatEl = ref<HTMLDivElement | null>(null);
const inputEl = ref<HTMLTextAreaElement | null>(null);
const attachListOpen = ref(false);
let unlistenOpened: UnlistenFn | null = null;

/**
 * Keep the newest message in view, but only when the user is already at the
 * bottom — otherwise their scroll position would be yanked away mid-read.
 */
function scrollToBottom(force = false) {
  nextTick(() => {
    const el = chatEl.value;
    if (!el) return;
    const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 120;
    if (force || nearBottom) el.scrollTop = el.scrollHeight;
  });
}

/** ChatMessages reports height changes (typewriter ticks, growing content). */
function onContentUpdated() {
  scrollToBottom();
}

watch(messages, () => scrollToBottom(), { deep: true });

async function close() {
  await invoke("close_pet_chat").catch(() => {});
}

/**
 * Hand the conversation over to the main window: it focuses, switches to this
 * session, and this popup hides itself.
 *
 * The popup now renders the same transcript as the main window, but the main
 * window is where the wider layout, file tree and terminal live — so this is the
 * "continue with more room" affordance.
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
  scrollToBottom(true);
}

/** Halt the running answer (the send button becomes Stop while busy). */
async function doStop() {
  await stop();
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
/** Anchor of the chip that opened the menu, in viewport coordinates. The menus
 *  are teleported to <body> and positioned `fixed`, so they escape the composer's
 *  clipping/scroll context entirely. `openUp` records which side had room. */
const menuAnchor = ref<
  { left: number; openUp: boolean; offset: number; width: number; maxHeight: number } | null
>(null);

/** Width of each menu panel (must match the classes below). */
const MENU_WIDTH: Record<"model" | "dir" | "perm" | "skills", number> = {
  model: 240,
  dir: 256,
  perm: 288,
  skills: 320,
};

/** Height budget per menu, used to decide where it fits and to cap it. */
const MENU_HEIGHT: Record<"model" | "dir" | "perm" | "skills", number> = {
  model: 220,
  dir: 240,
  perm: 260,
  skills: 240,
};

function toggleMenu(name: "model" | "dir" | "perm" | "skills", e?: MouseEvent) {
  if (openMenu.value === name) {
    openMenu.value = "";
    return;
  }
  const el = (e?.currentTarget ?? null) as HTMLElement | null;
  if (el) {
    const r = el.getBoundingClientRect();
    const width = MENU_WIDTH[name];
    // Keep the panel inside the window horizontally: shift it left when it would
    // overflow the right edge (the chips sit near the right side of a narrow
    // popup).
    const left = Math.max(4, Math.min(r.left, window.innerWidth - width - 4));

    // Open toward whichever side has room. These controls sit at the bottom of
    // the popup, so downward would normally be clipped by the window — but the
    // popup is resizable (down to 320px tall), so "always up" can also overflow
    // when it is short. Pick the larger side and cap the height to it; the panel
    // scrolls internally.
    const GAP = 4;
    const spaceAbove = r.top - GAP - 4;
    const spaceBelow = window.innerHeight - r.bottom - GAP - 4;
    const openUp = spaceAbove >= spaceBelow;
    const avail = Math.max(80, openUp ? spaceAbove : spaceBelow);
    menuAnchor.value = {
      left,
      openUp,
      offset: openUp ? window.innerHeight - r.top + GAP : r.bottom + GAP,
      width,
      maxHeight: Math.min(MENU_HEIGHT[name], avail),
    };
  }
  openMenu.value = name;
}

/** Computed style for the currently open menu. */
const menuStyle = computed(() => {
  const a = menuAnchor.value;
  if (!a) return {};
  return {
    left: `${a.left}px`,
    ...(a.openUp ? { bottom: `${a.offset}px` } : { top: `${a.offset}px` }),
    width: `${a.width}px`,
    maxHeight: `${a.maxHeight}px`,
  };
});

/** Toggle the staged-attachments list, anchoring it like the option menus (it is
 *  teleported to <body> for the same reason: the action row clips overflow). */
function toggleAttachList(e?: MouseEvent) {
  if (attachListOpen.value) {
    attachListOpen.value = false;
    return;
  }
  const el = (e?.currentTarget ?? null) as HTMLElement | null;
  if (el) {
    const r = el.getBoundingClientRect();
    const width = 280;
    const GAP = 4;
    const left = Math.max(4, Math.min(r.left, window.innerWidth - width - 4));
    const spaceAbove = r.top - GAP - 4;
    const spaceBelow = window.innerHeight - r.bottom - GAP - 4;
    const openUp = spaceAbove >= spaceBelow;
    const avail = Math.max(80, openUp ? spaceAbove : spaceBelow);
    menuAnchor.value = {
      left,
      openUp,
      offset: openUp ? window.innerHeight - r.top + GAP : r.bottom + GAP,
      width,
      maxHeight: Math.min(240, avail),
    };
  }
  attachListOpen.value = true;
}

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

const modelLabel = computed(() => {
  const m = availableModels.value.find((x) => x.id === config.value.modelId);
  // Prefer the alias: a local model's `name` is a full file path, far too wide
  // for the 460px popup chip.
  return m?.alias || m?.name || modelName.value || config.value.modelId || "";
});

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
  const model = availableModels.value.find((m) => m.id === id);
  // A local model whose server is not running cannot be used from the popup —
  // starting the server lives in Settings. Send the user there instead of
  // silently arming a model that would fail on send.
  if (model && isUnavailableLocalModel(model)) {
    await openLocalModelsSettings();
    return;
  }
  await applyConfig({ modelId: id });
}

/** Open the model menu, refreshing the local-server state in the background.
 *
 * The refresh probes HTTP ports (up to ~2s when nothing is listening), so
 * awaiting it would make the chip feel dead for seconds. Open the menu FIRST so
 * the click is instant, then let the refreshed availability settle into place —
 * the menu is reactive, so the greyed-out state updates when the probe returns.
 */
function openModelMenu(e?: MouseEvent) {
  toggleMenu("model", e);
  void refreshLocalServer();
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

// Compact toolbar control: one line tall, icon + truncated value. Shrinkable
// (`min-w-0` + no fixed width) so several of them fit on the popup's narrow
// action row.
const CHIP =
  "inline-flex items-center gap-1 px-1.5 h-6 rounded-md text-[10px] leading-none min-w-0 " +
  "text-gray-600 dark:text-gray-500 hover:bg-gray-100 dark:hover:bg-[#26262f] " +
  "transition-colors cursor-pointer";

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

    <!-- Transcript: the main window's own message list renders the whole turn —
         markdown, code blocks, thinking, tool calls, permission prompts. The
         popup owns scrolling, so ChatMessages reports height changes instead. -->
    <div ref="chatEl" class="flex-1 overflow-y-auto px-3 py-2 no-scrollbar">
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

      <ChatMessages
        v-else
        :messages="messages"
        :agent-id="config.agentId"
        :active="true"
        @content-updated="onContentUpdated"
      />
    </div>

    <!-- Composer: mirrors the main window's session composer — skills above the
         input, and the file/permission/model/reasoning controls on the action
         row below it. -->
    <div class="flex-shrink-0 border-t border-gray-100 dark:border-[#26262f] p-2.5">
      <!-- Skills (compact): the selected set as removable chips plus a picker. -->
      <div class="flex items-center gap-1.5 mb-1.5 min-h-[24px]">
        <div class="flex items-center gap-1 overflow-x-auto flex-1 min-w-0 no-scrollbar">
          <span
            v-for="name in config.skills"
            :key="name"
            class="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] flex-shrink-0
                   bg-gray-100 text-gray-700 dark:bg-[#26262f] dark:text-gray-500 cursor-pointer
                   hover:bg-gray-200 dark:hover:bg-[#35353f] transition-colors"
            @click="toggleSkill(name)"
          >
            {{ name }}<X :size="9" />
          </span>
        </div>
        <div class="relative flex-shrink-0" data-pet-popover>
          <button :class="CHIP" :title="t('pet.skills')" @click="toggleMenu('skills', $event)">
            <Wand2 :size="11" class="flex-shrink-0 opacity-70" />
            <span class="truncate">
              {{ t("pet.skills") }}<template v-if="config.skills.length"> · {{ config.skills.length }}</template>
            </span>
            <ChevronDown :size="10" class="flex-shrink-0 opacity-60" />
          </button>
          <Teleport to="body">
            <div v-if="openMenu === 'skills'" :class="MENU" :style="menuStyle" data-pet-popover>
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
      </div>

      <!-- Input box -->
      <div
        class="flex items-end gap-2 rounded-xl border border-gray-200 dark:border-[#2c2c36]
               bg-gray-50 dark:bg-[#131318] px-2.5 py-2 focus-within:border-indigo-300"
      >
        <textarea
          ref="inputEl"
          v-model="input"
          rows="1"
          :placeholder="busy ? t('pet.answering') : t('pet.askSomething')"
          class="flex-1 bg-transparent border-none outline-none resize-none text-[12px] text-gray-800
                 placeholder-gray-400 dark:placeholder-gray-600 leading-relaxed max-h-28 no-scrollbar"
          @keydown="onKeydown"
        />
        <button
          v-if="busy"
          class="p-1.5 rounded-lg flex-shrink-0 transition-colors bg-red-500 text-white hover:bg-red-600 cursor-pointer"
          :title="t('pet.stop')"
          @click="doStop"
        >
          <Square :size="14" />
        </button>
        <button
          v-else
          class="p-1.5 rounded-lg flex-shrink-0 transition-colors"
          :class="canSend
            ? 'bg-indigo-500 text-white hover:bg-indigo-600'
            : 'bg-gray-200 dark:bg-[#26262f] text-gray-400 cursor-not-allowed'"
          :disabled="!canSend"
          :title="t('pet.send')"
          @click="doSend"
        >
          <Send :size="14" />
        </button>
      </div>

      <!-- Action row: file + project folder + permission + model + reasoning, packed
           on ONE line. No `flex-1` spacer here: spreading them to the two ends
           made the attachments look like a separate row from the option chips. -->
      <div class="flex items-center gap-1.5 mt-1.5 flex-nowrap overflow-hidden">
        <button
          class="p-1 rounded-md flex-shrink-0 text-gray-400 hover:text-gray-600 hover:bg-gray-100
                 dark:hover:bg-[#26262f] transition-colors cursor-pointer"
          :title="t('pet.attachFile')"
          @click="attachFiles"
        >
          <Paperclip :size="13" />
        </button>
        <button
          v-if="attachedFiles.length > 0"
          class="relative min-w-[16px] h-[16px] px-1 rounded-full bg-gray-200 text-gray-700 dark:bg-[#26262f] dark:text-gray-500
                 text-[9px] font-semibold flex items-center justify-center cursor-pointer hover:bg-gray-300 transition-colors flex-shrink-0"
          :title="t('pet.attachments')"
          @click="toggleAttachList($event)"
        >{{ attachedFiles.length }}</button>

        <div class="relative min-w-0" data-pet-popover>
          <button
            class="inline-flex items-center gap-1 px-1.5 h-6 rounded-md text-[10px] leading-none min-w-0
                   text-gray-600 dark:text-gray-500 hover:bg-gray-100 dark:hover:bg-[#26262f] transition-colors cursor-pointer"
            :title="config.directory || t('pet.noProject')"
            @click="openDirMenu($event)"
          >
            <Folder :size="11" class="flex-shrink-0 opacity-70" />
            <span class="truncate">{{ dirLabel || t("pet.noProject") }}</span>
            <ChevronDown :size="10" class="flex-shrink-0 opacity-60" />
          </button>
          <Teleport to="body">
            <div v-if="openMenu === 'dir'" :class="MENU" :style="menuStyle" data-pet-popover>
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
            <div class="my-1 border-t border-gray-100 dark:border-[#26262f]"></div>
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

        <div class="relative min-w-0" data-pet-popover>
          <button :class="CHIP" :title="t('pet.permission')" @click="toggleMenu('perm', $event)">
            <ShieldCheck :size="11" class="flex-shrink-0 opacity-70" />
            <span class="truncate">{{ permissionLabel || t("pet.permission") }}</span>
            <ChevronDown :size="10" class="flex-shrink-0 opacity-60" />
          </button>
          <Teleport to="body">
            <div v-if="openMenu === 'perm'" :class="MENU" :style="menuStyle" data-pet-popover>
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

        <div class="relative min-w-0" data-pet-popover>
          <button :class="CHIP" :title="modelLabel || t('pet.model')" @click="openModelMenu($event)">
            <span class="truncate">{{ modelLabel || t("pet.model") }}</span>
            <ChevronDown :size="10" class="flex-shrink-0 opacity-60" />
          </button>
          <Teleport to="body">
            <div v-if="openMenu === 'model'" :class="MENU" :style="menuStyle" data-pet-popover>
            <button
              v-for="m in availableModels"
              :key="m.id"
              class="w-full flex items-center gap-1.5 px-2 py-1.5 text-left transition-colors"
              :class="isUnavailableLocalModel(m)
                ? 'opacity-50 cursor-pointer hover:bg-gray-50 dark:hover:bg-[#26262f]'
                : 'hover:bg-gray-50 dark:hover:bg-[#26262f]'"
              :title="isUnavailableLocalModel(m) ? t('pet.localModelNotRunning') : (m.alias || m.name)"
              @click="pickModel(m.id)"
            >
              <Check
                :size="11"
                class="flex-shrink-0"
                :class="m.id === config.modelId ? 'text-indigo-500' : 'opacity-0'"
              />
              <span class="min-w-0 flex-1">
                <span class="block text-[11px] truncate">{{ m.alias || m.name }}</span>
                <span
                  class="block text-[9px] truncate"
                  :class="isUnavailableLocalModel(m) ? 'text-amber-600 dark:text-amber-400' : 'text-gray-500'"
                >
                  {{ isUnavailableLocalModel(m) ? t("pet.localModelNotRunning") : m.provider_name }}
                </span>
              </span>
            </button>
            <div v-if="availableModels.length === 0" class="px-2 py-2 text-[10px] text-gray-500">
              {{ t("pet.model") }} —
            </div>
            </div>
          </Teleport>
        </div>

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
        </button>
      </div>

      <!-- Staged attachments list. Teleported like the option menus so it is
           never clipped by the action row (`overflow-hidden`) and uses the same
           open-up/​open-down anchoring. -->
      <Teleport to="body">
        <div
          v-if="attachedFiles.length > 0 && attachListOpen"
          :class="MENU"
          :style="menuStyle"
          class="w-[280px]"
          data-pet-popover
        >
          <div
            v-for="f in attachedFiles"
            :key="f.path"
            class="group flex items-center gap-2 px-2 py-1.5 hover:bg-gray-50 dark:hover:bg-[#26262f]"
          >
            <Paperclip :size="11" class="text-gray-400 flex-shrink-0" />
            <div class="flex-1 min-w-0">
              <div class="text-[11px] truncate" :title="f.path">{{ f.name }}</div>
              <div class="text-[9px] text-gray-500 truncate">{{ f.path }}</div>
            </div>
            <span class="text-[9px] text-gray-500 flex-shrink-0">{{ formatFileSize(f.size) }}</span>
            <button
              class="p-1 rounded text-gray-400 hover:text-red-500 transition-colors flex-shrink-0 cursor-pointer"
              :title="t('pet.removeAttachment')"
              @click="removeAttachedFile(f.path)"
            >
              <X :size="11" />
            </button>
          </div>
        </div>
      </Teleport>
    </div>
  </div>
</template>

<style scoped>
:global(html),
:global(body),
:global(#app) {
  background: transparent !important;
}

/*
 * The shared message list is designed for the main window's ~896px chat column,
 * where its agent bubble never reaches the global `min-width: 320px` floor. The
 * popup can be dragged down to 360px, where the bubble would be ~296px wide —
 * and `min-width` beats `max-width` in CSS, so the bubble would overflow its row
 * (a 24px horizontal scroll inside a 460px card). Drop the floor here only; the
 * main window is unaffected.
 */
:deep(.msg-agent-bubble) {
  min-width: 0;
}
</style>