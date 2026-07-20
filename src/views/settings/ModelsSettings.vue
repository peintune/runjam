<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import { useRoute } from "vue-router";
import { Plus, Trash2, Eye, EyeOff, HelpCircle, Users, Download, Check, ExternalLink, RefreshCw } from "lucide-vue-next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { listen } from "@tauri-apps/api/event";
import { getModels, saveModels, providers, getProviderById, getProviderByName, maskApiKey, getAgentModelMap, assignModelToAgent, removeModelFromAgent, setAgentDefaultModel, checkOllamaInstalled, getOllamaStatus, listOllamaModels, pullOllamaModel, recommendedLocalModels, type AgentModelInfo, type ProtocolType, type OllamaModel, type OllamaPullProgress } from "../../api/models";
import { getProviderLogo } from "../../utils/providerIcons";
import { getAgentStatuses } from "../../api/agents";
import type { AgentInfo } from "../../api/agents";
import AgentIcon from "../../components/AgentIcon.vue";
import ConfirmDialog from "../../components/ConfirmDialog.vue";

interface UIModel { 
  id: string; 
  name: string; 
  alias: string;
  provider: string; 
  apiBase: string; 
  apiKey: string;
  protocol: string;
  showKey: boolean;
  assignedAgents: string[];
  useProxy: Record<string, boolean>;
}

const models = ref<UIModel[]>([]);
const showAdd = ref(false);
const agents = ref<AgentInfo[]>([]);
const agentModelMap = ref<Record<string, AgentModelInfo[]>>({});
const route = useRoute();

const newModel = ref({ 
  provider: "openai", 
  name: "", 
  alias: "",
  apiBase: providers[0].defaultBase, 
  apiKey: "" 
});

const selectedProvider = ref(providers[0]);
const showDeleteDialog = ref(false);
const deletingModelId = ref<string | null>(null);

const ollamaInstalled = ref(false);
const ollamaStatus = ref("");
const ollamaModels = ref<OllamaModel[]>([]);
const pullingModel = ref<string | null>(null);
const pullProgress = ref<OllamaPullProgress | null>(null);

onMounted(async () => {
  if (route.query.action === "add") {
    showAdd.value = true;
  }
  try { 
    const list = await getModels(); 
    models.value = list.map(m => ({ 
      id: m.id, 
      name: m.name, 
      alias: m.alias || m.name,
      provider: m.provider, 
      apiBase: m.api_base, 
      apiKey: m.api_key,
      protocol: m.protocol || "unknown",
      showKey: false,
      assignedAgents: [],
      useProxy: {},
    })); 
    await loadAgentModelMap();
  } catch {}
  try {
    agents.value = await getAgentStatuses();
  } catch {}
  
  await loadOllamaInfo();
  
  listen<OllamaPullProgress>("ollama_pull_progress", (event) => {
    pullProgress.value = event.payload;
    if (event.payload.status === "completed" || event.payload.status === "failed") {
      pullingModel.value = null;
      loadOllamaModels();
    }
  });

  listen<string>("ollama_pull_error", (event) => {
    pullError.value = event.payload;
    console.error("Ollama pull error:", event.payload);
  });
});

onUnmounted(() => {
});

async function loadOllamaInfo() {
  try {
    ollamaInstalled.value = await checkOllamaInstalled();
    ollamaStatus.value = await getOllamaStatus();
    await loadOllamaModels();
  } catch (err) {
    console.error("Failed to load Ollama info:", err);
  }
}

async function loadOllamaModels() {
  try {
    ollamaModels.value = await listOllamaModels();
  } catch {
    ollamaModels.value = [];
  }
}

function isModelInstalled(modelName: string): boolean {
  return ollamaModels.value.some(m => m.name === modelName);
}

const pullError = ref("");

async function downloadModel(modelName: string) {
  if (pullingModel.value) return;
  pullingModel.value = modelName;
  pullProgress.value = { status: "downloading", percentage: 0 };
  pullError.value = "";
  try {
    await pullOllamaModel(modelName);
  } catch (err: any) {
    pullError.value = err.message || err;
    console.error("Failed to pull model:", err);
    pullingModel.value = null;
  }
}

