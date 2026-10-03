<script setup lang="ts">
import { ref, computed, watch, onMounted, onBeforeUnmount } from "vue";
import { useRoute, useRouter } from "vue-router";

defineOptions({ name: "WorkspaceLayout" });
import Sidebar from "./Sidebar.vue";
import SessionView from "./SessionView.vue";
import WorkspacePanel from "./WorkspacePanel.vue";
import TerminalPanel from "./TerminalPanel.vue";
import TaskBoardView from "../views/TaskBoardView.vue";
import ConfirmDialog from "./ConfirmDialog.vue";
import SearchButton from "./SearchButton.vue";
import WindowControls from "./WindowControls.vue";
import AppTabsBar from "./AppTabsBar.vue";
import { useWorkspaceStore } from "../stores/useWorkspaceStore";
import { useAppTabsStore } from "../stores/useAppTabsStore";
import { useDragResize } from "../composables/useDragResize";
import { useSessionLayout, layoutKeyFor, isWorkspaceAreaVisible } from "../composables/useSessionLayout";
import { PET_SESSION_CHANGED_EVENT, PET_OPEN_IN_MAIN_EVENT } from "../composables/usePetChat";
import { homeDir } from "@tauri-apps/api/path";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  PanelLeftOpen, PanelLeftClose,
  FolderTree, Terminal,
} from "lucide-vue-next";

// ── State ────────────────────────────────────────
// 用户是否主动展开过左侧导航，持久化到 localStorage。只用于回答一个问题：
// 打开文件树/终端时该不该自动收起导航。
//   - true : 用户主动展开过 → 不再自动收起（尊重用户的选择）
//   - 其他 : 从未表态/主动收起 → 打开面板时自动收起
// 注意这里**不**决定启动时的展开状态：启动始终默认展开（保持既有行为），
// 避免持久化的偏好让重启后的界面与用户预期不符。
const SIDEBAR_PIN_KEY = "runjam-sidebar-pinned";

function loadSidebarUserPinned(): boolean {
  try {
    return localStorage.getItem(SIDEBAR_PIN_KEY) === "true";
  } catch {
    return false;
  }
}

const sidebarUserPinned = ref(loadSidebarUserPinned());
const sidebarPinned = ref(true);
const sidebarHover = ref(false);

/** 用户手动展开/收起导航：若这次是主动展开，记住该表态（持久化）。 */
function setSidebarPinned(v: boolean) {
  sidebarPinned.value = v;
  if (v) {
    sidebarUserPinned.value = true;
    try {
      localStorage.setItem(SIDEBAR_PIN_KEY, "true");
    } catch {}
  }
}

/** 打开文件树/终端时收起导航。用户主动展开过导航，就不再强行把它合上。 */
function collapseSidebarForPanel() {
  if (!sidebarUserPinned.value) {
    sidebarPinned.value = false;
  }
}

const store = useWorkspaceStore();
const route = useRoute();
const router = useRouter();
const { layout, switchDirectory, saveLayout, openFileSignal } = useSessionLayout();
const appTabs = useAppTabsStore();

/** True when rendering the task-board view (route /board) instead of a session. */
const isBoard = computed(() => route.path === "/board");

// Cached home directory for computing default session paths. The backend
// uses ~/.runjam/session/{id} for sessions without a user-chosen directory;
// we need this path so the file explorer and terminal work for those
// sessions too (not just for sessions bound to a project directory).
const cachedHomeDir = ref("");

/** macOS keeps native traffic lights (titleBarStyle: Overlay); other platforms
 *  use decorations:false + our custom title bar, so no traffic-light spacer. */
const isMac =
  typeof navigator !== "undefined" && /mac/i.test(navigator.platform || navigator.userAgent);

