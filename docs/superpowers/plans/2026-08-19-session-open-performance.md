# 会话打开/切换卡顿优化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 消除重新点击已打开会话时的数秒卡顿 —— 降低大消息列表的布局/重渲成本、消除激活路径的强制布局、让 mermaid 渲染结果跨会话复用、将超长历史折叠为"加载更早"按钮。

**Architecture:** 六项局部优化叠加，全部在前端（Vue 3）：(1) `content-visibility: auto` 让浏览器原生跳过离屏消息组的布局；(2) 消息数组 `messages` 与状态数组 `state.messages` 建立同一响应式引用，ACP 事件原地修改自动触发视图更新，不再每次整体 `[...state.messages]` 换数组（换数组会让 ChatMessages 的引用 watcher 误判"换会话"，清空 mermaid 渲染缓存并重建 displayMap）；(3) 激活路径只在首次/换会话时滚动到底，滚动与 scroll 事件处理用 rAF 节流去掉强制布局；(4) module 级 mermaid SVG 缓存（图源码 → SVG），重挂载/重激活直接注入；(5) 超长会话头部组折叠成一个"查看更早消息"按钮；(6) KeepAlive 上限 10 → 20 减少重挂载。

**Tech Stack:** Vue 3 (script setup), TypeScript, Tauri 2, mermaid, marked/hljs/DOMPurify

**Spec:** 设计来自 2026-08-19 会话内 brainstorming（无独立 spec 文件；方案 A/B/C/D + 历史分页已由用户确认）

## Global Constraints

- 前端无测试框架（不引入 vitest/jest）；每任务验证门禁为 `npx vue-tsc --noEmit`（从仓库根目录运行），最终手动验证用 `npm run tauri dev` + 内置 diag（`Cmd/Ctrl+Shift+D` 看 long tasks）。
- `messages.value` 与 `state.messages` 必须保持**同一数组引用**（Task 3 起），这是 ChatMessages 不误判会话切换的前提。
- ChatMessages 的惰性渲染（IntersectionObserver + 140px ghost placeholder）与折叠（Task 5）是两套独立机制：前者决定"组是否渲染真实内容"，后者决定"组是否进入 v-for"。不要合并。
- mermaid 渲染缓存（Task 4）只缓存**成功渲染**的 SVG；渲染失败的走原有 code-block fallback，不缓存。
- 提交风格与仓库历史一致：`perf: ...`（前例：`perf: cache streaming markdown slices to reduce re-parsing during typewriter`）。

---

### Task 1: content-visibility 接管离屏消息组布局

**Files:**
- Modify: `src/components/ChatMessages.vue`（文件末尾追加 `<style scoped>` 块，当前无 style 块）

**Interfaces:**
- Consumes: 无
- Produces: 无（纯 CSS）。`content-visibility: auto` 生效后，已渲染但离屏的 `.msg-row` 不再参与布局/绘制，`scrollHeight`/`scrollTop` 读取只按估算尺寸参与，KeepAlive 恢复组件时离屏组几乎免费。

- [x] **Step 1: 追加 style 块**

在 `src/components/ChatMessages.vue` 文件末尾（`</template>` 之后）追加：

```html
<style scoped>
/* ── content-visibility：让浏览器原生跳过离屏消息组的布局/渲染 ──
   IntersectionObserver 决定"是否渲染真实内容"（placeholder ↔ 完整 DOM），
   这里决定"已渲染但离屏的组不再参与布局/绘制"。KeepAlive 恢复组件时，
   离屏组不再逐个 layout——这是"重新点击会话卡几秒"的主要来源之一。
   contain-intrinsic-size: auto 140px：离屏时用浏览器记忆的上次真实尺寸占位
   （auto 关键字），滚动条高度真实、无跳变。auto 不支持的旧内核（老
   WKWebView）会整体忽略这两条声明，退化为现状——无优化但不破坏功能。 */
.msg-row {
  content-visibility: auto;
  contain-intrinsic-size: auto 140px;
}
</style>
```

- [x] **Step 2: 类型检查**

Run: `cd /Users/guizhan/work/code/runjam && npx vue-tsc --noEmit`
Expected: 无错误（CSS 不参与类型检查，此步骤确认没有误伤其他内容）

- [x] **Step 3: 提交**

