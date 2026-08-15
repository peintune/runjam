<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed } from "vue";
import { useRoute } from "vue-router";
import { Plus, Trash2, Eye, EyeOff, HelpCircle, Users, Download, Check, ExternalLink, Play, Square, FolderOpen, ChevronDown, X, RefreshCw } from "lucide-vue-next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { getModels, saveModels, providers, getProviderById, getProviderByName, maskApiKey, getAgentModelMap, assignModelToAgent, removeModelFromAgent, checkLlamaServerAvailable, getLlamaServerStatus, listLlamaModels, downloadLlamaModel, startLlamaServer, stopLlamaServer, openLlamaModelsDir, getDownloadStatus, recommendedLocalModels, type AgentModelInfo, type ProtocolType, type LlamaModel, type LlamaPullProgress } from "../../api/models";
import { getProviderLogo } from "../../utils/providerIcons";
import { getAgentStatuses } from "../../api/agents";
import type { AgentInfo } from "../../api/agents";
import AgentIcon from "../../components/AgentIcon.vue";
import ConfirmDialog from "../../components/ConfirmDialog.vue";
import { useAgentStore } from "../../stores/useAgentStore";

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

const agentStore = useAgentStore();
const models = ref<UIModel[]>([]);
const showAdd = ref(false);
const agents = ref<AgentInfo[]>([]);
const agentModelMap = ref<Record<string, AgentModelInfo[]>>({});
const route = useRoute();
const refreshing = ref(false);

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

const llamaServerAvailable = ref(false);
const llamaServerStatus = ref("");
const llamaModels = ref<LlamaModel[]>([]);
const pullingModel = ref<string | null>(null);
const pullProgress = ref<LlamaPullProgress | null>(null);
const runningServerPort = ref(0);
const runningServerModel = ref<string | null>(null);

const showAgentDropdown = ref<string | null>(null);
const agentDropdownPosition = ref({ x: 0, y: 0 });

const startingServer = ref(false);
const serverError = ref("");
const serverLogs = ref<string[]>([]);
const serverStartFailed = ref(false);
const serverFailureReason = ref("");

function getFilename(name: string): string {
  const parts = name.split('/');
  return parts[parts.length - 1];
}

function isModelRunning(filename: string): boolean {
  const modelFilename = getFilename(filename);
  const runningFilename = runningServerModel.value ? getFilename(runningServerModel.value) : '';
  const result = runningServerPort.value > 0 && runningFilename === modelFilename;
  console.log("[DEBUG] isModelRunning:", filename, "->", modelFilename, "runningServerPort:", runningServerPort.value, "runningServerModel:", runningServerModel.value, "->", runningFilename, "result:", result);
  return result;
}

const showAddLocalModel = ref(false);
const newLocalModelName = ref("");

async function refreshModels() {
  refreshing.value = true;
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
    await Promise.all([
      loadAgentModelMap(),
      loadLlamaInfo(),
    ]);
  } catch {} finally {
    refreshing.value = false;
  }
}