/** Layout bucket key for the active session.
 *
 *  Keyed by the session's working-directory PATH (`session.directory`), not by
 *  the store's `directoryId` — that id is regenerated on every launch, so
 *  keying by it lost the layout (and the sidebar badges) across restarts.
 *
 *  Sessions bound to a project folder share that folder's bucket (directory-level
 *  memory, as before). Default-directory sessions get ~/.runjam/session/{id},
 *  already unique per session; legacy rows with an empty directory fall back to
 *  a per-session bucket. Previously the latter all shared a single bucket, so a
 *  terminal opened in one such session showed up in every other one.
 *
 *  `null` means "no active session" (new-session page) → the default bucket. */
function currentLayoutKey(): string | null {
  const session = store.activeSession;
  return layoutKeyFor(session?.directory, session?.id);
}

// Terminal / file-tree visibility come from persisted layout
const showTerminal = ref(false);
const showFileTree = ref(false);

const activeDirectory = computed(() => {
  const session = store.activeSession;
  // 新建会话页面（尚未创建会话）：用新会话页选中的项目目录；未选时回退到
  // home 目录，保证右上角那对按钮在新建会话页也能点开。
  if (!session) {
    return store.newSessionDirPath || cachedHomeDir.value || "";
  }
  // 1. Bound project directory (user-selected folder)
  if (session.directoryId) {
    const dir = store.directories.find((d) => d.id === session.directoryId);
    if (dir?.path) return dir.path;
  }
  // 2. Actual working directory captured from the backend's start_session
  //    response (includes default-directory sessions: ~/.runjam/session/{id})
  if (session.directory) return session.directory;
  // 3. Fallback for old sessions whose directory wasn't persisted — compute
  //    the default path so the explorer/terminal still work.
  if (cachedHomeDir.value) {
    return `${cachedHomeDir.value}/.runjam/session/${session.id}`;
  }
  return "";
});

/** 工作区面板（文件树 + 编辑器）是否占用空间。
 *
 *  判据就是**文件树开关**：右上角那个按钮既是"打开侧边文件列表"，也是整个
 *  右侧工作区（文件树 + 编辑器）的总开关。打开文件时会自动展开文件树（见
 *  `openFileSignal` 的 watcher），所以二者天然一致；关了文件树却还留一个孤立
 *  的编辑器占着宽度，就是实际发生过的 bug。
 *
 *  这里**刻意不看"工作区模式"**（曾有一个 isWorkspaceMode ref）。那个标志在
 *  打开终端时也会被置位，于是「某会话历史上用过文件树/开过文件」（这些按目录
 *  持久化）会让"只点终端"也把编辑器一起显示出来、把对话区挤窄。终端是底部
 *  横跨的区域，与上层工作区互不相干，不该参与这个判断。
 *
 *  `activeDirectory` 仍需校验：没有工作目录时编辑器无内容可显示。 */
const showWorkspaceArea = computed(() =>
  isWorkspaceAreaVisible({
    isBoard: isBoard.value,
    hasDirectory: !!activeDirectory.value,
    showFileTree: showFileTree.value,
  }),
);

// 会话里点开一个文件（消息正文/thinking 的文件链接，或工具调用的打开按钮）
// 时，把文件树一并展开，这样面板立刻有内容可看（否则只显示刚打开的那一个
// 文件）。
//
// 监听的是 openFileSignal（一个自增计数），不是 openFiles/activeFileIndex 的
// 差异。因为"重新打开一个已经打开的文件、且面板当前是隐藏的"这种情况，state
// 前后完全一样（length 不变，index 也不变），差异式监听必然漏掉它 —— 那正是
// 「打开文件 → 关闭文件树 → 再点同一个文件没反应」的成因。信号没有这个盲区。
//
// 一并把文件树展开：用户刚点开一个文件，旁边同时给出同级文件更有用。
watch(openFileSignal, () => {
  if (layout.openFiles.length === 0) return;
  if (!showFileTree.value) {
    showFileTree.value = true;
    layout.showFileTree = true;
  }
  collapseSidebarForPanel();
  saveLayout();
});