```bash
git add src/components/ChatMessages.vue
git commit -m "perf: use content-visibility to skip offscreen message layout"
```

---

### Task 2: 消息数组与状态数组建立同一响应式引用

**Files:**
- Modify: `src/components/SessionView.vue`（import 行、`getSessionState`、`loadSessionMessages`、18 处 `messages.value = [...state.messages]`）

**Interfaces:**
- Consumes: 无
- Produces:
  - `function syncMessagesToView(state: SessionState): void` — 让 `messages.value` 与 `state.messages` 指向同一数组；已同引用时 no-op。Task 3 的 `onActivated` 用 `messages.value !== state.messages` 判断"首次/换会话"。
  - `state.messages` 从创建起即为 `reactive()` 数组：对它的 `push`/原地属性修改自动触发 `messages` 视图更新。

原理：`ref` 赋值为一个已是 `reactive` proxy 的数组时，`messages.value` 直接持有该 proxy（不会二次包装）。因此只要 `state.messages` 是 proxy，`messages.value` 与之同引用后，`handleAcpEventInner` 里对 `state.messages` 的所有原地修改都会自动反映到视图——事件分支里那 14 行 `messages.value = [...state.messages]` 全都可以删除。

- [x] **Step 1: import 添加 reactive**

在 `src/components/SessionView.vue` 第 2 行：

```ts
import { ref, watch, onMounted, onBeforeUnmount, onActivated, computed, nextTick } from "vue";
```

改为：

```ts
import { ref, watch, onMounted, onBeforeUnmount, onActivated, computed, nextTick, reactive } from "vue";
```

- [x] **Step 2: getSessionState 用 reactive 初始化 state.messages**

找到 `getSessionState`（约 856-875 行），将：

```ts
function getSessionState(sessionId: string): SessionState {
  let state = sessionStates.get(sessionId);
  if (!state) {
    state = {
      messages: msgStore.getMessages(sessionId) || [],
```

改为：

```ts
function getSessionState(sessionId: string): SessionState {
  let state = sessionStates.get(sessionId);
  if (!state) {
    state = {
      // reactive()：状态数组本身是响应式 proxy。对它的 push / 原地属性
      // 修改会自动触发 messages 视图更新（Task 2 建立同引用后）。
      // reactive(proxy) 返回自身，不会二次包装。
      messages: reactive(msgStore.getMessages(sessionId) || []),
```

- [x] **Step 3: 新增 syncMessagesToView 函数**

在 `getSessionState` 函数定义之后（约 875 行后）插入：

```ts
/**
 * 让视图消息数组与状态数组建立同一引用。状态数组是 reactive proxy，
 * 之后 handleAcpEvent 等对 state.messages 的原地修改/追加会自动触发视图
 * 更新（Vue 追踪 proxy 的数组变更），无需每次整体换数组——整体换数组会
 * 让 ChatMessages 的引用 watcher 误判为"换会话"，清空 mermaid 渲染缓存并
 * 重建 displayMap，是重新打开长会话卡顿的主因之一。
 * 引用已相同时为 no-op（O(1)），可安全地在每个事件分支调用。
 */
function syncMessagesToView(state: SessionState) {
  if (messages.value !== state.messages) {
    messages.value = state.messages as unknown as Message[];
  }
}
```

- [x] **Step 4: loadSessionMessages 用 reactive + sync**

找到 `loadSessionMessages`（约 877-901 行），将：

```ts
      state.messages = loadedMessages;
      state.loaded = true;
      msgStore.setMessages(sessionId, [...state.messages]);
      if (effectiveSessionId.value === sessionId) {
        messages.value = [...state.messages];
      }
```

改为：

```ts
      state.messages = reactive(loadedMessages);
      state.loaded = true;
      msgStore.setMessages(sessionId, [...state.messages]);
      if (effectiveSessionId.value === sessionId) {
        syncMessagesToView(state);
      }
```

- [x] **Step 5: 替换所有 `messages.value = [...state.messages]` 为 `syncMessagesToView(state)`**

以下 18 处逐一替换（共 16 处直接替换为 `syncMessagesToView(state);`；其中 746/813 两处见 Step 6 的完整改写）。所有 `msgStore.setMessages(...)` 行保留不动。

