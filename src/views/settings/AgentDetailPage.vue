<script setup lang="ts">
import { ref, computed, onMounted, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  ArrowLeft, Download, Trash2, ExternalLink, Terminal,
  Loader2, ToggleLeft, ToggleRight, Save,
  Database, HelpCircle, Plus, CheckCircle2, XCircle, AlertCircle,
} from "lucide-vue-next";
import AgentIcon from "../../components/AgentIcon.vue";
import { useAgentStore } from "../../stores/useAgentStore";
import {
  getAgentStatuses, installAgent, uninstallAgent, setAgentEnabled,
  checkNodejs, getNodejsInstallGuide, readAgentConfig, writeAgentConfig,
  testAgent,
  type AgentInfo, type AgentStatus,
} from "../../api/agents";
import { getModels, getAgentModels, assignModelToAgent, removeModelFromAgent, readAgentConfigModels, getProviderById, getProviderByName, getAgentModelMap, setAgentDefaultModel, type AgentModelInfo } from "../../api/models";
import { getProviderLogo } from "../../utils/providerIcons";

const router = useRouter();
const route = useRoute();
const agentStore = useAgentStore();

const agentId = computed(() => route.params.agentId as string);
const agent = ref<AgentInfo | null>(null);
const agents = ref<AgentInfo[]>([]);

const installing = ref<string | null>(null);
const uninstalling = ref<string | null>(null);
const testing = ref<string | null>(null);
const installLogs = ref<Record<string, string[]>>({});

const nodeVersion = ref<string | null>(null);
const nodeInstallGuide = ref("");

const configContent = ref<Record<string, string>>({});
const configDirty = ref<Record<string, boolean>>({});
const configSaving = ref<Record<string, boolean>>({});

const allModels = ref<any[]>([]);
const agentModels = ref<Record<string, any[]>>({});
const agentConfigModels = ref<Record<string, any[]>>({});
const addingModel = ref<Record<string, boolean>>({});
const selectedModelId = ref<Record<string, string>>({});
const useProxy = ref<Record<string, boolean>>({});
const tooltipVisible = ref(false);

const providerColorMap: Record<string, string> = {
  anthropic: 'border-orange-300 text-orange-700 bg-orange-50',
  openai: 'border-emerald-300 text-emerald-700 bg-emerald-50',
  google: 'border-blue-300 text-blue-700 bg-blue-50',
  gemini: 'border-blue-300 text-blue-700 bg-blue-50',
  deepseek: 'border-violet-300 text-violet-700 bg-violet-50',
  ollama: 'border-amber-300 text-amber-700 bg-amber-50',
  groq: 'border-rose-300 text-rose-700 bg-rose-50',
  xai: 'border-gray-300 text-gray-700 bg-gray-50',
  grok: 'border-gray-300 text-gray-700 bg-gray-50',
  moonshot: 'border-sky-300 text-sky-700 bg-sky-50',
  zhipu: 'border-indigo-300 text-indigo-700 bg-indigo-50',
  aliyun: 'border-orange-300 text-orange-700 bg-orange-50',
  baidu: 'border-blue-300 text-blue-700 bg-blue-50',
  tencent: 'border-cyan-300 text-cyan-700 bg-cyan-50',
  siliconflow: 'border-purple-300 text-purple-700 bg-purple-50',
  openrouter: 'border-teal-300 text-teal-700 bg-teal-50',
  novita: 'border-pink-300 text-pink-700 bg-pink-50',
  dashscope: 'border-orange-300 text-orange-700 bg-orange-50',
  ark: 'border-gray-300 text-gray-700 bg-gray-50',
  newapi: 'border-gray-300 text-gray-700 bg-gray-50',
};

function getProviderColorClass(providerId: string): string {
  return providerColorMap[providerId] || 'border-gray-300 text-gray-700 bg-gray-50';
}