/** 文件树按钮：切换文件树。打开时收窄侧栏，为右侧工作区腾出空间。 */
function toggleFileTree() {
  showFileTree.value = !showFileTree.value;
  layout.showFileTree = showFileTree.value;
  if (showFileTree.value) {
    collapseSidebarForPanel();
  }
  saveLayout();
}

/** 顶栏终端按钮：只显示/隐藏底部面板，**不**终止后端 shell 进程。
 *  TerminalPanel 在 active=false 时把整组 xterm 实例 detach 进 LRU 缓存
 *  （后端进程与 listener 保活），再次打开即原样接回——这正是「隐藏但继续
 *  运行」。真正终止进程走面板自身的 X 按钮（见 closeTerminalAndKill）。 */
function toggleTerminal() {
  showTerminal.value = !showTerminal.value;
  layout.showTerminal = showTerminal.value;
  if (showTerminal.value) {
    // 只收窄侧栏给终端腾宽度；**不**去动上层工作区 —— 终端是底部横跨的区域，
    // 用户只点终端就只开终端，不该顺带弹出编辑器。
    collapseSidebarForPanel();
  }
  saveLayout();
}

// ── Terminal close confirmation ────────────────────
// 只有面板自身的 X 按钮会真正 kill 后端 shell 进程（顶栏按钮仅隐藏），
// 所以这里先确认。顶栏 toggleTerminal 不经过这条路径。
const showTerminalCloseConfirm = ref(false);
const terminalPanelRef = ref<InstanceType<typeof TerminalPanel> | null>(null);

// 桌面宠物会话变化（在宠物窗口创建/更新）时用于刷新侧边栏列表的监听句柄。
let petSessionUnlisten: UnlistenFn | null = null;
// 宠物窗口请求「在主窗口打开」当前会话时的监听句柄。
let petOpenUnlisten: UnlistenFn | null = null;

// 终端高度可拖拽（位于内容区底部，横跨全宽）
const terminalResize = useDragResize({
  direction: "vertical",
  minSize: 80,
  defaultSize: 130,
  initialSize: layout.terminalHeight,
  onDragEnd: (size) => { layout.terminalHeight = size; },
});
watch(() => layout.terminalHeight, (h) => { terminalResize.size.value = h; });

async function confirmCloseTerminal() {
  showTerminalCloseConfirm.value = false;
  await terminalPanelRef.value?.killAll();
  showTerminal.value = false;
  layout.showTerminal = false;
  saveLayout();
}

function cancelCloseTerminal() {
  showTerminalCloseConfirm.value = false;
}

/** 面板 X 按钮：真正关闭终端（终止进程），先弹确认。 */
function closeTerminalAndKill() {
  if (showTerminal.value) {
    showTerminalCloseConfirm.value = true;
  }
}

// ---- Session persistence (keyed by directoryId) ----
watch(() => store.activeSessionId, (newId, oldId) => {
  const key = currentLayoutKey();
  const panelsWereOpen = showFileTree.value || showTerminal.value;
  switchDirectory(key);
  if (newId) {
    if (oldId === null && panelsWereOpen) {
      // 从新建会话页创建出会话：延续用户在新会话页打开的面板，而不是被目标
      // 目录历史布局复位（否则刚点开的终端会静默消失）。
      layout.showFileTree = showFileTree.value;
      layout.showTerminal = showTerminal.value;
      saveLayout();
    } else {
      showTerminal.value = layout.showTerminal;
      showFileTree.value = layout.showFileTree;
    }
  } else {
    // 回到新建会话页：不复用上一个会话的面板状态，给用户一个干净的新建页
    // （否则会直接弹出上个会话的文件树/终端，且 cwd 落到 home）。
    showFileTree.value = false;
    showTerminal.value = false;
  }
});