| 行号 | 位置 | 原代码 | 改为 |
|---|---|---|---|
| 759 | `initSession` | `messages.value = [...state.messages];` | `syncMessagesToView(state);` |
| 892 | `loadSessionMessages` | （Step 4 已处理） | — |
| 1004 | `handleSendFailure`（retry 分支） | `messages.value = [...state.messages];` | `syncMessagesToView(state);` |
| 1023 | `handleSendFailure`（final 分支） | `messages.value = [...state.messages];` | `syncMessagesToView(state);` |
| 1108 | `handleAcpEventInner` case "start" | `messages.value = [...state.messages];` | `syncMessagesToView(state);` |
| 1164 | `handleAcpEventInner` case "thinking" | `messages.value = [...state.messages];` | `syncMessagesToView(state);` |
| 1205 | `handleAcpEventInner` case "text" | `messages.value = [...state.messages];` | `syncMessagesToView(state);` |
| 1256 | `handleAcpEventInner` case "tool_call" | `messages.value = [...state.messages];` | `syncMessagesToView(state);` |
| 1302 | `handleAcpEventInner` case "tool_result" | `messages.value = [...state.messages];` | `syncMessagesToView(state);` |
| 1322 | `handleAcpEventInner` case "permission_request" | `messages.value = [...state.messages];` | `syncMessagesToView(state);` |
| 1339 | `handleAcpEventInner` case "interaction_request" | `messages.value = [...state.messages];` | `syncMessagesToView(state);` |
| 1400 | `handleAcpEventInner` case "finish" | `messages.value = [...state.messages];` | `syncMessagesToView(state);` |
| 1762 | `handleSend` restart error 分支 | `messages.value = [...state.messages];` | `syncMessagesToView(state);` |
| 1773 | `handleSend` 推送 user 消息后 | `messages.value = [...state.messages];` | `syncMessagesToView(state);` |
| 1826 | `sendInput` catch 分支 | `messages.value = [...state.messages];` | `syncMessagesToView(state);` |
| 1938 | `stopCurrentSession` | `messages.value = [...state.messages];` | `syncMessagesToView(state);` |

> 替换注意：这些行的作用域内变量名不同（`state` / `sid` 的 `state` / `st` 等），替换时使用该作用域内已有的 SessionState 变量。例如 1826 所在 catch 块内变量名为 `state`；1938 所在函数内变量名为 `state`；1004/1023 在 `handleSendFailure(sessionId, state, ...)` 内变量名为 `state`。若某处局部变量名不是 `state`（如 813 行 legacy watch 里是 `state`），确认后使用对应变量。

- [x] **Step 6: 改写 onActivated 与 legacy watch 中的两处**

`onActivated`（740-751 行）：

```ts
onActivated(() => {
  if (props.sessionId) {
    const state = getSessionState(props.sessionId);
    // Only sync messages if they changed while deactivated (ACP events may
    // have added new messages). Avoids expensive re-render of cached DOM.
    if (state.messages.length !== messages.value.length) {
      messages.value = [...state.messages];
    }
    isProcessing.value = state.isProcessing;
    scrollToBottom();
  }
});
```

改为：

```ts
onActivated(() => {
  if (props.sessionId) {
    const state = getSessionState(props.sessionId);
    // 后台会话可能继续收到 ACP 事件。建立同引用（Task 2）即可让新增消息
    // 自动反映到视图；引用不同 = 首次激活或换会话，需滚动到底显示最新。
    const firstTime = messages.value !== state.messages;
    syncMessagesToView(state);
    isProcessing.value = state.isProcessing;
    if (firstTime) scrollToBottom();
  }
});
```

legacy watch（805-830 行）内：

```ts
    const state = getSessionState(newId);
    messages.value = [...state.messages];
    isProcessing.value = state.isProcessing;
```

改为：

```ts
    const state = getSessionState(newId);
    syncMessagesToView(state);
    isProcessing.value = state.isProcessing;
```

- [x] **Step 7: 类型检查**

Run: `cd /Users/guizhan/work/code/runjam && npx vue-tsc --noEmit`
Expected: 无错误

> 自查点：`grep -n "messages.value = \[...state.messages\]" src/components/SessionView.vue` 应只剩 0 处（line 826 的 `messages.value = []` 保留，是主动清空视图）。`grep -n "syncMessagesToView" src/components/SessionView.vue` 应约 19 处（1 定义 + 18 调用）。