onMounted(() => {
  if (route.query.action === "add") {
    showAdd.value = true;
  }

  if (agentStore.models.length > 0) {
    models.value = agentStore.models.map(m => ({
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
    Promise.all([loadAgentModelMap(), loadLlamaInfo()]);
  } else {
    refreshModels();
  }

  if (agentStore.agents.length > 0) {
    agents.value = agentStore.agents;
  } else {
    getAgentStatuses().then(list => { agents.value = list; }).catch(() => {});
  }
  
  listen<LlamaPullProgress>("llama_pull_progress", (event) => {
    pullProgress.value = event.payload;
    if (event.payload.status === "completed") {
      if (pullingModel.value) {
        onModelDownloaded(pullingModel.value);
      }
      pullingModel.value = null;
      loadLlamaModels();
    } else if (event.payload.status === "failed") {
      pullingModel.value = null;
      loadLlamaModels();
    }
  });

  listen<string>("llama_pull_error", (event) => {
    pullError.value = event.payload;
    console.error("Llama pull error:", event.payload);
  });

  listen<string>("llama_server_log", (event) => {
    console.log("Llama server log:", event.payload);
    if (startingServer.value) {
      serverLogs.value.push(event.payload);
      if (serverLogs.value.length > 100) {
        serverLogs.value = serverLogs.value.slice(-50);
      }
    }
  });

  listen<string>("llama_server_failed", (event) => {
    console.error("Llama server failed:", event.payload);
    serverStartFailed.value = true;
    serverFailureReason.value = event.payload;
    startingServer.value = false;
  });

  listen<string[]>("llama_server_stderr", (event) => {
    console.error("Llama server stderr:", event.payload);
    if (startingServer.value) {
      serverLogs.value.push(...event.payload.map(l => `[ERROR] ${l}`));
    }
  });

  document.addEventListener("click", closeAgentDropdown);
});

onUnmounted(() => {
  document.removeEventListener("click", closeAgentDropdown);
});

function closeAgentDropdown(e: MouseEvent) {
  const target = e.target as HTMLElement;
  if (!target.closest(".agent-dropdown-btn") && !target.closest(".agent-dropdown-panel")) {
    showAgentDropdown.value = null;
  }
}

async function loadLlamaInfo() {
  try {
    // 并行发起所有独立的后端调用
    const [available, status, serverStatus, downloadStatus] = await Promise.all([
      checkLlamaServerAvailable(),
      getLlamaServerStatus(),
      (async () => {
        await loadLlamaModels();
        try {
          return await invoke<{ running: boolean; port: number; model: string | null }>("get_server_status");
        } catch (e) {
          console.log("[DEBUG] get_server_status failed:", e);
          return null;
        }
      })(),
      getDownloadStatus().catch(() => ({ downloading: null as string | null, progress: { status: "", percentage: 0 } as LlamaPullProgress })),
    ]);

    llamaServerAvailable.value = available;
    llamaServerStatus.value = status;

    if (serverStatus) {
      console.log("[DEBUG] get_server_status result:", serverStatus);
      if (serverStatus.running) {
        runningServerPort.value = serverStatus.port;
        runningServerModel.value = serverStatus.model;
        console.log("[DEBUG] Set runningServerModel to:", runningServerModel.value);

        // Auto-add running model to models list if not already there
        if (serverStatus.model) {
          const provider = getProviderById("llama");
          const modelFilename = getFilename(serverStatus.model);
          const existingModel = models.value.find(m => getFilename(m.name) === modelFilename && m.provider === "llama");
          if (existingModel) {
            console.log("[DEBUG] Model already exists, updating:", existingModel.name);
            existingModel.apiBase = `http://localhost:${serverStatus.port}/v1`;
            persistModels();
          } else if (provider) {
            console.log("[DEBUG] Auto-adding running model to models list:", serverStatus.model);
            models.value.push({
              id: `llama-${serverStatus.model}-auto`,
              name: serverStatus.model,
              alias: serverStatus.model,
              provider: "llama",
              apiBase: `http://localhost:${serverStatus.port}/v1`,
              apiKey: "llama",
              protocol: provider.protocol,
              showKey: false,
              assignedAgents: [],
              useProxy: {},
            });
            persistModels();
          }
        }
      } else {
        runningServerPort.value = 0;
        runningServerModel.value = null;
      }
    } else {
      // get_server_status 失败，使用 llamaServerStatus 回退
      if (status.startsWith("running")) {
        const portStr = status.split(":")[1];
        runningServerPort.value = parseInt(portStr) || 19090;
        runningServerModel.value = null;
      } else {
        runningServerPort.value = 0;
        runningServerModel.value = null;
      }
    }

    if (downloadStatus.downloading) {
      pullingModel.value = downloadStatus.downloading;
      pullProgress.value = downloadStatus.progress;
    }
  } catch (err) {
    console.error("Failed to load Llama info:", err);
  }
}

async function loadLlamaModels() {
  try {
    llamaModels.value = await listLlamaModels();
  } catch {
    llamaModels.value = [];
  }
}

function isModelInstalled(filename: string): boolean {
  return llamaModels.value.some(m => m.name === filename);
}

const pullError = ref("");

async function downloadModel(hfRepo: string, filename: string) {
  if (pullingModel.value) return;
  pullingModel.value = filename;
  pullProgress.value = { status: "downloading", percentage: 0 };
  pullError.value = "";
  try {
    await downloadLlamaModel(hfRepo, filename);
  } catch (err: any) {
    pullError.value = err.message || err;
    console.error("Failed to download model:", err);
    pullingModel.value = null;
  }
}

async function onModelDownloaded(filename: string) {
  const provider = getProviderById("llama")!;
  const modelExists = models.value.some(m => m.name === filename && m.provider === "llama");
  if (modelExists) return;
  
  const apiBase = runningServerPort.value > 0 
    ? `http://localhost:${runningServerPort.value}/v1`
    : provider.defaultBase;
  
  models.value.push({
    id: `llama-${filename}-${Date.now().toString(36)}`,
    name: filename,
    alias: filename,
    provider: "llama",
    apiBase,
    apiKey: "llama",
    protocol: provider.protocol,
    showKey: false,
    assignedAgents: [],
    useProxy: {},
  });
  await persistModels();
}

async function handleAddLocalModel() {
  if (!newLocalModelName.value.trim()) return;
  
  const provider = getProviderById("llama")!;
  const modelName = newLocalModelName.value.trim();
  const modelExists = models.value.some(m => m.name === modelName && m.provider === "llama");
  if (modelExists) {
    return;
  }
  
  const apiBase = runningServerPort.value > 0 
    ? `http://localhost:${runningServerPort.value}/v1`
    : provider.defaultBase;
  
  models.value.push({
    id: `llama-${modelName}-${Date.now().toString(36)}`,
    name: modelName,
    alias: modelName,
    provider: "llama",
    apiBase,
    apiKey: "llama",
    protocol: provider.protocol,
    showKey: false,
    assignedAgents: [],
    useProxy: {},
  });
  await persistModels();
  
  showAddLocalModel.value = false;
  newLocalModelName.value = "";
}

async function handleStartServer(filename: string) {
  if (startingServer.value) return;
  
  startingServer.value = true;
  serverError.value = "";
  serverLogs.value = [];
  serverStartFailed.value = false;
  serverFailureReason.value = "";
  
  try {
    const onServerStarted = (startedPort: number) => {
      console.log("[DEBUG] onServerStarted called with port:", startedPort);
      if (startedPort > 0) {
        runningServerPort.value = startedPort;
        loadLlamaInfo();
        
        const provider = getProviderById("llama")!;
        const modelFilename = getFilename(filename);
        const existingModel = models.value.find(m => getFilename(m.name) === modelFilename && m.provider === "llama");
        console.log("[DEBUG] existingModel:", existingModel ? existingModel.name : "not found");
        console.log("[DEBUG] models before add:", models.value.filter(m => m.provider === "llama").map(m => m.name));
        
        if (existingModel) {
          existingModel.apiBase = `http://localhost:${startedPort}/v1`;
          persistModels();
        } else {
          models.value.push({
            id: `llama-${filename}-${Date.now().toString(36)}`,
            name: filename,
            alias: filename,
            provider: "llama",
            apiBase: `http://localhost:${startedPort}/v1`,
            apiKey: "llama",
            protocol: provider.protocol,
            showKey: false,
            assignedAgents: [],
            useProxy: {},
          });
          console.log("[DEBUG] models after add:", models.value.filter(m => m.provider === "llama").map(m => m.name));
          persistModels();
        }
        serverStartFailed.value = false;
      } else {
        serverStartFailed.value = true;
        serverFailureReason.value = serverFailureReason.value || "Failed to start server: Server did not respond within timeout";
        serverError.value = serverFailureReason.value;
      }
      startingServer.value = false;
    };
    
    // Setup event listener BEFORE calling startLlamaServer
    let eventReceived = false;
    const { listen } = await import("@tauri-apps/api/event");
    console.log("[DEBUG] Setting up event listener BEFORE startLlamaServer");
    const unlisten = await listen<number>("llama_server_started", (event) => {
      console.log("[DEBUG] Received llama_server_started event:", event.payload);
      eventReceived = true;
      unlisten();
      onServerStarted(event.payload);
    });
    
    console.log("Starting server with filename:", filename);
    const port = await startLlamaServer(filename);
    console.log("Server startLlamaServer returned port:", port);
    
    // Check if server was already running (no event will be fired)
    if (port > 0 && runningServerPort.value > 0) {
      console.log("[DEBUG] Server already running, calling onServerStarted directly");
      unlisten();
      onServerStarted(port);
      return;
    }
    
    // If server started successfully and event was received, onServerStarted already called
    // If server started but event wasn't received (new server), wait for it
    if (port > 0 && !eventReceived) {
      console.log("[DEBUG] New server starting, waiting for event...");
      setTimeout(() => {
        if (!eventReceived) {
          console.log("[DEBUG] Timeout fallback, calling onServerStarted with port:", port);
          onServerStarted(port);
        }
      }, 120000);
    }
    
  } catch (err: any) {
    console.error("Failed to start server:", err);
    serverError.value = err.message || err || "Failed to start server";
    startingServer.value = false;
  }
}

async function handleStopServer() {
  try {
    await stopLlamaServer();
    runningServerPort.value = 0;
    await loadLlamaInfo();
  } catch (err: any) {
    console.error("Failed to stop server:", err);
  }
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
      support_tools: true,
      tags: [],
      use_proxy: true,
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
    await assignModelToAgent(agentId, modelId, true);
  }
}

function getAgentDisplayName(agentId: string): string {
  const agent = agents.value.find(a => a.id === agentId);
  return agent?.display_name || agentId;
}

function getStatusText(): string {
  if (llamaServerStatus.value.startsWith("running")) {
    return "Running";
  }
  if (llamaServerAvailable.value) {
    return "Available";
  }
  return "Not available";
}

function getStatusColor(): string {
  if (llamaServerStatus.value.startsWith("running")) {
    return "bg-emerald-500";
  }
  if (llamaServerAvailable.value) {
    return "bg-blue-500";
  }
  return "bg-gray-300";
}

function openAgentDropdown(modelId: string, event: MouseEvent) {
  event.stopPropagation();
  const target = event.currentTarget as HTMLElement;
  const rect = target.getBoundingClientRect();
  const dropdownHeight = 320; // estimated max height of the dropdown
  const viewportHeight = window.innerHeight;

  // If there's not enough space below, open upward
  const spaceBelow = viewportHeight - rect.bottom;
  const top = spaceBelow >= dropdownHeight
    ? rect.bottom + 8
    : rect.top - dropdownHeight - 8;

  agentDropdownPosition.value = { x: rect.left, y: top };

  showAgentDropdown.value = modelId;
  console.log("Agent dropdown opened for:", modelId, "position:", agentDropdownPosition.value);
}

const commercialModels = computed(() => models.value.filter(m => m.provider !== "llama"));
const recommendedModelFilenames = ['ornith-1.0-9b-Q4_K_M.gguf', 'Qwen3-14B.Q4_K_M.gguf', 'qwen2.5-7b-instruct-q4_k_m-00001-of-00002.gguf', 'Qwen3-Coder-30B-A3B-Instruct-Q4_K_M.gguf'];
const userAddedModels = computed(() => {
  const result = models.value.filter(m => m.provider === "llama" && !recommendedModelFilenames.includes(m.name));
  console.log("[DEBUG] userAddedModels computed: models with provider=llama:", models.value.filter(m => m.provider === "llama").map(m => m.name));
  console.log("[DEBUG] userAddedModels result:", result.map(m => m.name));
  console.log("[DEBUG] runningServerModel:", runningServerModel.value);
  return result;
});
</script>

<template>
  <div class="max-w-3xl mx-auto p-8">
    <div class="mb-6 flex items-center justify-between">
      <div>
        <h2 class="text-[18px] font-semibold text-gray-900 tracking-tight">Models</h2>
        <p class="text-[13px] text-gray-500 mt-0.5">Configure LLM providers shared across all agents</p>
      </div>
      <button
        @click="refreshModels"
        :disabled="refreshing"
        class="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-medium bg-gray-50 border border-gray-200 text-gray-600 hover:bg-gray-100 disabled:opacity-50 transition-all duration-150 cursor-pointer active:scale-[0.98]"
      >
        <RefreshCw :size="14" :class="{ 'animate-spin': refreshing }" />
        {{ refreshing ? 'Refreshing...' : 'Refresh' }}
      </button>
    </div>

    <div v-if="!llamaServerAvailable" class="mb-6">
      <div class="flex items-center gap-3 bg-amber-50 border border-amber-200 rounded-xl px-4 py-3">
        <div class="text-amber-500">💡</div>
        <div class="flex-1">
          <p class="text-[13px] font-medium text-amber-800">Llama.cpp server not available</p>
          <p class="text-[12px] text-amber-600 mt-0.5">Please ensure llama-server binaries are present in the correct location</p>
        </div>
        <button @click="openUrl('https://github.com/ggerganov/llama.cpp')" class="flex items-center gap-1 px-3 py-1.5 rounded-lg bg-amber-500 text-white text-[12px] font-medium hover:bg-amber-600 active:scale-[0.98] transition-all duration-150 cursor-pointer shadow-sm">
          Download <ExternalLink :size="12" />
        </button>
      </div>
    </div>

    <div v-if="llamaServerAvailable && recommendedLocalModels.length > 0" class="mb-6">
      <div class="flex items-center justify-between mb-4">
        <div class="flex items-center gap-2">
          <img :src="getProviderLogo('llama')" alt="Llama.cpp" class="w-5 h-5 object-contain" />
          <span class="text-[15px] font-semibold text-gray-900">Local Models</span>
          <span :class="['w-2 h-2 rounded-full', getStatusColor()]"></span>
          <span class="text-[12px] text-gray-500">{{ getStatusText() }}</span>
        </div>
        <div class="flex items-center gap-2">
          <button @click="openLlamaModelsDir" class="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-medium bg-gray-50 border border-gray-200 text-gray-600 hover:bg-gray-100 active:scale-[0.98] transition-all duration-150 cursor-pointer">
            <FolderOpen :size="14" /> Open Folder
          </button>
          <button @click="showAddLocalModel = true"
            class="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-semibold bg-indigo-600 text-white hover:bg-indigo-700 active:scale-[0.98] transition-all duration-150 cursor-pointer shadow-sm">
            <Plus :size="12" /> Add Model
          </button>
        </div>
      </div>

      <div class="space-y-2.5">
        <div 
          v-for="rm in recommendedLocalModels" 
          :key="rm.name"
          :class="[
            'flex items-center gap-4 px-4 py-3 rounded-xl border transition-all duration-150',
            isModelRunning(rm.filename) ? 'border-emerald-300 bg-emerald-50' : 
            isModelInstalled(rm.filename) ? 'border-gray-200 bg-gray-50' : 'border-gray-100 bg-white hover:border-gray-200'
          ]"
        >
          <div class="flex-1 min-w-0">
            <div class="flex items-center gap-2">
              <span class="text-[15px] font-medium text-gray-900 truncate">{{ rm.alias }}</span>
              <span v-if="isModelRunning(rm.filename)" class="flex items-center gap-0.5 text-[11px] text-emerald-700 bg-emerald-100 px-2 py-0.5 rounded-full">
                <span class="w-1.5 h-1.5 bg-emerald-500 rounded-full animate-pulse"></span> Running
              </span>
              <span v-else-if="isModelInstalled(rm.filename)" class="flex items-center gap-0.5 text-[11px] text-emerald-600 bg-emerald-50 px-2 py-0.5 rounded-full">
                <Check :size="11" /> Installed
              </span>
            </div>
            <p class="text-[12px] text-gray-400 mt-0.5">{{ rm.name }}</p>
          </div>

          <span class="text-[12px] font-medium text-gray-500 bg-gray-100 px-2.5 py-1 rounded-lg flex-shrink-0">{{ rm.size }}</span>

          <div class="flex items-center gap-2 flex-shrink-0">
            <template v-if="pullingModel === rm.filename">
              <div class="w-24">
                <div class="h-1.5 bg-gray-200 rounded-full overflow-hidden">
                  <div 
                    class="h-full bg-gray-700 transition-all duration-200"
                    :style="{ width: `${pullProgress?.percentage || 0}%` }"
                  ></div>
                </div>
              </div>
            </template>
            <template v-else-if="isModelInstalled(rm.filename)">
              <div class="flex items-center gap-2 flex-shrink-0">
                <template v-if="startingServer && !isModelRunning(rm.filename)">
                  <span class="flex items-center gap-1 px-3 py-1.5 rounded-lg text-[12px] font-medium bg-gray-200 text-gray-500">
                    <div class="w-3 h-3 border-2 border-gray-400 border-t-gray-700 rounded-full animate-spin"></div>
                    Starting...
                  </span>
                </template>
                <template v-else-if="isModelRunning(rm.filename)">
                  <button
                    @click="handleStopServer"
                    class="flex items-center gap-1 px-3 py-1.5 rounded-lg text-[12px] font-medium bg-red-500 text-white hover:bg-red-600 active:scale-[0.98] transition-all duration-150 cursor-pointer shadow-sm"
                  >
                    <Square :size="12" /> Stop
                  </button>
                </template>
                <template v-else>
                  <button
                    v-if="!runningServerPort"
                    @click="handleStartServer(rm.filename)"
                    class="flex items-center gap-1 px-3 py-1.5 rounded-lg text-[12px] font-medium bg-blue-50 border border-blue-200 text-blue-700 hover:bg-blue-100 active:scale-[0.98] transition-all duration-150 cursor-pointer"
                  >
                    <Play :size="12" /> Start
                  </button>
                </template>
                <span v-if="models.some(m => getFilename(m.name) === rm.filename && m.provider === 'llama')" class="flex items-center gap-1 text-[12px] text-emerald-600">
                  <Check :size="12" /> Added
                </span>
              </div>
            </template>
            <template v-else>
              <button
                @click="downloadModel(rm.hfRepo, rm.filename)"
                class="flex items-center gap-1 px-3 py-1.5 rounded-lg text-[12px] font-semibold bg-indigo-600 text-white hover:bg-indigo-700 active:scale-[0.98] transition-all duration-150 cursor-pointer shadow-sm"
              >
                <Download :size="12" /> Download
              </button>
            </template>
          </div>
        </div>

        <div v-if="userAddedModels.length > 0" class="mt-4 pt-4 border-t border-gray-100 space-y-3">
          <div 
            v-for="model in userAddedModels" 
            :key="model.id"
            :class="[
              'flex items-center gap-4 px-4 py-3 rounded-xl border transition-all duration-150',
              isModelRunning(model.name) ? 'border-emerald-300 bg-emerald-50' : 'border-gray-200 bg-gray-50'
            ]"
          >
            <div class="flex-1 min-w-0">
              <div class="flex items-center gap-2">
                <span class="text-[15px] font-medium text-gray-900 truncate">{{ model.alias || model.name }}</span>
                <span v-if="isModelRunning(model.name)" class="flex items-center gap-0.5 text-[11px] text-emerald-700 bg-emerald-100 px-2 py-0.5 rounded-full">
                  <span class="w-1.5 h-1.5 bg-emerald-500 rounded-full animate-pulse"></span> Running
                </span>
                <span v-else class="flex items-center gap-0.5 text-[11px] text-emerald-600 bg-emerald-50 px-2 py-0.5 rounded-full">
                  <Check :size="11" /> Added
                </span>
              </div>
              <p class="text-[12px] text-gray-400 mt-0.5">{{ model.name }}</p>
            </div>

            <div class="flex items-center gap-2 flex-shrink-0">
              <template v-if="startingServer && !isModelRunning(model.name)">
                <span class="flex items-center gap-1 px-3 py-1.5 rounded-lg text-[12px] font-medium bg-gray-200 text-gray-500">
                  <div class="w-3 h-3 border-2 border-gray-400 border-t-gray-700 rounded-full animate-spin"></div>
                  Starting...
                </span>
              </template>
              <template v-else-if="isModelRunning(model.name)">
                <button
                  @click="handleStopServer"
                  class="flex items-center gap-1 px-3 py-1.5 rounded-lg text-[12px] font-medium bg-red-500 text-white hover:bg-red-600 active:scale-[0.98] transition-all duration-150 cursor-pointer shadow-sm"
                >
                  <Square :size="12" /> Stop
                </button>
              </template>
              <template v-else>
                <button
                  v-if="!runningServerPort"
                  @click="handleStartServer(model.name)"
                  class="flex items-center gap-1 px-3 py-1.5 rounded-lg text-[12px] font-medium bg-blue-50 border border-blue-200 text-blue-700 hover:bg-blue-100 active:scale-[0.98] transition-all duration-150 cursor-pointer"
                >
                  <Play :size="12" /> Start
                </button>
                <button
                  @click="removeModel(model.id)"
                  class="p-1.5 rounded-lg text-gray-400 hover:text-red-500 hover:bg-red-50 active:scale-[0.98] transition-all duration-150 cursor-pointer"
                  title="Remove model"
                >
                  <Trash2 :size="14" />
                </button>
              </template>
            </div>
          </div>
        </div>
      </div>
    </div>



    <div v-if="(llamaServerAvailable && recommendedLocalModels.length > 0) || llamaModels.length > 0" class="border-t border-gray-200 my-8"></div>

    <div v-if="commercialModels.length > 0 || (llamaServerAvailable && recommendedLocalModels.length > 0) || llamaModels.length > 0" class="space-y-2">
      <div class="flex items-center justify-between mb-4">
        <div class="flex items-center gap-2">
          <span class="text-[15px] font-semibold text-gray-900">Commercial Models</span>
          <span class="text-[12px] text-gray-400">({{ commercialModels.length }})</span>
        </div>
        <button @click="showAdd = true; nameError = ''; apiKeyError = ''"
          class="flex items-center gap-1.5 px-4 py-2 rounded-xl text-[13px] font-semibold bg-indigo-600 text-white hover:bg-indigo-700 active:scale-[0.98] transition-all duration-150 shadow-sm cursor-pointer">
          <Plus :size="15" /> Add Model
        </button>
      </div>

      <div v-for="model in commercialModels" :key="model.id"
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
          <div class="flex items-center gap-2">
            <button 
              @click="openAgentDropdown(model.id, $event)"
              class="agent-dropdown-btn flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-medium bg-gray-100 text-gray-600 hover:bg-gray-200 active:scale-[0.98] transition-all duration-150 cursor-pointer"
            >
              <Users :size="12" />
              Agents: {{ model.assignedAgents.length }}
              <ChevronDown :size="12" />
            </button>
            <button @click="removeModel(model.id)"
              class="p-2 rounded-lg text-gray-300 hover:text-red-500 hover:bg-red-50 opacity-0 group-hover:opacity-100 active:scale-[0.98] transition-all duration-150 flex-shrink-0 cursor-pointer">
              <Trash2 :size="15" />
            </button>
          </div>
        </div>
      </div>
    </div>

    <div v-if="commercialModels.length === 0 && !(llamaServerAvailable && recommendedLocalModels.length > 0) && !(llamaModels.length > 0) && !showAdd" class="text-center py-20">
      <div class="w-14 h-14 rounded-2xl bg-gray-100 flex items-center justify-center mx-auto mb-4 text-[24px]">
        ⚡
      </div>
      <h3 class="text-[15px] font-medium text-gray-700 mb-1">No models configured</h3>
      <p class="text-[13px] text-gray-400 mb-4">Add a model to start using it in conversations</p>
      <button @click="showAdd = true"
        class="px-4 py-2 rounded-xl text-[13px] font-semibold bg-indigo-600 text-white hover:bg-indigo-700 active:scale-[0.98] transition-all duration-150 shadow-sm cursor-pointer">
        Add your first model
      </button>
    </div>

    <Teleport to="body">
      <div v-if="showAgentDropdown" class="fixed z-[60]">
        <div 
          class="agent-dropdown-panel bg-white rounded-xl shadow-xl border border-gray-200 p-2 min-w-[280px] max-h-[320px] overflow-y-auto"
          :style="{ left: `${agentDropdownPosition.x}px`, top: `${agentDropdownPosition.y}px`, position: 'fixed', zIndex: 1000 }"
        >
          <div class="flex items-center justify-between px-3 py-2 border-b border-gray-100 mb-2">
            <span class="text-[13px] font-semibold text-gray-900">Assign to Agents</span>
            <button @click="showAgentDropdown = null" class="p-1 text-gray-400 hover:text-gray-600 cursor-pointer">
              <X :size="14" />
            </button>
          </div>
          <div class="space-y-1">
            <div
              v-for="agent in agents.filter(a => a.installed)"
              :key="agent.id"
              @click="toggleAgentAssignment(showAgentDropdown!, agent.id)"
              :class="[
                'flex items-center gap-3 px-3 py-2 rounded-lg border transition-all duration-150 cursor-pointer',
                (models.find(m => m.id === showAgentDropdown)?.assignedAgents.includes(agent.id))
                  ? 'bg-gray-50 border-gray-200'
                  : 'border-gray-100 hover:border-gray-200 hover:bg-gray-50'
              ]"
            >
              <AgentIcon :agent-id="agent.id" :size="20" />
              <div class="flex-1 min-w-0">
                <span class="text-[13px] font-medium text-gray-900">{{ getAgentDisplayName(agent.id) }}</span>
                <span v-if="agent.status === 'available'" class="ml-1.5 inline-block w-1.5 h-1.5 rounded-full bg-emerald-500"></span>
                <span v-else-if="agent.status === 'connection_failed'" class="ml-1.5 inline-block w-1.5 h-1.5 rounded-full bg-red-400"></span>
              </div>
              <div
                :class="[
                  'w-5 h-5 rounded-full border-2 flex items-center justify-center',
                  (models.find(m => m.id === showAgentDropdown)?.assignedAgents.includes(agent.id))
                    ? 'border-gray-700 bg-gray-700'
                    : 'border-gray-300'
                ]">
                <Check v-if="models.find(m => m.id === showAgentDropdown)?.assignedAgents.includes(agent.id)" :size="12" class="text-white" />
              </div>
            </div>
            <div v-if="agents.filter(a => a.installed).length === 0" class="text-center py-4">
              <p class="text-[12px] text-gray-400">No agents installed</p>
            </div>
          </div>
        </div>
      </div>
    </Teleport>

    <div v-if="showAddLocalModel" class="fixed inset-0 z-50 flex items-center justify-center p-4">
      <div class="absolute inset-0 bg-black/40 backdrop-blur-sm transition-opacity" @click="showAddLocalModel = false"></div>
      <div class="relative w-full max-w-md bg-white rounded-2xl shadow-xl border border-gray-100 overflow-hidden animate-in fade-in zoom-in duration-200">
        <div class="px-6 py-4 border-b border-gray-100 bg-gray-50">
          <h3 class="text-[16px] font-semibold text-gray-900">Add Local Model</h3>
        </div>
        <div class="p-6 space-y-4">
          <div>
            <label class="block text-[12px] font-medium text-gray-500 mb-2">Model Name</label>
            <input v-model="newLocalModelName" type="text" placeholder="e.g. deepreinforce-ai/Ornith-1.0-9B-GGUF"
              class="w-full px-3 py-2 rounded-xl border border-gray-200 bg-white text-[13px] text-gray-900 placeholder-gray-400 outline-none focus:ring-2 focus:ring-gray-600/20 focus:border-gray-400 transition-all" />
            <p class="text-[11px] text-gray-400 mt-1.5">Enter the full model name (e.g. repo/model.gguf)</p>
          </div>
          <div class="bg-blue-50 border border-blue-200 rounded-xl p-3">
            <p class="text-[12px] text-blue-700">💡 You can download GGUF models from <a href="https://huggingface.co/models" target="_blank" class="text-blue-600 underline hover:text-blue-800">Hugging Face</a> and place them in the models folder.</p>
            <button @click="openLlamaModelsDir" class="mt-2 text-[11px] text-blue-600 hover:text-blue-800 underline cursor-pointer">Open Models Folder</button>
          </div>
        </div>
        <div class="px-6 py-4 border-t border-gray-100 bg-gray-50 flex gap-2 justify-end">
          <button @click="showAddLocalModel = false" class="px-4 py-2 rounded-xl text-[13px] font-medium text-gray-600 bg-gray-50 border border-gray-200 hover:bg-gray-100 active:scale-[0.98] transition-all duration-150 cursor-pointer">Cancel</button>
          <button @click="handleAddLocalModel" class="px-5 py-2 rounded-xl text-[13px] font-semibold text-white bg-indigo-600 hover:bg-indigo-700 active:scale-[0.98] transition-all duration-150 shadow-sm cursor-pointer">Add Model</button>
        </div>
      </div>
    </div>

    <Teleport to="body">
      <div v-if="startingServer || serverStartFailed" class="fixed inset-0 z-[9999] flex items-center justify-center p-4">
        <div class="absolute inset-0 bg-black/40 backdrop-blur-sm"></div>
        <div class="relative bg-white rounded-2xl shadow-xl border border-gray-100 p-6 w-full max-w-lg">
          <div v-if="startingServer" class="flex items-center gap-3 mb-4">
            <div class="w-8 h-8 border-4 border-gray-200 border-t-gray-700 rounded-full animate-spin"></div>
            <span class="text-[14px] font-medium text-gray-700">Starting Llama.cpp server...</span>
          </div>
          <div v-else-if="serverStartFailed" class="flex items-center gap-3 mb-4">
            <div class="w-8 h-8 rounded-full bg-red-100 flex items-center justify-center">
              <span class="text-red-500 text-lg">✕</span>
            </div>
            <span class="text-[14px] font-medium text-red-700">Failed to start server</span>
          </div>
          
          <p v-if="startingServer" class="text-[12px] text-gray-400 mb-3">This may take a while as the model loads into memory.</p>
          <p v-else-if="serverStartFailed" class="text-[12px] text-red-500 mb-3">{{ serverFailureReason }}</p>
          
          <div v-if="serverLogs.length > 0" class="bg-gray-900 rounded-lg p-3 max-h-48 overflow-y-auto">
            <div v-for="(log, index) in serverLogs.slice(-20)" :key="index" 
              :class="['text-[11px] font-mono leading-relaxed', log.includes('[ERROR]') ? 'text-red-400' : 'text-gray-300']">
              {{ log }}
            </div>
          </div>
          
          <div v-if="serverStartFailed" class="flex gap-2 mt-4">
            <button @click="serverStartFailed = false; serverFailureReason = ''; serverLogs = []"
              class="flex-1 px-4 py-2 rounded-xl text-[13px] font-semibold bg-indigo-600 text-white hover:bg-indigo-700 active:scale-[0.98] transition-all duration-150 cursor-pointer shadow-sm">
              Close
            </button>
          </div>
        </div>
      </div>
    </Teleport>

    <div v-if="serverError && !serverStartFailed" class="fixed inset-0 z-50 flex items-center justify-center p-4">
      <div class="absolute inset-0 bg-black/40 backdrop-blur-sm transition-opacity" @click="serverError = ''"></div>
      <div class="relative bg-white rounded-2xl shadow-xl border border-gray-100 p-6 animate-in fade-in zoom-in duration-200 max-w-sm">
        <div class="flex items-center gap-2 mb-3">
          <div class="w-6 h-6 rounded-full bg-red-100 flex items-center justify-center">
            <span class="text-red-500 text-sm">!</span>
          </div>
          <span class="text-[14px] font-semibold text-gray-900">Server Error</span>
        </div>
        <p class="text-[13px] text-gray-600 mb-4">{{ serverError }}</p>
        <button @click="serverError = ''" class="w-full px-4 py-2 rounded-xl text-[13px] font-semibold bg-indigo-600 text-white hover:bg-indigo-700 active:scale-[0.98] transition-all duration-150 cursor-pointer shadow-sm">
          OK
        </button>
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
              <button v-for="p in providers.filter(p => p.id !== 'llama')" :key="p.id"
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
          <button @click="showAdd = false" class="px-4 py-2 rounded-xl text-[13px] font-medium text-gray-600 bg-gray-50 border border-gray-200 hover:bg-gray-100 active:scale-[0.98] transition-all duration-150 cursor-pointer">Cancel</button>
          <button @click="addModel" class="px-5 py-2 rounded-xl text-[13px] font-semibold text-white bg-indigo-600 hover:bg-indigo-700 active:scale-[0.98] transition-all duration-150 shadow-sm cursor-pointer">Add Model</button>
        </div>
      </div>
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