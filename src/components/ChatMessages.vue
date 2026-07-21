<script setup lang="ts">
import { ref, watch, nextTick, onBeforeUnmount, reactive, computed } from "vue";
import {
  ChevronDown, ChevronRight, Clock, Check, Copy,
  Wrench, MousePointerClick, RefreshCw, Quote,
} from "lucide-vue-next";
import { respondInteraction, respondPermission } from "../api/sessions";
import { useMarkdown } from "../composables/useMarkdown";
import AgentIcon from "./AgentIcon.vue";

const { render: renderMd, safeSliceForStreaming, renderMermaidBlocks, hasMermaid } = useMarkdown();

// ═══ Types ═══
export interface InteractionOption { key: string; label: string; is_default: boolean; }
export interface ToolCall {
  toolName: string;
  input: string;
  output?: string;
  status: string;
  startTime?: number;
  durationMs?: number;
  title?: string;
}
export interface PermissionPrompt {
  requestId: string;
  prompt: string;
  options: InteractionOption[];
  sessionId: string;
}
export interface Message {
  role: "user" | "agent";
  content: string;
  thinking?: string;
  thoughtDuration?: string;
  interaction?: { prompt: string; options: InteractionOption[]; sessionId: string };
  permission?: PermissionPrompt;
  isProcessing?: boolean;
  startTime?: number;
  toolCalls?: ToolCall[];
}

// ═══ Props ═══
const props = defineProps<{ messages: Message[]; agentId?: string }>();

// ═══ Message Groups: consecutive agent messages merge into one bubble ═══
const messageGroups = computed(() => {
  const groups: { type: "user" | "agent"; items: { msg: Message; oi: number }[] }[] = [];
  for (let i = 0; i < props.messages.length; i++) {
    const msg = props.messages[i];
    const item = { msg, oi: i };
    if (msg.role === "user") {
      groups.push({ type: "user", items: [item] });
    } else {
      const last = groups[groups.length - 1];
      if (last && last.type === "agent") {
        last.items.push(item);
      } else {
        groups.push({ type: "agent", items: [item] });
      }
    }
  }
  return groups;
});

// ═══ Collapsing state — track EXPANDED, not collapsed (default: hidden) ═══
const thinkingExpanded = ref<Set<number>>(new Set());
const toolExpanded = ref<Set<string>>(new Set());
const respondedPermissions = ref<Set<string>>(new Set());
const respondedInteractions = ref<Set<string>>(new Set());

// Auto-expand thinking if its message has no content yet (still thinking phase)
function shouldAutoExpandThinking(msg: Message): boolean {
  return !msg.content && !!msg.thinking;
}

watch(
  () => props.messages,
  (msgs) => {
    for (let i = 0; i < msgs.length; i++) {
      // Auto-expand thinking that hasn't reached content phase yet
      if (shouldAutoExpandThinking(msgs[i])) {
        thinkingExpanded.value.add(i);
      }
    }
  },
  { deep: true, immediate: true },
);

function toggleThinking(idx: number) {
  if (thinkingExpanded.value.has(idx)) thinkingExpanded.value.delete(idx);
  else thinkingExpanded.value.add(idx);
}
function toggleToolCall(msgIdx: number, toolIdx: number) {
  const key = `${msgIdx}-${toolIdx}`;
  if (toolExpanded.value.has(key)) toolExpanded.value.delete(key);
  else toolExpanded.value.add(key);
}

// ═══ Typewriter / streaming state ═══
const displayMap = reactive<Record<number, { thinking: string; content: string }>>({});
const startTimes = reactive<Record<number, { thinking: number; content: number }>>({});
const frozenDurations = reactive<Record<number, { thinking: number; content: number }>>({});
const now = ref(Date.now());
const tickTimer = setInterval(() => {
  now.value = Date.now();
}, 200);
const timers: ReturnType<typeof setInterval>[] = [];
onBeforeUnmount(() => {
  clearInterval(tickTimer);
  timers.forEach(clearInterval);
});
const thinkingRefs = ref<Record<number, HTMLElement>>({});