- [x] **Step 8: 提交**

```bash
git add src/components/SessionView.vue
git commit -m "perf: keep message array reference stable across ACP events"
```

---

### Task 3: 激活/滚动路径去强制布局

**Files:**
- Modify: `src/components/SessionView.vue`（`scrollToBottom`、`onChatScroll`、`checkScrollPosition`）

**Interfaces:**
- Consumes: `syncMessagesToView(state)`、`messages.value !== state.messages`（来自 Task 2）
- Produces: 无新接口。行为变化：激活同会话且消息未变时保留离开时的滚动位置；`scroll` 事件每帧最多算一次布局。

- [x] **Step 1: scrollToBottom 合并双查为单次 rAF**

找到 `scrollToBottom`（712-727 行）：

```ts
function scrollToBottom() {
  // 用户手动点击"回到底部"（或切换会话）时恢复自动跟随
  stickToBottom.value = true;
  showScrollToBottom.value = false;
  nextTick(() => {
    if (messageContainer.value) {
      messageContainer.value.scrollTop = messageContainer.value.scrollHeight;
      // For completed sessions, double-check after paint
      requestAnimationFrame(() => {
        if (messageContainer.value) {
          messageContainer.value.scrollTop = messageContainer.value.scrollHeight;
        }
      });
    }
  });
}
```

改为：

```ts
function scrollToBottom() {
  // 用户手动点击"回到底部"（或切换会话）时恢复自动跟随
  stickToBottom.value = true;
  showScrollToBottom.value = false;
  // 单次 rAF 设置即可：原 nextTick + rAF 双查会对超大 DOM 触发两次强制布局
  // （读 scrollHeight + 写 scrollTop）。响应式更新在 rAF 回调前已 flush；
  // 流式期间的持续跟随由 watch(messages) 负责。
  requestAnimationFrame(() => {
    if (messageContainer.value) {
      messageContainer.value.scrollTop = messageContainer.value.scrollHeight;
    }
  });
}
```

- [x] **Step 2: onChatScroll 与 checkScrollPosition 合并为 rAF 节流**

找到 `onChatScroll` 与 `checkScrollPosition`（101-121 行），整体替换为：

```ts
// scroll 事件高频触发，同步读 scrollHeight/clientHeight 会强制布局。
// 用 rAF 节流：同一帧内多次 scroll 只算一次，一次布局读取同时算出
// stickToBottom 与 showScrollToBottom。
let scrollCheckPending = false;
function onChatScroll() {
  if (scrollCheckPending) return;
  scrollCheckPending = true;
  requestAnimationFrame(() => {
    scrollCheckPending = false;
    const el = messageContainer.value;
    if (!el) return;
    const distFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    stickToBottom.value = distFromBottom <= STICK_THRESHOLD;
    showScrollToBottom.value = distFromBottom > 100;
  });
}
```

> 删除独立的 `checkScrollPosition` 函数（其逻辑已并入）。`scrollToMessage` 里不调用它，无其他引用。

- [x] **Step 3: 类型检查**

Run: `cd /Users/guizhan/work/code/runjam && npx vue-tsc --noEmit`
Expected: 无错误（若报 `checkScrollPosition` 未使用/未定义错误，说明还有引用处，检查后删除）

- [x] **Step 4: 提交**

```bash
git add src/components/SessionView.vue
git commit -m "perf: avoid forced layout on session activation and scroll"
```

---

### Task 4: mermaid 渲染结果跨会话缓存

**Files:**
- Modify: `src/composables/useMarkdown.ts`（新增 module 级缓存，改造 `renderMermaidBlocks`）

**Interfaces:**
- Consumes: 无
- Produces: `renderMermaidBlocks(container)` 签名不变（ChatMessages 无需改动）。行为变化：同一段 mermaid 源码第二次渲染时直接注入缓存 SVG，跳过 `mermaid.run()`。

- [x] **Step 1: 新增 mermaid SVG 缓存**

在 `useMarkdown.ts` 的 `STREAMING_MD_CACHE_MAX` 定义之后（约 124 行 `clearStreamingCache` 之前）插入：

