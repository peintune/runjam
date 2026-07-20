<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useRouter } from "vue-router";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  Download, Terminal, RefreshCw,
  Loader2, ToggleLeft, ToggleRight,
  Database, ChevronRight,
  AlertCircle, CheckCircle2, XCircle,
} from "lucide-vue-next";
import AgentIcon from "../../components/AgentIcon.vue";
import { useAgentStore } from "../../stores/useAgentStore";
import {
  getAgentStatuses, installAgent, setAgentEnabled,
  checkNodejs, getNodejsInstallGuide, testAgent,
  type AgentInfo, type AgentStatus,
} from "../../api/agents";
import { readAgentConfigModels } from "../../api/models";

const router = useRouter();
const agentStore = useAgentStore();
const agents = ref<AgentInfo[]>([]);
const installing = ref<string | null>(null);
const testing = ref<string | null>(null);
const installLogs = ref<Record<string, string[]>>({});

const nodeVersion = ref<string | null>(null);
const nodeInstallGuide = ref("");
const nodeChecking = ref(false);

const agentConfigModels = ref<Record<string, any[]>>({});
const loading = ref(true);
const refreshing = ref(false);

async function doRefresh() {
  refreshing.value = true;
  try {
    await loadAgents(true);
  } finally {
    refreshing.value = false;
  }
}

function getStatusConfig(status: AgentStatus) {
  switch (status) {
    case "not_installed":
      return { label: "Not installed", color: "text-gray-500", bg: "bg-gray-100", border: "border-gray-200", icon: XCircle };
    case "connection_failed":
      return { label: "Connection failed", color: "text-red-600", bg: "bg-red-50", border: "border-red-200", icon: AlertCircle };
    case "available":
      return { label: "Available", color: "text-emerald-600", bg: "bg-emerald-50", border: "border-emerald-200", icon: CheckCircle2 };
    default:
      return { label: "Unknown", color: "text-gray-500", bg: "bg-gray-100", border: "border-gray-200", icon: AlertCircle };
  }
}