function startTypewriter(
  idx: number,
  fullText: string,
  field: "thinking" | "content",
  speed = 8,
) {
  if (!displayMap[idx]) displayMap[idx] = { thinking: "", content: "" };
  if (!startTimes[idx]) startTimes[idx] = { thinking: 0, content: 0 };
  if (!frozenDurations[idx]) frozenDurations[idx] = { thinking: 0, content: 0 };
  if (startTimes[idx][field] === 0) startTimes[idx][field] = Date.now();

  const current = displayMap[idx][field];
  if (current.length >= fullText.length) return;

  const timer = setInterval(() => {
    if (!displayMap[idx]) {
      clearInterval(timer);
      return;
    }
    const cur = displayMap[idx][field];
    if (cur.length < fullText.length) {
      const chunk = 8 + Math.floor(Math.random() * 8);
      displayMap[idx][field] = fullText.substring(
        0,
        Math.min(cur.length + chunk, fullText.length),
      );
      nextTick(() => {
        const el = thinkingRefs.value[idx];
        if (el) el.scrollTop = el.scrollHeight;
      });
    } else {
      clearInterval(timer);
      frozenDurations[idx][field] = Date.now() - startTimes[idx][field];
    }
  }, speed);
  timers.push(timer);
}

function elapsed(ms: number): string {
  const s = Math.floor(ms / 1000);
  if (s < 60) return s + "s";
  return Math.floor(s / 60) + "m " + (s % 60) + "s";
}

function thinkingLabel(msg: Message, idx: number): string {
  if (msg.thoughtDuration) return `Thought • ${msg.thoughtDuration}`;
  // Frozen thinking duration takes priority over content check
  if (frozenDurations[idx]?.thinking)
    return `Thought • ${elapsed(frozenDurations[idx].thinking)}`;
  if (msg.content) return "Thought";
  const st = startTimes[idx]?.thinking || 0;
  return st ? `Thinking • ${elapsed(now.value - st)}` : "Thinking...";
}

function formatDuration(ms: number): string {
  const s = Math.floor(ms / 1000);
  if (s < 60) return s + "s";
  return Math.floor(s / 60) + "m " + (s % 60) + "s";
}

// ═══ Render content with safe streaming slice ═══
function renderContent(idx: number, msg: Message): string {
  const fullContent = msg.content;
  const displayed = displayMap[idx]?.content;
  if (displayed === undefined || displayed.length >= fullContent.length) {
    return renderMd(fullContent, { sanitize: true });
  }
  const safeSlice = safeSliceForStreaming(displayed);
  return renderMd(safeSlice, { sanitize: true });
}

function isTyping(idx: number, msg: Message): boolean {
  const displayed = displayMap[idx]?.content;
  return displayed !== undefined && displayed.length < msg.content.length;
}

// Whether any message in the group is still active (has typing or is processing)
function isGroupActive(items: { msg: Message; oi: number }[]): boolean {
  return items.some(
    (it) => isTyping(it.oi, it.msg) || it.msg.isProcessing === true,
  );
}

// ═══ Mermaid post-processing ═══
const mermaidRenderedMessages = ref<Set<number>>(new Set());

async function handleMermaidInContent(idx: number, msg: Message) {
  if (isTyping(idx, msg)) return;
  if (!hasMermaid(msg.content)) return;
  if (mermaidRenderedMessages.value.has(idx)) return;

  mermaidRenderedMessages.value.add(idx);
  await nextTick();

  const containers = (chatEl.value as HTMLElement | null)?.querySelectorAll(
    `[data-msg-content="${idx}"]`,
  );
  if (containers && containers.length > 0) {
    await renderMermaidBlocks(containers[0] as HTMLElement);
  }
}