```ts
// ── Mermaid SVG 渲染缓存 ──
// mermaid.run() 单图 100ms+（布局+排版），同一张图（同一段源码）在会话
// 重挂载/重激活时会反复渲染。缓存 图源码 → SVG outerHTML，命中时直接注入，
// 跳过 mermaid.run。只缓存成功渲染的结果（失败走 code-block fallback）。
const mermaidSvgCache = new Map<string, string>();
const MERMAID_SVG_CACHE_MAX = 50;
```

- [x] **Step 2: 改造 renderMermaidBlocks 命中缓存**

找到 `renderMermaidBlocks`（222-263 行），将：

```ts
      await mermaid.default.run({ nodes: mermaidEls });
    } catch (err) {
```

改为：

```ts
      // 命中缓存：直接注入 SVG，跳过 mermaid.run（主要成本）
      const toRender: HTMLElement[] = [];
      for (const el of mermaidEls) {
        const raw = el.textContent || "";
        const cached = mermaidSvgCache.get(raw);
        const wrapper = el.closest(".mermaid-block");
        if (cached && wrapper) {
          wrapper.innerHTML = cached;
        } else {
          toRender.push(el);
        }
      }

      if (toRender.length > 0) {
        await mermaid.default.run({ nodes: toRender });
        // 收集刚渲染的 SVG 入缓存（key = 图源码）
        for (const el of toRender) {
          const raw = el.textContent || "";
          const wrapper = el.closest(".mermaid-block");
          const svgEl = wrapper?.querySelector("svg");
          if (svgEl) {
            mermaidSvgCache.set(raw, svgEl.outerHTML);
            if (mermaidSvgCache.size > MERMAID_SVG_CACHE_MAX) {
              const oldest = mermaidSvgCache.keys().next().value;
              if (oldest !== undefined) mermaidSvgCache.delete(oldest);
            }
          }
        }
      }
    } catch (err) {
```

> 注：`el.closest(".mermaid-block")` 在 `mermaid.run` 之后仍有效（wrapper 元素留在 DOM 中，pre 内容被替换为 svg）。缓存命中分支里 `wrapper` 已存在才注入；`wrapper` 为 null 的孤儿元素直接加入 `toRender` 正常渲染，行为与原来一致。

- [x] **Step 3: 类型检查**

Run: `cd /Users/guizhan/work/code/runjam && npx vue-tsc --noEmit`
Expected: 无错误

- [x] **Step 4: 提交**

```bash
git add src/composables/useMarkdown.ts
git commit -m "perf: cache rendered mermaid SVG across remounts"
```

---

### Task 5: 超长会话历史折叠为"查看更早"按钮

**Files:**
- Modify: `src/components/ChatMessages.vue`（script 常量/状态/函数 + 模板折叠按钮与 v-for 包裹 + 真实组加 `data-gIdx`）

**Interfaces:**
- Consumes: 无（独立于 Task 1-4，但依赖 Task 4 的 mermaid 缓存获得更好效果）
- Produces:
  - `forceRenderAll()` 行为扩展：除 `forceRender = true` 外，同时 `showFullHistory = true`（scrollToMessage 需要全部组真实渲染）。
  - 新增模板结构：v-for 内层 `v-if="gIdx >= foldedHeadCount"`；v-for 前 `查看更早的 N 条消息` 折叠按钮。

- [x] **Step 1: script 新增折叠状态与函数**

在 `forceRenderAll` 定义（175-177 行）附近，将：

```ts
function forceRenderAll() {
  forceRender.value = true;
}
```

改为：

```ts
// ═══ 历史折叠：超长会话只渲染尾部，头部收进一个"查看更早"按钮 ═══
// 惰性渲染（IntersectionObserver + placeholder）决定"组是否渲染真实内容"；
// 折叠更进一步，让头部组根本不进入 v-for —— DOM 里只有 1 个按钮而不是
// 上千个 placeholder div。展开时锚定回原位置，视口不跳。
const HISTORY_FOLD_THRESHOLD = 40; // 组数超过该值就折叠头部
const showFullHistory = ref(false);
const foldedHeadCount = computed(() => {
  if (showFullHistory.value) return 0;
  return Math.max(0, messageGroups.value.length - HISTORY_FOLD_THRESHOLD);
});

function expandHistory() {
  const anchorIdx = foldedHeadCount.value;
  showFullHistory.value = true;
  nextTick(() => {
    // 锚定：展开后滚动到原第一组的位置，避免视口跳到历史开头
    const el = chatEl.value?.querySelector(`[data-gIdx="${anchorIdx}"]`);
    if (el) el.scrollIntoView({ block: "start" });
  });
}

function forceRenderAll() {
  forceRender.value = true;
  showFullHistory.value = true; // scrollToMessage 需要全部组真实渲染
}
```

