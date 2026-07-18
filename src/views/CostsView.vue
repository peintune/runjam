<script setup lang="ts">
import { onMounted, computed, ref } from "vue";
import { useRouter } from "vue-router";
import { useCostStore } from "../stores/useCostStore";
import { BarChart3, Zap, Folder, Calendar, MessageSquare, ArrowLeft } from "lucide-vue-next";

const router = useRouter();
const store = useCostStore();
const activeTab = ref<"agent" | "day" | "session" | "directory">("day");

onMounted(() => {
  store.loadAll();
});

// --- Formatting helpers ---
function fmtTokens(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M";
  if (n >= 1_000) return (n / 1_000).toFixed(1) + "K";
  return String(n);
}

function fmtCost(n: number): string {
  return "$" + n.toFixed(4);
}

function fmtDate(d: string): string {
  // d is "2026-07-18" → "Jul 18"
  const parts = d.split("-");
  if (parts.length !== 3) return d;
  const months = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"];
  const m = months[parseInt(parts[1]) - 1] || parts[1];
  return `${m} ${parseInt(parts[2])}`;
}

function shortDir(dir: string): string {
  if (dir === "Unknown" || !dir) return "Unknown";
  const parts = dir.split("/");
  return parts[parts.length - 1] || dir;
}

// --- Color palette for agents ---
const agentColors: Record<string, string> = {
  "claude-code": "#d97706",
  "codex-cli": "#2563eb",
  "gemini-cli": "#059669",
};
function agentColor(id: string): string {
  return agentColors[id] || "#6b7280";
}

// --- Bar chart data ---
const maxTokens = computed(() => {
  const vals = store.byDay.map(d => d.total_tokens);
  return vals.length ? Math.max(...vals, 1) : 1;
});

const barHeight = (tokens: number) => {
  return Math.max(4, (tokens / maxTokens.value) * 100);
};

// --- Donut chart data ---
const donutTotal = computed(() => store.byAgent.reduce((s, a) => s + a.total_tokens, 0));
function donutSegments() {
  let cumulative = 0;
  return store.byAgent.map((a, i) => {
    const pct = donutTotal.value > 0 ? a.total_tokens / donutTotal.value : 0;
    const start = cumulative;
    cumulative += pct;
    return {
      ...a,
      pct,
      dashArray: `${(pct * 100).toFixed(1)} ${(100 - pct * 100).toFixed(1)}`,
      dashOffset: -(start * 100),
    };
  });
}

// --- Days selector ---
const dayOptions = [
  { label: "7 days", value: 7 },
  { label: "14 days", value: 14 },
  { label: "30 days", value: 30 },
  { label: "90 days", value: 90 },
];

async function changeDays(d: number) {
  await store.setDays(d);
}
</script>