// ═══ Message list reactivity ═══
let lastMsgsRef: Message[] | null = null;
watch(
  () => props.messages,
  (msgs) => {
    if (msgs !== lastMsgsRef) {
      lastMsgsRef = msgs;
      for (const k of Object.keys(displayMap)) delete displayMap[Number(k)];
      for (const k of Object.keys(startTimes)) delete startTimes[Number(k)];
      for (const k of Object.keys(frozenDurations)) delete frozenDurations[Number(k)];
      mermaidRenderedMessages.value.clear();
      timers.forEach(clearInterval);
      timers.length = 0;
    }
    for (let i = 0; i < msgs.length; i++) {
      const m = msgs[i];
      if (!displayMap[i]) displayMap[i] = { thinking: "", content: "" };
      if (!startTimes[i]) startTimes[i] = { thinking: 0, content: 0 };
      if (!frozenDurations[i]) frozenDurations[i] = { thinking: 0, content: 0 };

      // Only messages explicitly marked as processing get the typewriter effect.
      // Completed/historical messages (isProcessing: false or undefined) render instantly.
      const isLive = m.isProcessing === true;

      if (isLive) {
        if (m.thinking && displayMap[i].thinking.length < m.thinking.length)
          startTypewriter(i, m.thinking, "thinking", 3);
        if (m.content && displayMap[i].content.length < m.content.length)
          startTypewriter(i, m.content, "content", 4);
      } else {
        // Message is no longer live — freeze any running durations
        if (startTimes[i].thinking && !frozenDurations[i].thinking) {
          frozenDurations[i].thinking = Date.now() - startTimes[i].thinking;
        }
        if (startTimes[i].content && !frozenDurations[i].content) {
          frozenDurations[i].content = Date.now() - startTimes[i].content;
        }
        displayMap[i].thinking = m.thinking || "";
        displayMap[i].content = m.content || "";
        // Auto-collapse thinking that now has content (done phase)
        if (m.content && thinkingExpanded.value.has(i)) {
          thinkingExpanded.value.delete(i);
        }
        if (m.content && hasMermaid(m.content)) {
          handleMermaidInContent(i, m);
        }
      }
    }
  },
  { deep: true },
);

watch(
  () => {
    return props.messages.map((_m, i) => {
      const full = props.messages[i]?.content || "";
      const displayed = displayMap[i]?.content || "";
      return displayed.length >= full.length && full.length > 0;
    });
  },
  (completed) => {
    completed.forEach((isComplete, i) => {
      if (isComplete) {
        handleMermaidInContent(i, props.messages[i]!);
      }
    });
  },
  { deep: true },
);

// ═══ DOM refs ═══
const chatEl = ref<HTMLElement | null>(null);
const copiedIndex = ref<number | null>(null);

// ═══ Code-block copy via event delegation ═══
function handleContentClick(e: MouseEvent) {
  const btn = (e.target as HTMLElement).closest("[data-copy]") as HTMLElement | null;
  if (!btn) return;

  const wrap = btn.closest(".cb-wrap");
  if (!wrap) return;
  const code = wrap.querySelector("code");
  if (!code) return;

  const txt = code.textContent || "";
  navigator.clipboard.writeText(txt).then(() => {
    btn.innerHTML = `<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="20 6 9 17 4 12"/></svg><span>Copied</span>`;
    setTimeout(() => {
      btn.innerHTML = `<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg><span>Copy</span>`;
    }, 2000);
  });
}

// ═══ Interaction / Permission handlers ═══
async function handleInteractionClick(sessionId: string, key: string, msgIdx: number) {
  try {
    await respondInteraction(sessionId, key);
    respondedInteractions.value.add(String(msgIdx));
  } catch (err) {
    console.error(err);
  }
}
async function handlePermissionClick(
  sessionId: string,
  requestId: string,
  key: string,
  msgIdx: number,
) {
  try {
    await respondPermission(sessionId, requestId, key);
    respondedPermissions.value.add(String(msgIdx));
  } catch (err) {
    console.error(err);
  }
}

// ═══ Message action bar ═══
async function copyMessage(content: string, idx: number) {
  try {
    await navigator.clipboard.writeText(content);
    copiedIndex.value = idx;
    setTimeout(() => (copiedIndex.value = null), 2000);
  } catch (err) {
    console.error("Copy failed:", err);
  }
}

// ═══ Auto-scroll ═══
watch(
  () => props.messages.length,
  async () => {
    await nextTick();
    if (chatEl.value) chatEl.value.scrollTop = chatEl.value.scrollHeight;
  },
);