async function addLocalModel(modelName: string) {
  const provider = getProviderById("ollama")!;
  const modelExists = models.value.some(m => m.name === modelName && m.provider === "ollama");
  if (modelExists) return;
  
  models.value.push({
    id: `ollama-${modelName}-${Date.now().toString(36)}`,
    name: modelName,
    alias: modelName,
    provider: "ollama",
    apiBase: provider.defaultBase,
    apiKey: "ollama",
    protocol: provider.protocol,
    showKey: false,
    assignedAgents: [],
    useProxy: {},
  });
  await persistModels();
}

async function loadAgentModelMap() {
  try {
    const entries = await getAgentModelMap();
    const map: Record<string, AgentModelInfo[]> = {};
    for (const info of entries) {
      if (!map[info.model_id]) map[info.model_id] = [];
      map[info.model_id].push(info);
      const model = models.value.find(m => m.id === info.model_id);
      if (model) {
        model.useProxy[info.agent_id] = info.use_proxy;
      }
    }
    agentModelMap.value = map;
    for (const model of models.value) {
      model.assignedAgents = (map[model.id] || []).map(e => e.agent_id);
    }
  } catch {}
}

async function persistModels() {
  const list = models.value.map(m => {
    const provider = getProviderById(m.provider) || getProviderById("custom")!;
    return {
      id: m.id, 
      name: m.name, 
      alias: m.alias || m.name,
      provider: provider.id, 
      provider_name: provider.name, 
      provider_icon: provider.icon,
      api_base: m.apiBase, 
      api_key: m.apiKey, 
      protocol: m.protocol as ProtocolType,
      context_window: 0,
      support_reasoning: false,
      tags: [],
    };
  });
  try { await saveModels(list); } catch (err) { console.error("saveModels failed:", err); }
}

function onProviderChange(id: string) {
  const p = providers.find(p => p.id === id);
  if (p) { 
    newModel.value.provider = p.id;
    selectedProvider.value = p; 
    newModel.value.apiBase = p.defaultBase; 
    newModel.value.name = "";
    newModel.value.alias = "";
  }
}

function openProviderHomepage() {
  if (selectedProvider.value.homepage) {
    openUrl(selectedProvider.value.homepage);
  }
}

const nameError = ref("");
const apiKeyError = ref("");

async function addModel() {
  nameError.value = "";
  apiKeyError.value = "";
  if (!newModel.value.name) {
    nameError.value = "Model name is required";
    return;
  }
  if (!newModel.value.apiKey) {
    apiKeyError.value = "API key is required";
    return;
  }
  const provider = getProviderById(newModel.value.provider) || providers[0];
  models.value.push({ 
    id: `${newModel.value.provider}-${Date.now().toString(36)}`, 
    name: newModel.value.name, 
    alias: newModel.value.alias || newModel.value.name,
    provider: provider.id, 
    apiBase: newModel.value.apiBase || provider.defaultBase, 
    apiKey: newModel.value.apiKey,
    protocol: provider.protocol,
    showKey: false,
    assignedAgents: [],
    useProxy: {},
  });
  await persistModels();
  showAdd.value = false;
  newModel.value = { provider:"openai", name:"", alias:"", apiBase: providers[0].defaultBase, apiKey:"" };
  nameError.value = "";
  apiKeyError.value = "";
}

function removeModel(id: string) {
  deletingModelId.value = id;
  showDeleteDialog.value = true;
}

async function confirmDelete() {
  if (deletingModelId.value) {
    models.value = models.value.filter(m => m.id !== deletingModelId.value);
    await persistModels();
  }
  showDeleteDialog.value = false;
  deletingModelId.value = null;
}

function cancelDelete() {
  showDeleteDialog.value = false;
  deletingModelId.value = null;
}

function toggleShowKey(model: UIModel) {
  model.showKey = !model.showKey;
}

async function toggleAgentAssignment(modelId: string, agentId: string) {
  const model = models.value.find(m => m.id === modelId);
  if (!model) return;
  const idx = model.assignedAgents.indexOf(agentId);
  if (idx >= 0) {
    model.assignedAgents.splice(idx, 1);
    await removeModelFromAgent(agentId, modelId);
  } else {
    model.assignedAgents.push(agentId);
    model.useProxy[agentId] = model.useProxy[agentId] ?? true;
    await assignModelToAgent(agentId, modelId, model.useProxy[agentId]);
  }
}