onMounted(async () => {
  homeDir().then(h => { cachedHomeDir.value = h; }).catch(() => {});
  await store.loadSessions();
  if (store.activeSessionId) {
    const key = currentLayoutKey();
    switchDirectory(key);
    showTerminal.value = layout.showTerminal;
    showFileTree.value = layout.showFileTree;
  }
  // App tabs: re-align child webviews on window resize and restore the
  // previously active tab when returning to the workspace.
  appTabs.init();
  appTabs.restore();

  // Desktop pet: the pet chat window owns its own session but persists it, so
  // when it creates or touches a conversation the sidebar here must refresh to
  // show it. The event is broadcast from the pet window (see usePetChat).
  listen(PET_SESSION_CHANGED_EVENT, () => {
    store.loadSessions().catch(() => {});
  })
    .then((un) => { petSessionUnlisten = un; })
    .catch(() => {});

  // Desktop pet: "open in main window" from the popup. Bring the session it was
  // talking in into this window's view (leaving the task board if needed), so
  // the user lands on the full transcript instead of the minimal popup.
  listen<{ id: string | null; route?: string }>(PET_OPEN_IN_MAIN_EVENT, (e) => {
    const { id, route: target } = e.payload ?? {};
    // A route request (e.g. "open Settings → Local models") needs no session:
    // the popup uses it to hand the user off to a page it cannot render itself.
    if (target) {
      router.push(target).catch(() => {});
      if (id) store.loadSessions().then(() => store.selectSession(id)).catch(() => {});
      return;
    }
    if (!id) return;
    store.loadSessions()
      .then(() => {
        store.selectSession(id);
        if (route.path !== "/") router.push("/");
      })
      .catch(() => {});
  })
    .then((un) => { petOpenUnlisten = un; })
    .catch(() => {});
});

// ---- Resizable sidebar ----
const sidebarResize = useDragResize({
  direction: "horizontal",
  minSize: 180,
  defaultSize: 270,
  initialSize: layout.sidebarWidth,
  onDragEnd: (size) => { layout.sidebarWidth = size; },
});

// ---- Resizable workspace panel（右侧：文件树 + 编辑器）----
// 拖拽调整的是右侧工作区的宽度；对话区占据剩下的空间。句柄在它左边，所以
// 仍然是 reversed（向左拖 = 变宽）。
const workspaceResize = useDragResize({
  direction: "horizontal",
  minSize: 300,
  defaultSize: 420,
  reversed: true,
  initialSize: layout.workspaceWidth,
  onDragEnd: (size) => { layout.workspaceWidth = size; },
});

// Sync resize sizes when layout changes (session switch)
watch(() => layout.sidebarWidth, (w) => { sidebarResize.size.value = w; });
watch(() => layout.workspaceWidth, (w) => { workspaceResize.size.value = w; });

// App webview visibility is managed globally by a router guard
// (see src/router/index.ts): leaving the workspace routes hides every app
// webview, returning re-shows the active one. `dispose()` clears any
// un-closed webviews when this layout is torn down.
onBeforeUnmount(() => {
  appTabs.dispose();
  if (petSessionUnlisten) {
    try { petSessionUnlisten(); } catch {}
    petSessionUnlisten = null;
  }
  if (petOpenUnlisten) {
    try { petOpenUnlisten(); } catch {}
    petOpenUnlisten = null;
  }
});
</script>