// Truncate a long label for display, keep full text in title
function truncateLabel(label: string, maxLen = 32): string {
  return label.length > maxLen ? label.slice(0, maxLen) + "…" : label;
}
</script>

<template>
  <div ref="chatEl" class="space-y-6 py-2">
    <template v-for="(group, gIdx) in messageGroups" :key="gIdx">
      <!-- ── User message group (always single) ── -->
      <div
        v-if="group.type === 'user'"
        class="msg-row flex gap-3 justify-end"
      >
        <div class="msg-user-bubble max-w-[75%] px-4 py-2.5 text-[15px] leading-relaxed">
          {{ group.items[0].msg.content }}
        </div>
      </div>

      <!-- ── Agent message group (1+ messages in one bubble) ── -->
      <div v-else class="msg-row flex gap-3 justify-start">
        <div class="msg-agent-avatar shrink-0 mt-0.5">
          <AgentIcon v-if="agentId" :agent-id="agentId" :size="28" />
          <span v-else class="text-[13px] font-bold text-gray-400">A</span>
        </div>
        <div class="msg-agent-bubble max-w-[85%] px-5 py-4">
          <!-- Iterate each message in the group -->
          <template v-for="(item, ii) in group.items" :key="item.oi">
            <!-- Thinking -->
            <div
              v-if="item.msg.thinking"
              :class="ii > 0 ? 'mt-3 pt-3 border-t border-gray-100' : ''"
              class="mb-2.5"
            >
              <button
                @click="toggleThinking(item.oi)"
                class="flex items-center gap-1.5 text-[12px] font-medium mb-1.5 text-gray-400 hover:text-gray-600 transition-colors cursor-pointer"
              >
                <ChevronDown v-if="thinkingExpanded.has(item.oi)" :size="12" />
                <ChevronRight v-else :size="12" />
                <Clock :size="11" />
                {{ thinkingLabel(item.msg, item.oi) }}
              </button>
              <div
                v-if="thinkingExpanded.has(item.oi)"
                :ref="
                  (el) => {
                    if (el) thinkingRefs[item.oi] = el as HTMLElement;
                  }
                "
                class="text-[13px] text-gray-500 leading-relaxed max-h-40 overflow-y-auto rounded-xl p-3 bg-gray-50/70 border border-gray-100 whitespace-pre-wrap break-words font-mono"
              >
                {{ displayMap[item.oi]?.thinking || item.msg.thinking }}
                <span
                  v-if="
                    (displayMap[item.oi]?.thinking?.length || 0) <
                    (item.msg.thinking?.length || 0)
                  "
                  class="animate-pulse text-gray-300"
                  >▌</span
                >
              </div>
            </div>

            <!-- Tool calls for this message -->
            <div
              v-if="item.msg.toolCalls && item.msg.toolCalls.length > 0"
              class="mb-2.5 space-y-1.5"
            >
              <div
                v-for="(tc, ti) in item.msg.toolCalls"
                :key="ti"
                class="rounded-xl border border-gray-100 bg-gray-50/50 overflow-hidden"
              >
                <button
                  @click="toggleToolCall(item.oi, ti)"
                  class="w-full flex items-center gap-1.5 px-3 py-1.5 text-[12px] font-medium text-gray-600 hover:bg-gray-50 transition-colors cursor-pointer"
                >
                  <ChevronDown
                    v-if="toolExpanded.has(`${item.oi}-${ti}`)"
                    :size="11"
                  />
                  <ChevronRight v-else :size="11" />
                  <Wrench :size="12" />
                  <span class="text-gray-500 flex-1 min-w-0 truncate">{{
                    tc.title || tc.toolName || "Tool"
                  }}</span>
                  <span
                    v-if="tc.status === 'started' || tc.status === 'running'"
                    class="text-gray-400 animate-pulse"
                  >
                    running{{
                      tc.startTime ? " " + elapsed(now - tc.startTime) : "..."
                    }}
                  </span>
                  <Check
                    v-else-if="tc.status === 'completed'"
                    :size="11"
                    class="text-green-500"
                  />
                  <span
                    v-if="tc.durationMs"
                    class="text-[11px] text-gray-400 ml-auto"
                    >{{ formatDuration(tc.durationMs) }}</span
                  >
                  <span
                    v-else-if="tc.status === 'completed' && tc.startTime"
                    class="text-[11px] text-gray-400 ml-auto"
                    >{{ elapsed(now - tc.startTime) }}</span
                  >
                </button>
                <!-- Tool call details (only when expanded) -->
                <template v-if="toolExpanded.has(`${item.oi}-${ti}`)">
                  <div
                    v-if="tc.input"
                    class="px-3 py-1.5 text-[12px] text-gray-600 bg-white/50 border-t border-gray-100 font-mono truncate"
                  >
                    {{ tc.input }}
                  </div>
                  <div
                    v-if="tc.output"
                    class="px-3 py-1.5 text-[12px] text-gray-500 bg-gray-50/50 border-t border-gray-100 font-mono truncate"
                  >
                    {{ tc.output }}
                  </div>
                </template>
              </div>
            </div>

            <!-- Interaction options -->
            <div
              v-if="
                item.msg.interaction &&
                item.msg.interaction.options.length > 0 &&
                !respondedInteractions.has(String(item.oi))
              "
              class="mb-2.5 rounded-xl border border-indigo-200 bg-indigo-50/30 overflow-hidden"
            >
              <div
                class="flex items-center gap-1.5 px-3 py-1.5 text-[12px] font-medium text-indigo-700"
              >
                <MousePointerClick :size="12" />
                <span>{{ item.msg.interaction.prompt || "Select an option" }}</span>
              </div>
              <div
                class="flex flex-wrap gap-1.5 px-3 py-2 bg-white/50 border-t border-indigo-100"
              >
                <button
                  v-for="opt in item.msg.interaction.options"
                  :key="opt.key"
                  @click="
                    handleInteractionClick(
                      item.msg.interaction!.sessionId,
                      opt.key,
                      item.oi,
                    )
                  "
                  :title="opt.label"
                  class="px-3 py-1.5 rounded-lg text-[12px] font-medium transition-all duration-150 border cursor-pointer max-w-[200px] truncate"
                  :class="
                    opt.is_default
                      ? 'bg-indigo-600 text-white border-indigo-600 hover:bg-indigo-500'
                      : 'bg-white text-gray-700 border-gray-200 hover:bg-gray-50'
                  "
                >
                  {{ truncateLabel(opt.label) }}
                </button>
              </div>
            </div>

            <!-- Permission request -->
            <div
              v-if="
                item.msg.permission &&
                item.msg.permission.options.length > 0 &&
                !respondedPermissions.has(String(item.oi))
              "
              class="mb-2.5 rounded-xl border border-gray-200 bg-gray-50/50 overflow-hidden"
            >
              <div
                class="flex items-center gap-1.5 px-3 py-1.5 text-[12px] font-medium text-gray-600"
              >
                <MousePointerClick :size="12" />
                <span>{{ item.msg.permission.prompt || "Choose an option" }}</span>
              </div>
              <div
                class="flex flex-wrap gap-1.5 px-3 py-2 bg-white/50 border-t border-gray-100"
              >
                <button
                  v-for="opt in item.msg.permission.options"
                  :key="opt.key"
                  @click="
                    handlePermissionClick(
                      item.msg.permission!.sessionId,
                      item.msg.permission!.requestId,
                      opt.key,
                      item.oi,
                    )
                  "
                  :title="opt.label"
                  class="px-3 py-1.5 rounded-lg text-[12px] font-medium transition-all duration-150 border cursor-pointer max-w-[200px] truncate"
                  :class="
                    opt.is_default
                      ? 'bg-gray-900 text-white border-gray-900 hover:bg-gray-800'
                      : 'bg-white text-gray-700 border-gray-200 hover:bg-gray-50'
                  "
                >
                  {{ truncateLabel(opt.label) }}
                </button>
              </div>
            </div>

            <!-- Markdown Content for this message -->
            <div
              v-if="item.msg.content"
              :data-msg-content="item.oi"
              @click="handleContentClick"
              class="md-content prose prose-base max-w-none
                prose-p:text-[15px] prose-p:leading-[1.75] prose-p:text-[#1e1e2e] prose-p:my-2.5
                prose-headings:text-[#111127] prose-headings:font-semibold prose-headings:tracking-tight
                prose-h1:text-[22px] prose-h1:mt-6 prose-h1:mb-3
                prose-h2:text-[18px] prose-h2:mt-5 prose-h2:mb-2.5
                prose-h3:text-[15px] prose-h3:mt-4 prose-h3:mb-2
                prose-blockquote:border-l-[3px] prose-blockquote:border-indigo-200 prose-blockquote:pl-4 prose-blockquote:my-4 prose-blockquote:text-[#4a4a6a] prose-blockquote:not-italic prose-blockquote:text-[14px]
                prose-code:bg-[#f1f4f9] prose-code:text-[#c14a6b] prose-code:px-[5px] prose-code:py-[2px] prose-code:rounded-[4px] prose-code:text-[13px] prose-code:font-medium prose-code:before:content-none prose-code:after:content-none
                prose-pre:bg-transparent prose-pre:p-0 prose-pre:m-0 prose-pre:rounded-none
                prose-a:text-indigo-500 prose-a:no-underline hover:prose-a:underline prose-a:font-medium
                prose-strong:text-[#111127] prose-strong:font-semibold
                prose-ul:my-3 prose-ol:my-3 prose-li:my-1 prose-li:leading-[1.75] prose-li:text-[15px] prose-li:text-[#1e1e2e]
                prose-table:text-[13px] prose-th:border prose-th:border-[#e4e7ed] prose-th:bg-[#f8f9fc] prose-th:px-3 prose-th:py-2 prose-th:font-semibold prose-th:text-[#111127] prose-td:border prose-td:border-[#e4e7ed] prose-td:px-3 prose-td:py-2
                prose-hr:my-5 prose-hr:border-[#e4e7ed]
                prose-img:rounded-xl"
              v-html="renderContent(item.oi, item.msg)"
            />
          </template>

          <!-- Typewriter cursor for group -->
          <span
            v-if="isGroupActive(group.items)"
            class="animate-pulse text-gray-300 text-[15px]"
            >▌</span
          >

          <!-- Unknown / loading for empty group -->
          <div
            v-if="
              group.items.every(
                (it) => !it.msg.content && !it.msg.thinking,
              )
            "
            class="flex items-center gap-1 text-gray-300 text-[14px]"
          >
            <span class="animate-pulse">●</span>
            <span class="animate-pulse" style="animation-delay: 0.2s">●</span>
            <span class="animate-pulse" style="animation-delay: 0.4s">●</span>
          </div>

          <!-- ── Action bar (hover-visible) ── -->
          <div
            v-if="group.items.some((it) => !!it.msg.content)"
            class="msg-action-bar"
          >
            <!-- Status: show working timer if any msg is processing, else checkmark -->
            <template
              v-if="group.items.some((it) => it.msg.isProcessing)"
            >
              <span
                v-for="(it, _ix) in group.items"
                :key="'status-' + it.oi"
                class="text-[12px] text-gray-400"
              >
                <template v-if="it.msg.isProcessing && it.msg.startTime">
                  working {{ Math.floor((now - it.msg.startTime) / 1000) }}s
                </template>
              </span>
            </template>
            <span v-else class="flex items-center gap-1 text-green-500">
              <Check :size="12" />
            </span>
            <!-- Copy - copies last content in group -->
            <button
              @click="
                copyMessage(
                  group.items[group.items.length - 1].msg.content,
                  group.items[group.items.length - 1].oi,
                )
              "
              class="msg-action-btn"
              title="Copy message"
            >
              <Check
                v-if="
                  copiedIndex ===
                  group.items[group.items.length - 1].oi
                "
                :size="12"
              />
              <Copy v-else :size="12" />
              <span>{{
                copiedIndex === group.items[group.items.length - 1].oi
                  ? "Copied"
                  : "Copy"
              }}</span>
            </button>
            <button class="msg-action-btn" title="Retry">
              <RefreshCw :size="12" />
            </button>
            <button class="msg-action-btn" title="Quote">
              <Quote :size="12" />
            </button>
          </div>
        </div>
      </div>
    </template>
  </div>
</template>