- [x] **Step 2: 模板加折叠按钮 + v-for 包裹 v-if**

在模板 v-for（711 行 `<template v-for="(group, gIdx) in messageGroups" :key="gIdx">`）之前插入折叠按钮：

```html
      <!-- ── 历史折叠：头部组收进一个按钮，点击展开并锚定回原位 ── -->
      <button
        v-if="foldedHeadCount > 0"
        @click="expandHistory"
        class="msg-row w-full flex items-center justify-center gap-2 py-3 text-[13px] text-indigo-500 hover:text-indigo-600 hover:bg-indigo-50/40 rounded-xl border border-dashed border-indigo-200 transition-colors cursor-pointer"
      >
        <ChevronDown :size="14" />
        查看更早的 {{ foldedHeadCount }} 条消息
      </button>
```

> 按钮加 `msg-row` 类，让它同样获得 Task 1 的 content-visibility 样式（无害且保持一致）。

将 v-for 开头的：

```html
    <template v-for="(group, gIdx) in messageGroups" :key="gIdx">
      <!-- ── Placeholder: cheap fixed-height, flips to full render when near viewport ── -->
      <div
        v-if="!shouldRenderGroup(group, gIdx)"
```

改为：

```html
    <template v-for="(group, gIdx) in messageGroups" :key="gIdx">
      <template v-if="gIdx >= foldedHeadCount">
      <!-- ── Placeholder: cheap fixed-height, flips to full render when near viewport ── -->
      <div
        v-if="!shouldRenderGroup(group, gIdx)"
```

并将 v-for 内层结尾（1070-1071 行）：

```html
      </template>
    </template>
```

改为：

```html
      </template>
      </template>
    </template>
```

> 缩进不必精确匹配（Vue 模板不敏感），但结构必须正确闭合：外层 `template v-for` 内新增一层 `template v-if="gIdx >= foldedHeadCount"`，包裹原有 placeholder 与 v-else 分支。

- [x] **Step 3: 真实渲染组补 data-gIdx（锚定可靠）**

展开后锚定的组可能在视口外（placeholder，已有 `data-gIdx`）也可能近视口（真实渲染）。给两个真实渲染分支的根 div 也加上 `data-gIdx`：

user 组（728 行）：

```html
      <div
        v-if="group.type === 'user'"
        class="msg-row flex gap-3 justify-end"
        :data-user-msg-index="gIdx"
      >
```

改为：

```html
      <div
        v-if="group.type === 'user'"
        class="msg-row flex gap-3 justify-end"
        :data-user-msg-index="gIdx"
        :data-gIdx="gIdx"
      >
```

agent 组（737 行）：

```html
      <div v-else class="msg-row flex gap-3 justify-start">
```

改为：

```html
      <div v-else class="msg-row flex gap-3 justify-start" :data-gIdx="gIdx">
```

- [x] **Step 4: 类型检查**

Run: `cd /Users/guizhan/work/code/runjam && npx vue-tsc --noEmit`
Expected: 无错误

- [x] **Step 5: 提交**

```bash
git add src/components/ChatMessages.vue
git commit -m "perf: fold long histories behind a load-earlier button"
```

---

### Task 6: KeepAlive 会话缓存上限提升

**Files:**
- Modify: `src/components/WorkspaceLayout.vue:301`（`<KeepAlive :max="10">`）

**Interfaces:**
- Consumes: 无
- Produces: 无。KeepAlive 最多缓存 20 个 SessionView 实例，重挂载（最重的卡顿场景：整链重渲染 + IPC 串行）出现频率减半。缓存未命中路径不变。

- [x] **Step 1: 修改 max**

`src/components/WorkspaceLayout.vue` 第 301 行：

```html
          <KeepAlive :max="10">
```

改为：

```html
          <KeepAlive :max="20">
```