function timeAgo(iso: string): string {
  const seconds = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
  if (seconds < 60) return 'just now';
  const mins = Math.floor(seconds / 60);
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

onMounted(() => {
  requestAnimationFrame(() => {
    loadAgents();
    checkNode();
    getInstallGuide();
  });
});

async function checkNode() {
  nodeChecking.value = true;
  try { nodeVersion.value = await checkNodejs(); } catch { nodeVersion.value = null; }
  nodeChecking.value = false;
}

async function getInstallGuide() {
  try { nodeInstallGuide.value = await getNodejsInstallGuide(); } catch { nodeInstallGuide.value = ""; }
}

async function loadAgents(forceRefresh = false) {
  loading.value = true;
  try { 
    agents.value = await getAgentStatuses(forceRefresh); 
    agentStore.agents = agents.value;
    await Promise.all(agents.value.map(a => loadAgentConfigModels(a.id)));
  } catch (e) {
    console.error("loadAgents error", e);
  } finally {
    loading.value = false;
  }
}

async function loadAgentConfigModels(agentId: string) {
  try {
    const configModels = await readAgentConfigModels(agentId);
    agentConfigModels.value[agentId] = configModels;
  } catch (e) {
    agentConfigModels.value[agentId] = [];
  }
}

async function doInstall(id: string) {
  installing.value = id;
  installLogs.value[id] = [];
  let unlisten: UnlistenFn | null = null;
  try {
    unlisten = await listen<{ status: string; message: string }>(
      `agent-install:${id}`,
      (event) => { if (!installLogs.value[id]) installLogs.value[id] = []; installLogs.value[id]!.push(`[${event.payload.status}] ${event.payload.message}`); },
    );
  } catch { /* */ }
  try {
    const info = await installAgent(id);
    const idx = agents.value.findIndex((a) => a.id === id);
    if (idx >= 0) {
      agents.value[idx].installed = info.installed;
      agents.value[idx].version = info.version;
      if (info.installed) {
        agents.value[idx].status = "available";
      }
    }
  } catch (err) { if (!installLogs.value[id]) installLogs.value[id] = []; installLogs.value[id]!.push(`[error] ${err}`); }
  if (unlisten) unlisten();
  installing.value = null;
  await loadAgents();
}

async function doTest(id: string) {
  testing.value = id;
  try {
    const result = await testAgent(id);
    if (!installLogs.value[id]) installLogs.value[id] = [];
    installLogs.value[id]!.push(`[test] ${result.message}`);
  } catch (err) {
    if (!installLogs.value[id]) installLogs.value[id] = [];
    installLogs.value[id]!.push(`[test error] ${err}`);
  }
  testing.value = null;
  // Reload from DB so cached status + last_tested_at are consistent
  await loadAgents();
}

async function toggleEnabled(id: string, enabled: boolean) {
  try {
    await setAgentEnabled(id, enabled);
    const idx = agents.value.findIndex((a) => a.id === id);
    if (idx >= 0) agents.value[idx].enabled = enabled;
    const storeIdx = agentStore.agents.findIndex((a: any) => a.id === id);
    if (storeIdx >= 0) agentStore.agents[storeIdx].enabled = enabled;
  } catch (err) { console.error(err); }
}

function goToDetail(id: string) {
  router.push(`/settings/agents/${id}`);
}
</script>

<template>
  <div class="flex flex-col h-full">
    <div class="flex items-center justify-between mb-6">
      <h2 class="text-lg font-semibold text-gray-900">Agents</h2>
      <button
        @click="doRefresh"
        :disabled="refreshing"
        class="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-medium bg-gray-50 border border-gray-200 text-gray-600 hover:bg-gray-100 disabled:opacity-50 transition-all duration-150 cursor-pointer active:scale-[0.98]"
      >
        <RefreshCw :size="14" :class="{ 'animate-spin': refreshing }" />
        {{ refreshing ? 'Refreshing...' : 'Refresh' }}
      </button>
    </div>

    <div class="flex-1 overflow-y-auto space-y-3">
      <!-- Skeleton while detecting -->
      <template v-if="loading && agents.length === 0">
        <div v-for="i in 3" :key="'sk-' + i" class="rounded-2xl border border-gray-100 bg-white shadow-sm overflow-hidden">
          <div class="flex items-center gap-4 p-5 animate-pulse">
            <div class="w-12 h-12 rounded-2xl bg-gray-100 flex-shrink-0" />
            <div class="flex-1 space-y-2">
              <div class="h-4 bg-gray-100 rounded-lg w-32" />
              <div class="h-3 bg-gray-50 rounded-lg w-20" />
            </div>
            <div class="flex items-center gap-2">
              <div class="h-7 bg-gray-100 rounded-lg w-16" />
              <div class="h-7 bg-gray-100 rounded-lg w-16" />
            </div>
          </div>
        </div>
        <div class="flex items-center justify-center pt-4 gap-2 text-[13px] text-gray-400">
          <Loader2 :size="15" class="animate-spin" />
          检测 Agent 中...
        </div>
      </template>

      <!-- Real agent cards -->
      <template v-else>
        <div
          v-for="agent in agents"
          :key="agent.id"
          class="rounded-2xl border border-gray-100 bg-white shadow-sm overflow-hidden transition-all duration-150 hover:border-gray-200 hover:shadow-md"
        >
          <div
            class="flex items-center gap-4 p-5 cursor-pointer hover:bg-gray-50/50 transition-colors"
            @click="goToDetail(agent.id)"
          >
            <AgentIcon :agent-id="agent.id" :size="40" />
            <div class="flex-1 min-w-0">
              <div class="flex items-center gap-2 mb-0.5">
                <span class="text-[15px] font-semibold text-gray-900 tracking-tight">{{ agent.display_name }}</span>
                <span :class="['w-1.5 h-1.5 rounded-full flex-shrink-0', agent.status === 'available' ? 'bg-emerald-500' : agent.status === 'connection_failed' ? 'bg-red-500' : 'bg-gray-400']" />
              </div>
              <div class="flex items-center gap-3">
                <p class="text-[13px] text-gray-400">
                  {{ agent.installed ? (agent.version ? 'v' + agent.version : 'Installed') : 'Not installed' }}
                </p>
                <p v-if="agent.last_tested_at" class="text-[12px] text-gray-300">
                  Tested {{ timeAgo(agent.last_tested_at) }}
                </p>
                <span
                  v-if="(agentConfigModels[agent.id] || []).length > 0"
                  class="inline-flex items-center gap-1 text-[12px] text-indigo-500 font-medium"
                >
                  <Database :size="12" />
                  {{ (agentConfigModels[agent.id] || []).length }} models
                </span>
              </div>
            </div>

            <div class="flex items-center gap-2 flex-shrink-0">
              <span
                :class="[
                  'inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-[12px] font-medium border',
                  getStatusConfig(agent.status).bg,
                  getStatusConfig(agent.status).border,
                  getStatusConfig(agent.status).color,
                ]"
              >
                <component :is="getStatusConfig(agent.status).icon" :size="12" />
                {{ getStatusConfig(agent.status).label }}
              </span>

              <button
                @click.stop="toggleEnabled(agent.id, !agent.enabled)"
                :class="[
                  'flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-medium transition-all duration-150 border cursor-pointer active:scale-[0.98]',
                  agent.enabled
                    ? 'bg-emerald-50 border-emerald-200 text-emerald-700 hover:bg-emerald-100'
                    : 'bg-gray-50 border-gray-200 text-gray-500 hover:bg-gray-100',
                ]"
              >
                <ToggleRight v-if="agent.enabled" :size="14" />
                <ToggleLeft v-else :size="14" />
                {{ agent.enabled ? 'Enabled' : 'Disabled' }}
              </button>

              <button
                v-if="agent.installed"
                @click.stop="doTest(agent.id)"
                :disabled="testing === agent.id"
                :class="[
                  'flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-medium transition-all duration-150 border cursor-pointer active:scale-[0.98]',
                  testing === agent.id
                    ? 'bg-gray-50 border-gray-200 text-gray-400 cursor-not-allowed'
                    : 'bg-blue-50 border-blue-200 text-blue-700 hover:bg-blue-100',
                ]"
              >
                <Loader2 v-if="testing === agent.id" :size="14" class="animate-spin" />
                <Terminal v-else :size="14" />
                Test
              </button>

              <button
                v-if="!agent.installed"
                @click.stop="doInstall(agent.id)"
                :disabled="installing === agent.id"
                class="flex items-center gap-1.5 px-3.5 py-1.5 rounded-lg text-[12px] font-semibold bg-indigo-600 text-white hover:bg-indigo-700 active:scale-[0.98] disabled:opacity-50 transition-all duration-150 cursor-pointer shadow-sm"
              >
                <Loader2 v-if="installing === agent.id" :size="14" class="animate-spin" />
                <Download v-else :size="14" />
                Install
              </button>

              <button
                @click.stop="goToDetail(agent.id)"
                class="p-1.5 rounded-lg text-gray-300 hover:text-gray-500 hover:bg-gray-100 transition-colors duration-150 cursor-pointer"
              >
                <ChevronRight :size="18" />
              </button>
            </div>
          </div>

          <div v-if="installLogs[agent.id]?.length" class="border-t border-gray-100 bg-gray-50/50 px-5 py-3">
            <pre class="text-[12px] text-gray-500 font-mono leading-relaxed max-h-32 overflow-y-auto">{{ installLogs[agent.id]!.join('\n') }}</pre>
          </div>
        </div>

        <div v-if="agents.length === 0" class="flex flex-col items-center justify-center py-12 text-gray-400">
          <Loader2 :size="48" class="mb-3 opacity-50" />
          <p class="text-sm">No agents detected</p>
        </div>
      </template>
    </div>
  </div>
</template>