async function toggleProtocolTranslation(modelId: string, agentId: string) {
  const model = models.value.find(m => m.id === modelId);
  if (!model) return;
  model.useProxy[agentId] = !(model.useProxy[agentId] || false);
  await assignModelToAgent(agentId, modelId, model.useProxy[agentId]);
}

async function setAsDefault(modelId: string, agentId: string) {
  await setAgentDefaultModel(agentId, modelId);
  await loadAgentModelMap();
}

function isDefaultForAgent(modelId: string, agentId: string): boolean {
  const entries = agentModelMap.value[modelId] || [];
  return entries.some(e => e.agent_id === agentId && e.is_default);
}

function getAgentDisplayName(agentId: string): string {
  const agent = agents.value.find(a => a.id === agentId);
  return agent?.display_name || agentId;
}

function protocolDiffers(model: UIModel, agentId: string): boolean {
  const nativeProtocol = getAgentNativeProtocol(agentId);
  return nativeProtocol !== "" && model.protocol !== nativeProtocol;
}

function getAgentNativeProtocol(agentId: string): string {
  const agentProtocol: Record<string, string> = {
    "claude-code": "anthropic",
    "codex-cli": "openai_chat",
    "gemini-cli": "gemini",
  };
  return agentProtocol[agentId] || "";
}

function getStatusText(): string {
  if (ollamaStatus.value.startsWith("installed")) {
    return "Ollama running";
  }
  return "Ollama not installed";
}

function getStatusColor(): string {
  if (ollamaStatus.value.startsWith("installed")) {
    return "bg-emerald-500";
  }
  return "bg-gray-300";
}
</script>