<template>
  <div class="flex flex-col h-screen bg-[#f2f3f5] dark:bg-[#101015] relative overflow-hidden">

    <!-- ═══ Top Nav Bar (always visible) ═══ -->
    <div
      data-tauri-drag-region
      class="flex-shrink-0 h-8 flex items-center px-4 w-full border-b border-gray-200/60 bg-white/90 backdrop-blur-sm z-30"
      style="-webkit-app-region: drag"
    >
      <!-- macOS traffic-light spacer (hidden on Windows/Linux custom title bar) -->
      <div v-if="isMac" class="w-[70px] flex-shrink-0" />

      <!-- Sidebar toggle: hide when pinned, show when hidden -->
      <button
        v-if="sidebarPinned && !appTabs.activeTabId"
        @click="setSidebarPinned(false)"
        class="p-1.5 rounded-lg text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors duration-150"
        style="-webkit-app-region: no-drag"
        :title="$t('workspace.hideSidebar')"
      >
        <PanelLeftOpen :size="18" />
      </button>
      <button
        v-if="!sidebarPinned && !appTabs.activeTabId"
        @click="setSidebarPinned(true)"
        @mouseenter="sidebarHover = true"
        class="p-1.5 rounded-lg text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors duration-150"
        style="-webkit-app-region: no-drag"
        :title="$t('workspace.showSidebar')"
      >
        <PanelLeftClose :size="18" />
      </button>

      <SearchButton />
      <div class="flex-1" />

      <!-- File explorer toggle -->
      <button
        v-if="!isBoard && !appTabs.activeTabId && !!activeDirectory"
        @click="toggleFileTree"
        class="p-1.5 rounded-lg transition-colors duration-150 ml-1"
        :class="showFileTree ? 'text-gray-700 bg-gray-200 hover:bg-gray-300' : 'text-gray-400 hover:text-gray-600 hover:bg-gray-100'"
        style="-webkit-app-region: no-drag"
        :title="showFileTree ? $t('workspace.closeExplorer') : $t('workspace.openExplorer')"
      >
        <FolderTree :size="18" />
      </button>

      <!-- Terminal toggle -->
      <button
        v-if="!isBoard && !appTabs.activeTabId && !!activeDirectory"
        @click="toggleTerminal"
        class="p-1.5 rounded-lg transition-colors duration-150 ml-0.5"
        :class="showTerminal ? 'text-gray-700 bg-gray-200 hover:bg-gray-300' : 'text-gray-400 hover:text-gray-600 hover:bg-gray-100'"
        style="-webkit-app-region: no-drag"
        :title="$t('workspace.toggleTerminal')"
      >
        <Terminal :size="18" />
      </button>

      <!-- Window controls (Windows/Linux: replaces native title bar buttons) -->
      <WindowControls />
    </div>

    <!-- ═══ App Tabs Bar (browser-like tabs for quick-launch apps) ═══ -->
    <AppTabsBar />

    <!-- ═══ Sidebar Overlay: slides in from left when hovering toggle ═══ -->
    <Transition name="sidebar-slide">
      <div
        v-if="sidebarHover && !sidebarPinned"
        class="fixed top-8 left-0 z-40 h-[calc(100vh-32px)] flex flex-col py-0 pl-[3px] pr-[3px] pb-[3px]"
        :style="{ width: (sidebarResize.size.value + 6) + 'px' }"
        @mouseleave="sidebarHover = false"
      >
        <div class="flex-1 min-h-0">
          <Sidebar class="h-full" />
        </div>
      </div>
    </Transition>

    <!-- ═══ Body ═══ -->
    <div class="flex flex-1 min-h-0">
      <!-- Pinned Sidebar (with smooth width transition) -->
      <!-- contain:layout isolates sidebar from main-content reflow during animation -->
      <div
        class="transition-[width] duration-200 ease-out flex-shrink-0 flex flex-col py-0 px-[3px] pb-[3px] will-change-[width]"
        :class="sidebarPinned ? 'contain-layout' : 'w-0 overflow-hidden'"
        :style="sidebarPinned
          ? { width: (sidebarResize.size.value + 6) + 'px' }
          : undefined"
      >
        <div class="flex-1 min-h-0">
          <Sidebar class="h-full" />
        </div>
      </div>

      <!-- Resize handle between sidebar and main content -->
      <div
        v-if="sidebarPinned"
        class="w-px flex-shrink-0 cursor-col-resize transition-colors rounded-full hover:bg-blue-400/40"
        :class="sidebarResize.isDragging.value ? 'bg-blue-400' : 'bg-transparent'"
        @mousedown="sidebarResize.startDrag"
      />

      <!-- Main content area：纵向分为上下两层。上层是「左侧对话 + 右侧工作区」，
           下层是横跨整个内容区宽度的终端。
           对话区固定在左边（也是视线与操作的落点），文件树/编辑器从右侧展开。 -->
      <div class="flex-1 flex flex-col min-w-0 min-h-0 gap-[3px]">
        <!-- Upper row: chat + workspace -->
        <div class="flex-1 flex min-w-0 min-h-0 gap-[3px]">
          <!-- Chat / Session View（始终在左，占满未被工作区占用的宽度） -->
          <div
            class="rounded-lg overflow-hidden bg-white shadow-[0_0_0_1px_rgba(0,0,0,0.04)] flex flex-col min-h-0 flex-1 min-w-0"
          >
            <KeepAlive :max="20">
              <SessionView
                v-if="!isBoard"
                :key="store.activeSessionId || '__new__'"
                :session-id="store.activeSessionId || ''"
                :compact="showWorkspaceArea"
              />
              <TaskBoardView v-else />
            </KeepAlive>
          </div>

          <!-- Resize handle between chat and workspace -->
          <div
            v-if="showWorkspaceArea"
            class="w-px flex-shrink-0 cursor-col-resize transition-colors rounded-full hover:bg-blue-400/40"
            :class="workspaceResize.isDragging.value ? 'bg-blue-400' : 'bg-transparent'"
            @mousedown="workspaceResize.startDrag"
          />

          <!-- Workspace Panel（右侧：文件树 + 编辑器）。
               v-show (not v-if): mounting/unmounting the whole workspace on every
               explorer toggle is what made opening/closing it freeze for seconds —
               it re-imported Monaco and re-scanned the file tree each time.
               Keeping it mounted means those expensive resources persist. -->
          <div
            v-show="showWorkspaceArea"
            class="flex-shrink-0 flex min-h-0"
            :style="{ width: workspaceResize.size.value + 'px' }"
          >
            <WorkspacePanel
              ref="workspacePanelRef"
              :show-file-tree="showFileTree"
              :root-path="activeDirectory"
            />
          </div>
        </div>

        <!-- Terminal resize handle（横跨全宽，拖拽调整终端高度） -->
        <div
          v-show="!isBoard && showTerminal"
          class="h-px flex-shrink-0 cursor-row-resize transition-colors rounded-full hover:bg-blue-400/40"
          :class="terminalResize.isDragging.value ? 'bg-blue-400' : 'bg-transparent'"
          @mousedown="terminalResize.startDrag"
        />

        <!-- Terminal：内容区底部横跨全宽（v-show 保持后端 shell 存活） -->
        <div
          v-show="!isBoard && showTerminal"
          class="flex-shrink-0 rounded-lg overflow-hidden shadow-[0_0_0_1px_rgba(0,0,0,0.04)]"
          :style="{ height: terminalResize.size.value + 'px' }"
        >
          <TerminalPanel
            ref="terminalPanelRef"
            :cwd="activeDirectory"
            :active="!isBoard && showTerminal && !!activeDirectory"
            @close="closeTerminalAndKill"
          />
        </div>
      </div>
    </div>

    <ConfirmDialog
      :show="showTerminalCloseConfirm"
      :title="$t('workspace.closeTerminal')"
      message="Closing the terminal will terminate all running terminal processes. This cannot be undone."
      @confirm="confirmCloseTerminal"
      @cancel="cancelCloseTerminal"
    />
  </div>
</template>

<style scoped>
.sidebar-slide-enter-active,
.sidebar-slide-leave-active {
  transition: transform 0.2s ease, opacity 0.2s ease;
}
.sidebar-slide-enter-from,
.sidebar-slide-leave-to {
  transform: translateX(-100%);
  opacity: 0;
}
</style>