<template>
  <div class="flex flex-col h-screen bg-[#f8f9fb]">
    <div data-tauri-drag-region class="flex-shrink-0 h-8 w-full" style="-webkit-app-region: drag" />
    <div class="flex-1 overflow-auto">
      <div class="max-w-4xl mx-auto p-8">
        <!-- Header -->
        <div class="flex items-center justify-between mb-8">
          <div>
            <div class="flex items-center gap-3 mb-1">
              <button
                @click="router.push('/')"
                class="flex items-center gap-1.5 px-2 py-1 -ml-2 rounded-lg text-[13px] text-gray-400 hover:text-gray-700 hover:bg-gray-100 transition-colors cursor-pointer"
              >
                <ArrowLeft :size="15" />
                Back
              </button>
            </div>
            <h2 class="text-[20px] font-semibold text-gray-900 tracking-tight">Cost &amp; Token Tracking</h2>
            <p class="text-[13px] text-gray-500 mt-0.5">Monitor AI usage across agents, projects, and sessions</p>
          </div>
          <button
            @click="store.loadAll()"
            class="px-3 py-1.5 text-[12px] font-medium text-gray-600 bg-white border border-gray-200 rounded-lg hover:bg-gray-50 transition-colors cursor-pointer"
          >
            Refresh
          </button>
        </div>

        <!-- Summary cards -->
        <div class="grid grid-cols-4 gap-4 mb-8">
          <div class="bg-white rounded-xl border border-gray-100 p-5">
            <p class="text-[12px] text-gray-500 mb-1">Today</p>
            <p class="text-[13px] font-medium text-gray-400 mt-1">{{ fmtTokens(store.summary?.today_tokens ?? 0) }} tokens</p>
          </div>
          <div class="bg-white rounded-xl border border-gray-100 p-5">
            <p class="text-[12px] text-gray-500 mb-1">This Week</p>
            <p class="text-[13px] font-medium text-gray-400 mt-1">{{ fmtTokens(store.summary?.week_tokens ?? 0) }} tokens</p>
          </div>
          <div class="bg-white rounded-xl border border-gray-100 p-5">
            <p class="text-[12px] text-gray-500 mb-1">This Month</p>
            <p class="text-[13px] font-medium text-gray-400 mt-1">{{ fmtTokens(store.summary?.month_tokens ?? 0) }} tokens</p>
          </div>
          <div class="bg-white rounded-xl border border-gray-100 p-5">
            <p class="text-[12px] text-gray-500 mb-1">All Time</p>
            <p class="text-[13px] font-medium text-gray-400 mt-1">{{ fmtTokens(store.summary?.total_tokens ?? 0) }} tokens</p>
          </div>
        </div>

        <div class="grid grid-cols-5 gap-6 mb-8">
          <!-- Agent distribution donut chart -->
          <div class="col-span-2 bg-white rounded-xl border border-gray-100 p-5">
            <h3 class="text-[14px] font-medium text-gray-700 mb-4">By Agent</h3>
            <div v-if="store.byAgent.length === 0" class="flex flex-col items-center justify-center py-10 text-gray-400">
              <BarChart3 :size="32" class="mb-2 opacity-40" />
              <p class="text-[13px]">No data yet</p>
              <p class="text-[11px] mt-1">Start a session to see usage</p>
            </div>
            <div v-else class="flex items-center gap-5">
              <!-- SVG donut -->
              <svg viewBox="0 0 100 100" class="w-[100px] h-[100px] flex-shrink-0 -rotate-90">
                <circle cx="50" cy="50" r="40" fill="none" stroke="#f3f4f6" stroke-width="14" />
                <circle
                  v-for="seg in donutSegments()"
                  :key="seg.agent_id"
                  cx="50" cy="50" r="40"
                  fill="none"
                  :stroke="agentColor(seg.agent_id)"
                  stroke-width="14"
                  :stroke-dasharray="seg.dashArray"
                  :stroke-dashoffset="seg.dashOffset"
                  class="transition-all duration-500"
                />
              </svg>
              <!-- Legend -->
              <div class="flex flex-col gap-2 flex-1 min-w-0">
                <div v-for="a in store.byAgent" :key="a.agent_id" class="flex items-center gap-2">
                  <span class="w-2.5 h-2.5 rounded-sm flex-shrink-0" :style="{ backgroundColor: agentColor(a.agent_id) }" />
                  <span class="text-[12px] text-gray-700 truncate">{{ a.agent_name }}</span>
                  <span class="text-[11px] text-gray-400 ml-auto flex-shrink-0">{{ fmtTokens(a.total_tokens) }}</span>
                </div>
              </div>
            </div>
          </div>

          <!-- Daily bar chart -->
          <div class="col-span-3 bg-white rounded-xl border border-gray-100 p-5">
            <div class="flex items-center justify-between mb-4">
              <h3 class="text-[14px] font-medium text-gray-700">Daily Trend</h3>
              <div class="flex items-center gap-1">
                <button
                  v-for="o in dayOptions" :key="o.value"
                  @click="changeDays(o.value)"
                  class="px-2 py-1 text-[11px] rounded-md transition-colors cursor-pointer"
                  :class="store.selectedDays === o.value ? 'bg-gray-100 text-gray-800 font-medium' : 'text-gray-500 hover:text-gray-700'"
                >{{ o.label }}</button>
              </div>
            </div>
            <div v-if="store.byDay.length === 0" class="flex flex-col items-center justify-center py-10 text-gray-400">
              <Calendar :size="32" class="mb-2 opacity-40" />
              <p class="text-[13px]">No daily data</p>
            </div>
            <div v-else>
              <!-- SVG bar chart -->
              <div class="relative" style="height: 160px">
                <svg class="w-full h-full" preserveAspectRatio="none">
                  <!-- Grid lines -->
                  <line v-for="i in 4" :key="'g'+i" :x1="0" :y1="(i/4)*140" :x2="'100%'" :y2="(i/4)*140" stroke="#f3f4f6" stroke-width="1" />
                  <!-- Bars -->
                  <rect
                    v-for="(d, idx) in store.byDay"
                    :key="d.date"
                    :x="(idx / store.byDay.length) * 100 + '%'"
                    :y="140 - barHeight(d.total_tokens) + 'px'"
                    :width="(70 / store.byDay.length) + '%'"
                    :height="barHeight(d.total_tokens) + 'px'"
                    :rx="2"
                    fill="#6366f1"
                    class="transition-all duration-300"
                    opacity="0.85"
                  >
                    <title>{{ d.date }}: {{ fmtTokens(d.total_tokens) }} tokens</title>
                  </rect>
                </svg>
              </div>
              <!-- X-axis labels -->
              <div class="flex justify-between mt-2">
                <span v-for="d in store.byDay" :key="d.date" class="text-[10px] text-gray-400 text-center" style="width: 0; white-space: nowrap">
                  {{ fmtDate(d.date) }}
                </span>
              </div>
            </div>
          </div>
        </div>

        <!-- Tab bar for breakdown tables -->
        <div class="flex items-center gap-1 mb-4 bg-white rounded-lg border border-gray-100 p-1 w-fit">
          <button
            v-for="tab in [
              { id: 'day' as const, label: 'By Day', icon: Calendar },
              { id: 'agent' as const, label: 'By Agent', icon: Zap },
              { id: 'session' as const, label: 'By Session', icon: MessageSquare },
              { id: 'directory' as const, label: 'By Project', icon: Folder },
            ]"
            :key="tab.id"
            @click="activeTab = tab.id"
            class="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-[12px] font-medium transition-colors cursor-pointer"
            :class="activeTab === tab.id ? 'bg-gray-100 text-gray-900' : 'text-gray-500 hover:text-gray-700'"
          >
            <component :is="tab.icon" :size="13" />
            {{ tab.label }}
          </button>
        </div>

        <!-- Breakdown tables -->
        <div class="bg-white rounded-xl border border-gray-100 overflow-hidden">
          <!-- Loading -->
          <div v-if="store.loading" class="p-12 text-center text-[13px] text-gray-400">Loading...</div>

          <!-- Empty state -->
          <div v-else-if="store.summary && store.summary.total_tokens === 0" class="p-12 text-center">
            <BarChart3 :size="36" class="mx-auto mb-3 text-gray-300" />
            <p class="text-[14px] text-gray-500 font-medium mb-1">No usage data yet</p>
            <p class="text-[12px] text-gray-400">Token tracking begins automatically when you start an AI session.</p>
          </div>

          <!-- By Day table -->
          <table v-else-if="activeTab === 'day'" class="w-full">
            <thead>
              <tr class="border-b border-gray-100">
                <th class="text-left px-5 py-3 text-[12px] font-medium text-gray-500">Date</th>
                <th class="text-right px-5 py-3 text-[12px] font-medium text-gray-500">Input</th>
                <th class="text-right px-5 py-3 text-[12px] font-medium text-gray-500">Output</th>
                <th class="text-right px-5 py-3 text-[12px] font-medium text-gray-500">Total</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="d in [...store.byDay].reverse()" :key="d.date" class="border-b border-gray-50 hover:bg-gray-50/50 transition-colors">
                <td class="px-5 py-3 text-[13px] text-gray-800 font-medium">{{ fmtDate(d.date) }}</td>
                <td class="px-5 py-3 text-[13px] text-gray-600 text-right tabular-nums">{{ fmtTokens(d.input_tokens) }}</td>
                <td class="px-5 py-3 text-[13px] text-gray-600 text-right tabular-nums">{{ fmtTokens(d.output_tokens) }}</td>
                <td class="px-5 py-3 text-[13px] text-gray-900 font-medium text-right tabular-nums">{{ fmtTokens(d.total_tokens) }}</td>
              </tr>
            </tbody>
          </table>

          <!-- By Agent table -->
          <table v-else-if="activeTab === 'agent'" class="w-full">
            <thead>
              <tr class="border-b border-gray-100">
                <th class="text-left px-5 py-3 text-[12px] font-medium text-gray-500">Agent</th>
                <th class="text-right px-5 py-3 text-[12px] font-medium text-gray-500">Input</th>
                <th class="text-right px-5 py-3 text-[12px] font-medium text-gray-500">Output</th>
                <th class="text-right px-5 py-3 text-[12px] font-medium text-gray-500">Total</th>
                <th class="text-right px-5 py-3 text-[12px] font-medium text-gray-500">Sessions</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="a in store.byAgent" :key="a.agent_id" class="border-b border-gray-50 hover:bg-gray-50/50 transition-colors">
                <td class="px-5 py-3">
                  <div class="flex items-center gap-2">
                    <span class="w-2 h-2 rounded-full flex-shrink-0" :style="{ backgroundColor: agentColor(a.agent_id) }" />
                    <span class="text-[13px] text-gray-800 font-medium">{{ a.agent_name }}</span>
                  </div>
                </td>
                <td class="px-5 py-3 text-[13px] text-gray-600 text-right tabular-nums">{{ fmtTokens(a.input_tokens) }}</td>
                <td class="px-5 py-3 text-[13px] text-gray-600 text-right tabular-nums">{{ fmtTokens(a.output_tokens) }}</td>
                <td class="px-5 py-3 text-[13px] text-gray-900 font-medium text-right tabular-nums">{{ fmtTokens(a.total_tokens) }}</td>
                <td class="px-5 py-3 text-[13px] text-gray-600 text-right tabular-nums">{{ a.sessions }}</td>
              </tr>
            </tbody>
          </table>

          <!-- By Session table -->
          <table v-else-if="activeTab === 'session'" class="w-full">
            <thead>
              <tr class="border-b border-gray-100">
                <th class="text-left px-5 py-3 text-[12px] font-medium text-gray-500">Session</th>
                <th class="text-left px-5 py-3 text-[12px] font-medium text-gray-500">Agent</th>
                <th class="text-left px-5 py-3 text-[12px] font-medium text-gray-500">Project</th>
                <th class="text-right px-5 py-3 text-[12px] font-medium text-gray-500">Tokens</th>
                <th class="text-right px-5 py-3 text-[12px] font-medium text-gray-500">Msgs</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="s in store.bySession" :key="s.session_id" class="border-b border-gray-50 hover:bg-gray-50/50 transition-colors">
                <td class="px-5 py-3">
                  <span class="text-[13px] text-gray-800 font-medium font-mono">{{ s.session_id.slice(0, 8) }}...</span>
                  <p class="text-[11px] text-gray-400 mt-0.5">{{ fmtDate(s.created_at.slice(0, 10)) }}</p>
                </td>
                <td class="px-5 py-3">
                  <div class="flex items-center gap-1.5">
                    <span class="w-1.5 h-1.5 rounded-full" :style="{ backgroundColor: agentColor(s.agent_name === 'Claude Code' ? 'claude-code' : s.agent_name === 'Codex CLI' ? 'codex-cli' : 'gemini-cli') }" />
                    <span class="text-[12px] text-gray-700">{{ s.agent_name }}</span>
                  </div>
                </td>
                <td class="px-5 py-3 text-[12px] text-gray-600 font-mono max-w-[200px] truncate">{{ shortDir(s.directory) }}</td>
                <td class="px-5 py-3 text-[13px] text-gray-800 font-medium text-right tabular-nums">{{ fmtTokens(s.total_tokens) }}</td>
                <td class="px-5 py-3 text-[13px] text-gray-600 text-right tabular-nums">{{ s.message_count }}</td>
              </tr>
            </tbody>
          </table>

          <!-- By Directory table -->
          <table v-else-if="activeTab === 'directory'" class="w-full">
            <thead>
              <tr class="border-b border-gray-100">
                <th class="text-left px-5 py-3 text-[12px] font-medium text-gray-500">Project Directory</th>
                <th class="text-right px-5 py-3 text-[12px] font-medium text-gray-500">Input</th>
                <th class="text-right px-5 py-3 text-[12px] font-medium text-gray-500">Output</th>
                <th class="text-right px-5 py-3 text-[12px] font-medium text-gray-500">Total</th>
                <th class="text-right px-5 py-3 text-[12px] font-medium text-gray-500">Sessions</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="d in store.byDirectory" :key="d.directory" class="border-b border-gray-50 hover:bg-gray-50/50 transition-colors">
                <td class="px-5 py-3">
                  <span class="text-[13px] text-gray-800 font-medium font-mono">{{ shortDir(d.directory) }}</span>
                  <p v-if="d.directory !== 'Unknown'" class="text-[11px] text-gray-400 mt-0.5 truncate max-w-[250px]">{{ d.directory }}</p>
                </td>
                <td class="px-5 py-3 text-[13px] text-gray-600 text-right tabular-nums">{{ fmtTokens(d.input_tokens) }}</td>
                <td class="px-5 py-3 text-[13px] text-gray-600 text-right tabular-nums">{{ fmtTokens(d.output_tokens) }}</td>
                <td class="px-5 py-3 text-[13px] text-gray-900 font-medium text-right tabular-nums">{{ fmtTokens(d.total_tokens) }}</td>
                <td class="px-5 py-3 text-[13px] text-gray-600 text-right tabular-nums">{{ d.sessions }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>
    </div>
  </div>
</template>