- [x] **Step 2: 类型检查**

Run: `cd /Users/guizhan/work/code/runjam && npx vue-tsc --noEmit`
Expected: 无错误

- [x] **Step 3: 提交**

```bash
git add src/components/WorkspaceLayout.vue
git commit -m "perf: raise KeepAlive session cache to 20"
```

---

### Task 7: 手动验证（无代码改动）

**Files:**
- 无

- [ ] **Step 1: 启动应用**

Run: `cd /Users/guizhan/work/code/runjam && npm run tauri dev`

- [ ] **Step 2: 复现优化前场景**

1. 开启 diag：`Cmd/Ctrl+Shift+D`（记住 overlay 上的 long tasks 数值，或先 `resetDiag`）。
2. 打开/创建 20+ 个会话（触发 KeepAlive 淘汰）。
3. 点回一个消息很多（100+ 组）的旧会话，观察切换耗时与 diag 的 long tasks / maxLongTaskMs。

- [ ] **Step 3: 验证各优化点**

1. **content-visibility**：长会话打开后滚动，应无占位高度跳动；滚动条高度真实。
2. **激活不强制滚动**：打开 A 会话滚到中部 → 切到 B → 切回 A，应停留在离开时的位置（内容未变时）。
3. **同引用同步**：打开会话 A 时让 B 在后台流式（可用另一个 agent 会话发送消息），切到 B 时新消息已在；mermaid 图不重新闪烁。
4. **mermaid 缓存**：滚动经过含 mermaid 的组后再滚走滚回，图立即显示（无渲染延迟）；切换会话再回来同样立即显示。
5. **历史折叠**：长会话顶部出现"查看更早的 N 条消息"按钮，点击后展开并锚定到原位置。
6. **KeepAlive 20**：开 20 个会话后点回第 1 个，仍无重新挂载的加载态（isSessionLoading 不闪）。

- [ ] **Step 4: 回归验证**

1. 流式输出仍正常（typewriter 效果、thinking 折叠、tool call 展开）。
2. 发送消息后自动跟随滚动；上翻历史后不被打扰；点击"回到底部"按钮正常。
3. 右侧 message list（scrollToMessage）跳转正常，跳转后全部组渲染。
4. 停止会话（tool call 状态置 failed）正常。
5. 发送失败重试（try 1/3）显示正常。

- [ ] **Step 5: 确认完成**

若 diag 的 long tasks 数量/最大值显著下降且上述回归均通过，任务完成。

---

## Self-Review

**Spec 覆盖（对应用户确认的方案）：**
- 方案 A（content-visibility）→ Task 1 ✓
- 方案 B（激活路径去强制布局）→ Task 3 ✓
- 方案 C（消息增量/同引用 sync）→ Task 2 ✓
- 方案 D（mermaid 缓存 + 视口化）→ Task 4（缓存实现；视口化由既有惰性渲染机制承担，未单独加 observer——折叠后头部组不进入 v-for，视口化收益已覆盖）✓
- 业务侧历史分页 → Task 5（折叠式分页，比 slice 更安全——不破坏 ChatMessages 的 oi 索引/mermaid displayMap 映射）✓
- 方案 E 的 KeepAlive 上限提升 → Task 6 ✓（IPC 并行化未纳入，属方案 E 残留项，验证后如需再加）

**Placeholder 扫描：** 无 TBD/TODO；所有 step 含具体代码或精确行号。Task 2 的替换表引用行号基于当前文件（2778 行），若执行时行号漂移，用代码内容定位。

**Type consistency：**
- `syncMessagesToView(state: SessionState)` 在 Task 2 定义、Task 3 的 `onActivated` 消费，签名一致。
- `forceRenderAll` 在 Task 5 扩展行为，SessionView 侧调用方式（`chatMessagesRef.value?.forceRenderAll()`）不变。
- `messages.value !== state.messages` 判断在 Task 2 的 onActivated（Step 6）与 Task 3 无冲突——Task 3 不再改动 onActivated。
- Task 5 的 `expandHistory` 使用 `chatEl`（已定义于 641 行）与 `messageGroups`/`foldedHeadCount`（同文件 computed），无未定义引用。
- mermaid 缓存 key 用 `el.textContent`（图源码），与 renderer.code 输出的 `data-mermaid` 内容一致。