const agentMeta: Record<string, { website: string; installManual: string; configPath: string; description: string }> = {
  "claude-code": {
    website: "https://docs.anthropic.com/en/docs/claude-code",
    installManual: "npm install -g @anthropic-ai/claude-code",
    configPath: "~/.claude/settings.json",
    description: "Anthropic's official CLI for Claude. AI-powered coding assistant that works directly in your terminal.",
  },
  "codex-cli": {
    website: "https://github.com/openai/codex",
    installManual: "npm install -g @openai/codex",
    configPath: "~/.codex/config.toml",
    description: "OpenAI's CLI coding agent. Uses GPT-4o to understand and modify your codebase from the terminal.",
  },
  "gemini-cli": {
    website: "https://github.com/google-gemini/gemini-cli",
    installManual: "npm install -g @google/gemini-cli",
    configPath: "~/.gemini/settings.json",
    description: "Google's CLI for Gemini models. Brings Gemini 2.0 Flash capabilities to your local development workflow.",
  },
};

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

onMounted(async () => {
  await checkNode();
  await getInstallGuide();
  await loadAgent();
  await loadAllModels();
});

watch(() => route.params.agentId, async () => {
  await loadAgent();
});

async function checkNode() {
  try { nodeVersion.value = await checkNodejs(); } catch { nodeVersion.value = null; }
}

async function getInstallGuide() {
  try { nodeInstallGuide.value = await getNodejsInstallGuide(); } catch { nodeInstallGuide.value = ""; }
}

async function loadAgent() {
  try {
    agents.value = await getAgentStatuses();
    agentStore.agents = agents.value;
    agent.value = agents.value.find(a => a.id === agentId.value) || null;
    if (agent.value) {
      await loadConfig(agent.value.id);
      await loadAgentModels(agent.value.id);
      await loadAgentConfigModels(agent.value.id);
      const models = agentModels.value[agent.value.id] || [];
      for (const m of models) {
        useProxy.value[m.id] = m.use_proxy || false;
      }
    }
  } catch (e) {
    console.error("loadAgent error", e);
  }
}

async function loadAgentConfigModels(agentIdStr: string) {
  try {
    const configModels = await readAgentConfigModels(agentIdStr);
    agentConfigModels.value[agentIdStr] = configModels;
  } catch (e) {
    agentConfigModels.value[agentIdStr] = [];
  }
}

async function loadAllModels() {
  try {
    allModels.value = await getModels();
  } catch { /* */ }
}

async function loadAgentModels(agentIdStr: string) {
  try {
    const models = await getAgentModels(agentIdStr);
    agentModels.value[agentIdStr] = models;
  } catch (e) {
    agentModels.value[agentIdStr] = [];
  }
}

async function addModelToAgent(agentIdStr: string, modelId: string) {
  if (!modelId) return;
  addingModel.value[agentIdStr] = true;
  try {
    await assignModelToAgent(agentIdStr, modelId, useProxy.value[agentIdStr] || false);
    await loadAgentModels(agentIdStr);
    await loadAgentModelMapDetail(agentIdStr);
    await loadAgentConfigModels(agentIdStr);
    await loadConfig(agentIdStr);
    selectedModelId.value[agentIdStr] = '';
  } catch {}
  addingModel.value[agentIdStr] = false;
}

const agentModelMapDetail = ref<AgentModelInfo[]>([]);
async function loadAgentModelMapDetail(_agentIdStr: string) {
  try { agentModelMapDetail.value = await getAgentModelMap(); } catch { agentModelMapDetail.value = []; }
}
function isDefaultForAgentDetail(modelId: string, agentId: string): boolean {
  return agentModelMapDetail.value.some((e: AgentModelInfo) => e.agent_id === agentId && e.model_id === modelId && e.is_default);
}
async function setAsDefaultDetail(modelId: string, agentIdStr: string) {
  await setAgentDefaultModel(agentIdStr, modelId);
  await loadAgentModelMapDetail(agentIdStr);
  await loadAgentModels(agentIdStr);
}

async function removeModelFromAgentUI(agentIdStr: string, modelId: string) {
  try {
    await removeModelFromAgent(agentIdStr, modelId);
    await loadAgentModels(agentIdStr);
    await loadAgentConfigModels(agentIdStr);
    await loadConfig(agentIdStr);
  } catch {}
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
    await installAgent(id);
    await loadAgent();
  } catch (err) { if (!installLogs.value[id]) installLogs.value[id] = []; installLogs.value[id]!.push(`[error] ${err}`); }
  if (unlisten) unlisten();
  installing.value = null;
}