<template>
  <div class="max-w-3xl mx-auto p-8">
    <div class="flex items-center justify-between mb-8">
      <div>
        <h2 class="text-[18px] font-semibold text-gray-900 tracking-tight">Models</h2>
        <p class="text-[13px] text-gray-500 mt-0.5">Configure LLM providers shared across all agents</p>
      </div>
      <button @click="showAdd = true; nameError = ''; apiKeyError = ''"
        class="flex items-center gap-1.5 px-4 py-2 rounded-xl text-[13px] font-medium bg-gray-700 text-white hover:bg-gray-600 active:scale-[0.98] transition-all duration-150 shadow-sm cursor-pointer">
        <Plus :size="15" /> Add Model
      </button>
    </div>

    <div class="mb-8 bg-white rounded-xl border border-gray-100 shadow-sm overflow-hidden">
      <div class="flex items-center justify-between px-5 py-3 bg-gray-700 border-b border-gray-600">
        <div class="flex items-center gap-3">
          <div class="w-8 h-8 rounded-lg flex items-center justify-center overflow-hidden bg-gray-200">
            <img :src="getProviderLogo('ollama')" alt="Ollama" class="w-5 h-5 object-contain" />
          </div>
          <span class="text-[15px] font-semibold text-white">Local Models (Ollama)</span>
        </div>
        <div class="flex items-center gap-2">
          <span :class="['w-2 h-2 rounded-full', getStatusColor()]"></span>
          <span class="text-[11px] text-gray-300">{{ getStatusText() }}</span>
        </div>
      </div>

      <div v-if="!ollamaInstalled" class="px-5 py-6">
        <div class="flex items-center gap-3 bg-amber-50 border border-amber-200 rounded-xl px-4 py-3">
          <div class="text-amber-500">💡</div>
          <div class="flex-1">
            <p class="text-[13px] font-medium text-amber-800">Ollama is not installed</p>
            <p class="text-[12px] text-amber-600 mt-0.5">Install Ollama to run models locally without cloud API keys</p>
          </div>
          <button @click="openUrl('https://ollama.com/download')" class="flex items-center gap-1 px-3 py-1.5 rounded-lg bg-amber-500 text-white text-[12px] font-medium hover:bg-amber-600 transition-colors cursor-pointer">
            Download <ExternalLink :size="12" />
          </button>
        </div>
      </div>

      <div v-else class="px-5 py-4">
        <div class="flex items-center justify-between mb-4">
          <div class="flex items-center gap-2">
            <span class="text-[14px] font-medium text-gray-600">Recommended Models</span>
          </div>
          <button @click="loadOllamaModels" class="flex items-center gap-1.5 text-[12px] text-gray-500 hover:text-gray-700 transition-colors cursor-pointer">
            <RefreshCw :size="14" /> Refresh
          </button>
        </div>

        <div class="grid grid-cols-2 gap-4">
          <div
            v-for="rm in recommendedLocalModels"
            :key="rm.name"
            :class="[
              'relative rounded-xl border transition-all duration-150',
              isModelInstalled(rm.name) ? 'border-gray-200 bg-gray-50' : 'border-gray-100 bg-white hover:border-gray-200'
            ]"
          >
            <div class="p-5">
              <div class="flex items-start justify-between mb-3">
                <div>
                  <div class="flex items-center gap-2">
                    <span class="text-[17px] font-semibold text-gray-900">{{ rm.alias }}</span>
                    <span v-if="isModelInstalled(rm.name)" class="flex items-center gap-0.5 text-[11px] text-emerald-600 bg-emerald-50 px-2 py-0.5 rounded-full">
                      <Check :size="11" /> Installed
                    </span>
                  </div>
                  <p class="text-[12px] text-gray-400 mt-1">{{ rm.name }}</p>
                </div>
                <span class="text-[12px] font-medium text-gray-500 bg-gray-100 px-2.5 py-1 rounded-lg">{{ rm.size }}</span>
              </div>
              <p class="text-[13px] text-gray-500 mb-4">{{ rm.desc }}</p>

              <div class="flex items-center gap-2">
                <template v-if="pullingModel === rm.name">
                  <div class="flex-1">
                    <div class="h-1.5 bg-gray-200 rounded-full overflow-hidden">
                      <div 
                        class="h-full bg-gray-700 transition-all duration-200"
                        :style="{ width: `${pullProgress?.percentage || 0}%` }"
                      ></div>
                    </div>
                    <p class="text-[10px] text-gray-400 mt-1">
                      {{ pullProgress?.status === 'downloading' ? `${Math.round(pullProgress.percentage)}%` : pullProgress?.status || 'Downloading...' }}
                    </p>
                    <p v-if="pullError" class="text-[10px] text-red-500 mt-1.5 truncate">{{ pullError }}</p>
                  </div>
                </template>
                <template v-else-if="isModelInstalled(rm.name)">
                  <button 
                    @click="addLocalModel(rm.name)"
                    class="flex-1 flex items-center justify-center gap-1 px-3 py-1.5 rounded-lg text-[12px] font-medium bg-gray-700 text-white hover:bg-gray-600 transition-colors cursor-pointer"
                  >
                    <Plus :size="12" /> Add to Models
                  </button>
                </template>
                <template v-else>
                  <button 
                    @click="downloadModel(rm.name)"
                    class="flex-1 flex items-center justify-center gap-1 px-3 py-1.5 rounded-lg text-[12px] font-medium bg-gray-700 text-white hover:bg-gray-600 transition-colors cursor-pointer"
                  >
                    <Download :size="12" /> Download
                  </button>
                </template>
              </div>
            </div>
          </div>
        </div>

        <div v-if="ollamaModels.length > 0" class="mt-6 pt-5 border-t border-gray-100">
          <div class="flex items-center gap-2 mb-4">
            <span class="text-[14px] font-medium text-gray-600">Installed Models</span>
            <span class="text-[12px] text-gray-400">({{ ollamaModels.length }})</span>
          </div>
          <div class="space-y-2.5">
            <div 
              v-for="om in ollamaModels" 
              :key="om.name"
              class="flex items-center justify-between px-4 py-3 rounded-xl bg-gray-50"
            >
              <div>
                <span class="text-[15px] font-medium text-gray-900">{{ om.name }}</span>
                <span class="text-[12px] text-gray-400 ml-3">{{ om.size }}</span>
              </div>
              <button 
                v-if="!models.some(m => m.name === om.name && m.provider === 'ollama')"
                @click="addLocalModel(om.name)"
                class="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-medium bg-gray-700 text-white hover:bg-gray-600 transition-colors cursor-pointer"
              >
                <Plus :size="12" /> Add
              </button>
              <span v-else class="flex items-center gap-1.5 text-[12px] text-emerald-600">
                <Check :size="14" /> Added
              </span>
            </div>
          </div>
        </div>
      </div>
    </div>

    <div v-if="showAdd" class="fixed inset-0 z-50 flex items-center justify-center p-4">
      <div class="absolute inset-0 bg-black/40 backdrop-blur-sm transition-opacity" @click="showAdd = false"></div>
      <div class="relative w-full max-w-lg bg-white rounded-2xl shadow-xl border border-gray-100 overflow-hidden animate-in fade-in zoom-in duration-200">
        <div class="px-6 py-4 border-b border-gray-100 bg-gray-50">
          <h3 class="text-[16px] font-semibold text-gray-900">Add New Model</h3>
        </div>
        <div class="p-6 space-y-5 max-h-[70vh] overflow-y-auto">
          <div>
            <label class="block text-[12px] font-medium text-gray-500 mb-2">Provider</label>
            <div class="grid grid-cols-3 gap-2">
              <button v-for="p in providers" :key="p.id"
                @click="onProviderChange(p.id)"
                :class="['p-3 rounded-xl border-2 text-left transition-all duration-150 cursor-pointer',
                  newModel.provider === p.id ? 'border-gray-300 bg-gray-100/50' : 'border-gray-100 hover:border-gray-200 hover:bg-gray-50']">
                <div class="flex items-center gap-2">
                  <img :src="getProviderLogo(p.id)" :alt="p.name" class="w-5 h-5 object-contain" />
                  <span class="text-[13px] font-semibold text-gray-900">{{ p.name }}</span>
                </div>
                <div class="text-[11px] text-gray-400 mt-0.5">{{ p.desc }}</div>
              </button>
            </div>
          </div>

          <div>
            <label class="block text-[12px] font-medium text-gray-500 mb-2">Alias</label>
            <input v-model="newModel.alias" type="text" placeholder="Short name (optional)"
              class="w-full px-3 py-2 rounded-xl border border-gray-200 bg-white text-[13px] text-gray-900 placeholder-gray-400 outline-none focus:ring-2 focus:ring-gray-600/20 focus:border-gray-400 transition-all" />
          </div>

          <div>
            <label class="flex items-center gap-1 text-[12px] font-medium text-gray-500 mb-2">
              Model Name
              <button v-if="selectedProvider.homepage" @click="openProviderHomepage"
                class="text-gray-300 hover:text-gray-500 transition-colors cursor-pointer" title="Open provider homepage for model names">
                <HelpCircle :size="13" />
              </button>
            </label>
            <input v-model="newModel.name" type="text" placeholder="e.g. gpt-4o"
              class="w-full px-3 py-2 rounded-xl border border-gray-200 bg-white text-[13px] text-gray-900 placeholder-gray-400 outline-none focus:ring-2 focus:ring-gray-600/20 focus:border-gray-400 transition-all" />
            <p v-if="nameError" class="text-[12px] text-red-500 mt-1.5">{{ nameError }}</p>
          </div>

          <div>
            <label class="block text-[12px] font-medium text-gray-500 mb-2">API Base URL</label>
            <input v-model="newModel.apiBase" type="text" :placeholder="selectedProvider.defaultBase"
              class="w-full px-3 py-2 rounded-xl border border-gray-200 bg-white text-[13px] text-gray-900 placeholder-gray-400 outline-none focus:ring-2 focus:ring-gray-600/20 focus:border-gray-400 transition-all" />
          </div>

          <div>
            <label class="flex items-center gap-1 text-[12px] font-medium text-gray-500 mb-2">
              API Key
              <button v-if="selectedProvider.homepage" @click="openProviderHomepage"
                class="text-gray-300 hover:text-gray-500 transition-colors cursor-pointer" title="Open provider homepage to get API key">
                <HelpCircle :size="13" />
              </button>
            </label>
            <input v-model="newModel.apiKey" type="password" placeholder="sk-..."
              class="w-full px-3 py-2 rounded-xl border border-gray-200 bg-white text-[13px] text-gray-900 placeholder-gray-400 outline-none focus:ring-2 focus:ring-gray-600/20 focus:border-gray-400 transition-all" />
            <p v-if="apiKeyError" class="text-[12px] text-red-500 mt-1.5">{{ apiKeyError }}</p>
          </div>
        </div>
        <div class="px-6 py-4 border-t border-gray-100 bg-gray-50 flex gap-2 justify-end">
          <button @click="showAdd = false" class="px-4 py-2 rounded-xl text-[13px] text-gray-500 hover:bg-gray-100 transition-colors duration-150 cursor-pointer">Cancel</button>
          <button @click="addModel" class="px-5 py-2 rounded-xl text-[13px] font-medium text-white bg-gray-700 hover:bg-gray-600 active:scale-[0.98] transition-all duration-150 shadow-sm cursor-pointer">Add Model</button>
        </div>
      </div>
    </div>

    <div class="space-y-2">
      <div v-for="model in models" :key="model.id"
        class="group bg-white rounded-xl border border-gray-100 shadow-sm hover:shadow-md transition-shadow duration-200 overflow-hidden">
        <div class="flex items-center justify-between px-5 py-2.5 bg-gray-700 border-b border-gray-600">
          <div class="flex items-center gap-2">
            <div class="w-6 h-6 rounded-lg flex items-center justify-center overflow-hidden bg-gray-200">
              <img :src="getProviderLogo(getProviderById(model.provider)?.id || getProviderByName(model.provider)?.id || 'custom')" :alt="model.provider" class="w-4 h-4 object-contain" />
            </div>
            <span class="text-[11px] font-medium text-gray-300">{{ getProviderById(model.provider)?.name || model.provider }}</span>
          </div>
          <span class="text-[14px] font-semibold text-white">{{ model.alias || model.name }}</span>
        </div>
        <div class="flex items-center justify-between px-5 py-4">
          <div class="min-w-0">
            <p class="text-[13px] font-medium text-gray-900 mb-2">
              {{ model.name }}
              <span :class="[
                'ml-1.5 text-[10px] font-medium rounded-full px-1.5 py-0.5 border',
                model.protocol === 'anthropic' ? 'bg-orange-50 text-orange-600 border-orange-200' :
                model.protocol === 'gemini' ? 'bg-blue-50 text-blue-600 border-blue-200' :
                model.protocol === 'openai_responses' ? 'bg-purple-50 text-purple-600 border-purple-200' :
                'bg-green-50 text-green-600 border-green-200'
              ]">
                {{ model.protocol === 'openai_chat' ? 'Chat' : model.protocol === 'openai_responses' ? 'Responses' : model.protocol }}
              </span>
            </p>
            <div class="flex items-center gap-2">
              <p class="text-[11px] text-gray-400 truncate flex-1">{{ model.apiBase }}</p>
              <div class="flex items-center gap-1">
                <span class="text-[11px] text-gray-400">{{ model.showKey ? model.apiKey : maskApiKey(model.apiKey) }}</span>
                <button @click="toggleShowKey(model)" class="p-0.5 text-gray-300 hover:text-gray-500 transition-colors cursor-pointer">
                  <Eye v-if="model.showKey" :size="12" />
                  <EyeOff v-else :size="12" />
                </button>
              </div>
            </div>
          </div>
          <button @click="removeModel(model.id)"
            class="p-2 rounded-lg text-gray-300 hover:text-red-500 hover:bg-red-50 opacity-0 group-hover:opacity-100 transition-all duration-150 flex-shrink-0 cursor-pointer">
            <Trash2 :size="15" />
          </button>
        </div>

        <div class="px-5 py-3 bg-gray-50 border-t border-gray-100">
          <div class="flex items-center justify-between mb-3">
            <div class="flex items-center gap-2">
              <Users :size="13" class="text-gray-400" />
              <span class="text-[11px] font-medium text-gray-500">Assigned to Agents</span>
            </div>
            <span class="text-[11px] text-gray-400">
              {{ model.assignedAgents.length }} / {{ agents.filter(a => a.installed).length }} agents
            </span>
          </div>

          <div class="space-y-1.5">
            <div
              v-for="agent in agents.filter(a => a.installed)"
              :key="agent.id"
              @click="toggleAgentAssignment(model.id, agent.id)"
              :class="[
                'flex items-center gap-3 px-3 py-2.5 rounded-xl border transition-all duration-150 cursor-pointer',
                model.assignedAgents.includes(agent.id)
                  ? 'bg-white border-gray-300 shadow-sm'
                  : 'bg-white/50 border-gray-100 hover:border-gray-200 hover:bg-white'
              ]"
            >
              <div class="flex items-center gap-2 flex-1 min-w-0">
                <AgentIcon :agent-id="agent.id" :size="22" />
                <div class="min-w-0">
                  <span class="text-[13px] font-medium text-gray-900">{{ getAgentDisplayName(agent.id) }}</span>
                  <span v-if="agent.status === 'available'" class="ml-1.5 inline-block w-1.5 h-1.5 rounded-full bg-emerald-500"></span>
                  <span v-else-if="agent.status === 'connection_failed'" class="ml-1.5 inline-block w-1.5 h-1.5 rounded-full bg-red-400"></span>
                </div>
              </div>

              <template v-if="model.assignedAgents.includes(agent.id)">
                <button
                  v-if="isDefaultForAgent(model.id, agent.id)"
                  @click.stop
                  class="flex items-center gap-1 px-2 py-1 rounded-lg text-[11px] font-medium bg-yellow-50 border border-yellow-200 text-yellow-700 cursor-default"
                >
                  <span class="text-[12px]">★</span> Default
                </button>
                <button
                  v-else
                  @click.stop="setAsDefault(model.id, agent.id)"
                  class="flex items-center gap-1 px-2 py-1 rounded-lg text-[11px] font-medium border border-gray-200 text-gray-500 hover:border-yellow-300 hover:text-yellow-600 hover:bg-yellow-50 transition-all duration-150 cursor-pointer"
                >
                  Set Default
                </button>

                <button
                  v-if="protocolDiffers(model, agent.id)"
                  @click.stop="toggleProtocolTranslation(model.id, agent.id)"
                  :title="`This model uses ${model.protocol} protocol, but ${getAgentDisplayName(agent.id)} expects ${getAgentNativeProtocol(agent.id)}. Enable translation to bridge the difference.`"
                  :class="[
                    'flex items-center gap-1 px-2 py-1 rounded-lg text-[11px] font-medium transition-all duration-150 cursor-pointer border',
                    model.useProxy[agent.id]
                      ? 'bg-indigo-50 border-indigo-200 text-indigo-600'
                      : 'bg-white border-gray-200 text-gray-400 hover:text-gray-600 hover:border-gray-300'
                  ]"
                >
                  <span class="text-[10px]">⟳</span>
                  Translate
                  <HelpCircle :size="10" class="text-gray-300" />
                </button>
              </template>

              <div
                :class="[
                  'flex items-center justify-center w-16 h-7 rounded-full text-[10px] font-semibold transition-all duration-200 flex-shrink-0 border',
                  model.assignedAgents.includes(agent.id)
                    ? 'bg-emerald-500 border-emerald-500 text-white'
                    : 'bg-gray-100 border-gray-200 text-gray-400'
                ]">
                {{ model.assignedAgents.includes(agent.id) ? 'ON' : 'OFF' }}
              </div>
            </div>

            <div v-if="agents.filter(a => a.installed).length === 0" class="text-center py-4">
              <p class="text-[12px] text-gray-400">No agents installed. Go to <router-link to="/settings/agents" class="text-indigo-500 hover:underline">Agents</router-link> to install one.</p>
            </div>
          </div>
        </div>
      </div>
    </div>

    <div v-if="models.length === 0 && !showAdd" class="text-center py-20">
      <div class="w-14 h-14 rounded-2xl bg-gray-100 flex items-center justify-center mx-auto mb-4 text-[24px]">
        ⚡
      </div>
      <h3 class="text-[15px] font-medium text-gray-700 mb-1">No models configured</h3>
      <p class="text-[13px] text-gray-400 mb-4">Add a model to start using it in conversations</p>
      <button @click="showAdd = true"
        class="px-4 py-2 rounded-xl text-[13px] font-medium bg-gray-700 text-white hover:bg-gray-600 active:scale-[0.98] transition-all duration-150 shadow-sm cursor-pointer">
        Add your first model
      </button>
    </div>

    <ConfirmDialog
      :show="showDeleteDialog"
      title="Delete Model"
      message="Deleting a model will not affect the configuration of already configured agents."
      @confirm="confirmDelete"
      @cancel="cancelDelete"
    />
  </div>
</template>