async function doUninstall(id: string) {
  uninstalling.value = id;
  installLogs.value[id] = [];
  let unlisten: UnlistenFn | null = null;
  try {
    unlisten = await listen<{ status: string; message: string }>(
      `agent-uninstall:${id}`,
      (event) => { if (!installLogs.value[id]) installLogs.value[id] = []; installLogs.value[id]!.push(`[${event.payload.status}] ${event.payload.message}`); },
    );
  } catch { /* */ }
  try { await uninstallAgent(id); } catch (err) {
    if (!installLogs.value[id]) installLogs.value[id] = [];
    installLogs.value[id]!.push(`[error] ${err}`);
  }
  if (unlisten) unlisten();
  uninstalling.value = null;
  await loadAgent();
}

async function doTest(id: string) {
  testing.value = id;
  try {
    const result = await testAgent(id);
    await loadAgent();
    if (!installLogs.value[id]) installLogs.value[id] = [];
    installLogs.value[id]!.push(`[test] ${result.message}`);
  } catch (err) {
    if (!installLogs.value[id]) installLogs.value[id] = [];
    installLogs.value[id]!.push(`[test error] ${err}`);
  }
  testing.value = null;
}

async function toggleEnabled(id: string, enabled: boolean) {
  try {
    await setAgentEnabled(id, enabled);
    await loadAgent();
  } catch (err) { console.error(err); }
}

async function loadConfig(id: string) {
  try { configContent.value[id] = await readAgentConfig(id); configDirty.value[id] = false; }
  catch { configContent.value[id] = ''; }
}

async function saveConfig(id: string) {
  configSaving.value[id] = true;
  try { await writeAgentConfig(id, configContent.value[id] || ''); configDirty.value[id] = false; }
  catch (err) { console.error(err); }
  configSaving.value[id] = false;
}

async function toggleModelProxy(agentIdStr: string, modelId: string, currentProxy: boolean) {
  try {
    await assignModelToAgent(agentIdStr, modelId, !currentProxy);
    await loadAgentModels(agentIdStr);
    await loadConfig(agentIdStr);
  } catch {}
}

</script>

<template>
  <div class="flex flex-col h-full">
    <!-- Back navigation -->
    <div class="flex items-center px-6 pt-5 pb-4">
      <button
        @click="router.push('/settings/agents')"
        class="group flex items-center gap-1.5 px-3 py-1.5 rounded-xl text-[13px] font-medium text-gray-400 hover:text-gray-700 hover:bg-white/60 transition-all duration-150 cursor-pointer"
      >
        <ArrowLeft :size="15" class="group-hover:-translate-x-0.5 transition-transform duration-150" />
        Back to Agents
      </button>
    </div>

    <!-- Loading -->
    <div v-if="!agent" class="flex flex-col items-center justify-center py-20 text-gray-400">
      <Loader2 :size="40" class="mb-3 animate-spin opacity-40" />
      <p class="text-sm">Loading agent...</p>
    </div>

    <!-- Main content -->
    <div v-else class="flex-1 overflow-y-auto px-6 pb-8 space-y-5">

      <!-- ========== HERO CARD ========== -->
      <div class="bg-white rounded-2xl border border-gray-100 shadow-sm overflow-hidden">
        <!-- Top gradient bar -->
        <div class="h-1 bg-gradient-to-r from-indigo-400 via-indigo-500 to-violet-500" />

        <div class="p-6">
          <!-- Identity row: icon + name + status -->
          <div class="flex items-start gap-5 mb-5">
            <div class="w-16 h-16 rounded-2xl bg-gradient-to-br from-gray-50 to-gray-100 border border-gray-200 flex items-center justify-center flex-shrink-0 shadow-sm">
              <AgentIcon :agent-id="agent.id" :size="44" />
            </div>
            <div class="flex-1 min-w-0 pt-1">
              <div class="flex items-center gap-3 mb-2 flex-wrap">
                <h1 class="text-[20px] font-semibold text-gray-900 tracking-tight">{{ agent.display_name }}</h1>
                <span
                  :class="[
                    'inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full text-[12px] font-medium',
                    agent.status === 'available'
                      ? 'bg-emerald-50 text-emerald-700 border border-emerald-200'
                      : agent.status === 'connection_failed'
                      ? 'bg-red-50 text-red-700 border border-red-200'
                      : 'bg-gray-100 text-gray-500 border border-gray-200',
                  ]"
                >
                  <span :class="['w-1.5 h-1.5 rounded-full', agent.status === 'available' ? 'bg-emerald-500' : agent.status === 'connection_failed' ? 'bg-red-500' : 'bg-gray-400']" />
                  {{ getStatusConfig(agent.status).label }}
                </span>
              </div>
              <div class="flex items-center gap-2 flex-wrap">
                <span v-if="agent.version" class="inline-flex items-center px-2 py-0.5 rounded-md bg-gray-100 text-[12px] font-mono text-gray-500">v{{ agent.version }}</span>
                <span v-if="agent.install_path" class="text-[12px] font-mono text-gray-400 truncate">{{ agent.install_path }}</span>
              </div>
            </div>
          </div>

          <!-- Description -->
          <p class="text-[13px] text-gray-500 leading-relaxed mb-5">{{ agentMeta[agent.id]?.description }}</p>

          <!-- Links row -->
          <div class="flex items-center gap-3 mb-5 flex-wrap">
            <a
              :href="agentMeta[agent.id]?.website"
              target="_blank"
              class="inline-flex items-center gap-1.5 px-3.5 py-2 rounded-xl text-[13px] font-medium bg-gray-50 border border-gray-200 text-gray-600 hover:bg-white hover:border-gray-300 hover:shadow-sm transition-all duration-150 cursor-pointer"
            >
              <ExternalLink :size="14" />
              Official Website
            </a>
            <div class="inline-flex items-center gap-2 bg-gray-50 rounded-xl border border-gray-200 px-3.5 py-2">
              <Terminal :size="14" class="text-gray-400 flex-shrink-0" />
              <code class="text-[13px] text-gray-600 font-mono select-all">{{ agentMeta[agent.id]?.installManual }}</code>
            </div>
          </div>

          <!-- Action buttons -->
          <div class="flex items-center gap-2.5 flex-wrap">
            <button
              @click="toggleEnabled(agent.id, !agent.enabled)"
              :class="[
                'flex items-center gap-1.5 px-4 py-2 rounded-xl text-[13px] font-medium transition-all duration-150 border cursor-pointer active:scale-[0.98]',
                agent.enabled
                  ? 'bg-emerald-50 border-emerald-200 text-emerald-700 hover:bg-emerald-100'
                  : 'bg-gray-50 border-gray-200 text-gray-500 hover:bg-gray-100',
              ]"
            >
              <ToggleRight v-if="agent.enabled" :size="16" />
              <ToggleLeft v-else :size="16" />
              {{ agent.enabled ? 'Enabled' : 'Disabled' }}
            </button>
            <button
              v-if="agent.installed"
              @click="doTest(agent.id)"
              :disabled="testing === agent.id"
              :class="[
                'flex items-center gap-1.5 px-4 py-2 rounded-xl text-[13px] font-medium transition-all duration-150 border cursor-pointer active:scale-[0.98]',
                testing === agent.id
                  ? 'bg-gray-50 border-gray-200 text-gray-400 cursor-not-allowed'
                  : 'bg-blue-50 border-blue-200 text-blue-700 hover:bg-blue-100',
              ]"
            >
              <Loader2 v-if="testing === agent.id" :size="16" class="animate-spin" />
              <Terminal v-else :size="16" />
              Test
            </button>
            <button
              v-if="!agent.installed"
              @click="doInstall(agent.id)"
              :disabled="installing === agent.id"
              class="flex items-center gap-1.5 px-5 py-2 rounded-xl text-[13px] font-semibold bg-indigo-600 text-white hover:bg-indigo-700 active:scale-[0.98] disabled:opacity-50 disabled:cursor-not-allowed transition-all duration-150 cursor-pointer shadow-sm"
            >
              <Loader2 v-if="installing === agent.id" :size="16" class="animate-spin" />
              <Download v-else :size="16" />
              Install Agent
            </button>
            <button
              v-if="agent.installed"
              @click="doUninstall(agent.id)"
              :disabled="uninstalling === agent.id"
              class="flex items-center gap-1.5 px-4 py-2 rounded-xl text-[13px] font-medium bg-red-50 border border-red-200 text-red-700 hover:bg-red-100 active:scale-[0.98] disabled:opacity-50 disabled:cursor-not-allowed transition-all duration-150 cursor-pointer"
            >
              <Loader2 v-if="uninstalling === agent.id" :size="16" class="animate-spin" />
              <Trash2 v-else :size="16" />
              Uninstall
            </button>
          </div>
        </div>
      </div>

      <!-- ========== MODEL CONFIGURATION ========== -->
      <div class="bg-white rounded-2xl border border-gray-100 shadow-sm overflow-hidden">
        <div class="px-5 py-4 border-b border-gray-100">
          <div class="flex items-center gap-2.5">
            <div class="w-7 h-7 rounded-lg bg-indigo-50 flex items-center justify-center">
              <Database :size="14" class="text-indigo-500" />
            </div>
            <h3 class="text-[14px] font-semibold text-gray-800 tracking-tight">Model Configuration</h3>
          </div>
        </div>

        <div class="p-5">
          <!-- No model assigned: picker -->
          <div v-if="(agentModels[agent.id] || []).length === 0">
            <div class="flex flex-wrap gap-2 mb-4">
              <button
                v-for="model in allModels"
                :key="model.id"
                @click="selectedModelId[agent.id] = model.id"
                :class="[
                  'flex items-center gap-2.5 px-4 py-2.5 rounded-xl text-[13px] font-medium border transition-all duration-150 cursor-pointer',
                  selectedModelId[agent.id] === model.id
                    ? `${getProviderColorClass(getProviderById(model.provider)?.id || getProviderByName(model.provider)?.id || 'custom')} shadow-sm`
                    : 'bg-gray-50 border-gray-200 text-gray-600 hover:border-gray-300 hover:bg-white',
                ]"
              >
                <div
                  :class="[
                    'w-6 h-6 rounded-lg flex items-center justify-center overflow-hidden flex-shrink-0',
                    selectedModelId[agent.id] === model.id ? 'bg-white/30' : 'bg-gray-200',
                  ]"
                >
                  <img
                    :src="getProviderLogo(getProviderById(model.provider)?.id || getProviderByName(model.provider)?.id || 'custom')"
                    :alt="model.provider"
                    class="w-4 h-4 object-contain"
                  />
                </div>
                {{ model.alias || model.name }}
              </button>
            </div>

            <div class="flex items-center gap-4 pt-1">
              <label class="flex items-center gap-2 cursor-pointer group">
                <input
                  type="checkbox"
                  v-model="useProxy[agent.id]"
                  @change="toggleModelProxy(agent.id, (agentModels[agent.id] || [])[0]?.id || '', !useProxy[agent.id])"
                  class="w-4 h-4 rounded border-gray-300 text-gray-700 focus:ring-gray-900/20 cursor-pointer"
                />
                <span class="text-[13px] font-medium text-gray-600 group-hover:text-gray-900 transition-colors">Protocol Translation</span>
                <span class="relative" @mouseenter="tooltipVisible = true" @mouseleave="tooltipVisible = false">
                  <HelpCircle :size="13" class="text-gray-400 hover:text-gray-600 transition-colors cursor-help" />
                  <div v-show="tooltipVisible" class="absolute left-1/2 -translate-x-1/2 top-5 z-50 w-64 px-3 py-2 rounded-xl bg-gray-900 text-[11px] text-white leading-relaxed shadow-xl">
                    Translate model protocol to agent protocol. Required when using different protocol models (e.g., use OpenAI model in Claude Code)
                  </div>
                </span>
              </label>
              <button
                @click="addModelToAgent(agent.id, selectedModelId[agent.id])"
                :disabled="addingModel[agent.id] || !selectedModelId[agent.id]"
                class="flex items-center gap-1.5 px-5 py-2 rounded-xl text-[13px] font-semibold bg-gray-800 text-white hover:bg-gray-900 active:scale-[0.98] disabled:opacity-30 disabled:cursor-not-allowed transition-all duration-150 ml-auto cursor-pointer shadow-sm"
              >
                <Plus :size="14" />
                Apply
              </button>
            </div>
          </div>

          <!-- Model assigned: card -->
          <div v-else class="space-y-2">
            <div
              v-for="model in agentModels[agent.id]"
              :key="model.id"
              class="flex items-center justify-between bg-gray-50 rounded-xl border border-gray-100 px-4 py-3 group hover:bg-white hover:border-gray-200 hover:shadow-sm transition-all duration-150"
            >
              <div class="flex items-center gap-3">
                <div class="w-9 h-9 rounded-xl flex items-center justify-center overflow-hidden bg-white border border-gray-200 shadow-sm">
                  <img
                    :src="getProviderLogo(getProviderById(model.provider)?.id || getProviderByName(model.provider)?.id || 'custom')"
                    :alt="model.provider"
                    class="w-5 h-5 object-contain"
                  />
                </div>
                <div>
                  <span class="text-[14px] font-medium text-gray-900">{{ model.alias || model.name }}</span>
                  <p class="text-[12px] text-gray-400">{{ model.name }}</p>
                </div>
              </div>
              <div class="flex items-center gap-3">
                <label class="flex items-center gap-2 cursor-pointer group/label">
                  <input
                    type="checkbox"
                    :checked="useProxy[model.id] || false"
                    @change="useProxy[model.id] = !useProxy[model.id]; toggleModelProxy(agent.id, model.id, !useProxy[model.id])"
                    class="w-4 h-4 rounded border-gray-300 text-gray-700 focus:ring-gray-900/20 cursor-pointer"
                  />
                  <span class="text-[12px] font-medium text-gray-500 group-hover/label:text-gray-700 transition-colors">Protocol Translation</span>
                </label>
                <button
                  v-if="!isDefaultForAgentDetail(model.id, agent.id)"
                  @click="setAsDefaultDetail(model.id, agent.id)"
                  class="ml-1 px-2 py-1 rounded-lg text-[11px] text-yellow-600 bg-yellow-50 hover:bg-yellow-100 border border-yellow-200 transition-colors cursor-pointer"
                >★ Default</button>
                <button
                  @click="removeModelFromAgentUI(agent.id, model.id)"
                  class="p-1.5 rounded-lg text-gray-300 hover:text-red-500 hover:bg-red-50 transition-colors cursor-pointer"
                >
                  <Trash2 :size="16" />
                </button>
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- ========== OPERATION LOG ========== -->
      <div v-if="installLogs[agent.id]?.length" class="bg-white rounded-2xl border border-gray-100 shadow-sm overflow-hidden">
        <div class="px-5 py-3 border-b border-gray-100">
          <h3 class="text-[14px] font-semibold text-gray-800 tracking-tight">Operation Log</h3>
        </div>
        <pre class="bg-gray-50 p-4 text-[12px] text-gray-600 font-mono leading-relaxed max-h-48 overflow-y-auto">{{ installLogs[agent.id]!.join('\n') }}</pre>
      </div>

      <!-- ========== CONFIGURATION FILE ========== -->
      <div class="bg-white rounded-2xl border border-gray-100 shadow-sm overflow-hidden">
        <div class="px-5 py-4 border-b border-gray-100">
          <div class="flex items-center justify-between">
            <div class="flex items-center gap-3">
              <h3 class="text-[14px] font-semibold text-gray-800 tracking-tight">Configuration File</h3>
              <span class="text-[12px] text-gray-400 font-mono bg-gray-100 px-2.5 py-0.5 rounded-lg">{{ agentMeta[agent.id]?.configPath }}</span>
            </div>
            <div class="flex items-center gap-3">
              <span class="text-[12px] text-gray-400 hidden sm:inline">Edit the raw JSON config directly</span>
              <button
                v-if="configDirty[agent.id]"
                @click="saveConfig(agent.id)"
                :disabled="configSaving[agent.id]"
                class="flex items-center gap-1.5 px-4 py-1.5 rounded-xl text-[12px] font-semibold text-white bg-indigo-600 hover:bg-indigo-700 active:scale-[0.98] disabled:opacity-50 transition-all duration-150 cursor-pointer shadow-sm"
              >
                <Save :size="13" />
                {{ configSaving[agent.id] ? 'Saving...' : 'Save Changes' }}
              </button>
            </div>
          </div>
        </div>
        <div class="p-4">
          <textarea
            :value="configContent[agent.id]"
            @input="(e) => { configContent[agent!.id] = (e.target as HTMLTextAreaElement).value; configDirty[agent!.id] = true; }"
            rows="20"
            spellcheck="false"
            class="w-full p-4 rounded-xl border border-gray-200 bg-gray-50 font-mono text-[12px] text-gray-700 leading-relaxed resize-none outline-none focus:ring-2 focus:ring-indigo-500/10 focus:border-indigo-400 transition-all placeholder:text-gray-400"
            placeholder="Config file content — will be created on save if it doesn't exist"
          />
        </div>
      </div>

    </div>
  </div>
</template>
