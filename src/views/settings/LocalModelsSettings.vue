<script setup lang="ts">
import { ref, onMounted, computed } from "vue";
import { storeToRefs } from "pinia";
import { Plus, Check, ExternalLink, Play, Square, FolderOpen, Trash2, RefreshCw, Download } from "lucide-vue-next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { listen } from "@tauri-apps/api/event";
import {
  getModels, saveModels, getProviderById,
  downloadLlamaModel, startLlamaServer, stopLlamaServer, openLlamaModelsDir,
  recommendedLocalModels,
  type LlamaPullProgress, type ProtocolType,
} from "../../api/models";
import { getProviderLogo } from "../../utils/providerIcons";
import ConfirmDialog from "../../components/ConfirmDialog.vue";
import { t } from "../../i18n";
import { track } from "../../api/telemetry";
import { useLlamaStore } from "../../stores/useLlamaStore";

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
const refreshing = ref(false);

const pullingModel = ref<string | null>(null);
const pullProgress = ref<LlamaPullProgress | null>(null);

// Llama.cpp server state lives in the shared store so it can be prefetched
// at app startup; the page binds to it and refreshes in the background so it
// renders instantly instead of blocking on the slow port probes.
const llamaStore = useLlamaStore();
const {
  available: llamaServerAvailable,
  serverStatus: llamaServerStatus,
  models: llamaModels,
  runningPort: runningServerPort,
  runningModel: runningServerModel,
  downloadStatus,
} = storeToRefs(llamaStore);

const showAddLocalModel = ref(false);
const newLocalModelName = ref("");
const startingServer = ref(false);
const serverError = ref("");
const serverLogs = ref<string[]>([]);
const serverStartFailed = ref(false);
const serverFailureReason = ref("");
const pullError = ref("");
const showDeleteDialog = ref(false);
const deletingModelId = ref<string | null>(null);

function getFilename(name: string): string {
  const parts = name.split('/');
  return parts[parts.length - 1];
}

function isModelRunning(filename: string): boolean {
  const modelFilename = getFilename(filename);
  const runningFilename = runningServerModel.value ? getFilename(runningServerModel.value) : '';
  return runningServerPort.value > 0 && runningFilename === modelFilename;
}

const runningServerUrl = computed(() => {
  const port = runningServerPort.value > 0 ? runningServerPort.value : 19090;
  return `http://127.0.0.1:${port}`;
});

function openLocalServer() {
  openUrl(runningServerUrl.value);
}

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
    await llamaStore.refresh();
    syncRunningModel();
    restoreDownloadStatus();
  } catch {} finally {
    refreshing.value = false;
  }
}

onMounted(() => {
  refreshModels();

  listen<LlamaPullProgress>("llama_pull_progress", (event) => {
    pullProgress.value = event.payload;
    if (event.payload.status === "completed") {
      if (pullingModel.value) {
        onModelDownloaded(pullingModel.value);
      }
      pullingModel.value = null;
      llamaStore.loadModels();
    } else if (event.payload.status === "failed") {
      pullingModel.value = null;
      llamaStore.loadModels();
    }
  });

  listen<string>("llama_pull_error", (event) => {
    pullError.value = event.payload;
    console.error("Llama pull error:", event.payload);
  });

  listen<string>("llama_server_log", (event) => {
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
});

/** Reconcile the models list with a detected running llama server: update the
 *  matching entry's API base or auto-add the model if it isn't there yet. */
function syncRunningModel() {
  const port = llamaStore.runningPort;
  const modelName = llamaStore.runningModel;
  if (!port || !modelName) return;

  const provider = getProviderById("llama");
  const modelFilename = getFilename(modelName);
  const existingModel = models.value.find(m => getFilename(m.name) === modelFilename && m.provider === "llama");
  if (existingModel) {
    existingModel.apiBase = `http://localhost:${port}/v1`;
    persistModels();
  } else if (provider) {
    models.value.push({
      id: `llama-${modelName}-auto`,
      name: modelName,
      alias: modelName,
      provider: "llama",
      apiBase: `http://localhost:${port}/v1`,
      apiKey: "llama",
      protocol: provider.protocol,
      showKey: false,
      assignedAgents: [],
      useProxy: {},
    });
    persistModels();
  }
}

/** Restore an in-flight model pull from the persisted status (e.g. when the
 *  app was restarted mid-download). */
function restoreDownloadStatus() {
  if (downloadStatus.value?.downloading) {
    pullingModel.value = downloadStatus.value.downloading;
    pullProgress.value = downloadStatus.value.progress;
  }
}

function isModelInstalled(filename: string): boolean {
  return llamaModels.value.some(m => m.name === filename);
}

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

  track("llama_server_start", { model: filename });
  startingServer.value = true;
  serverError.value = "";
  serverLogs.value = [];
  serverStartFailed.value = false;
  serverFailureReason.value = "";

  try {
    const onServerStarted = (startedPort: number) => {
      if (startedPort > 0) {
        runningServerPort.value = startedPort;
        llamaStore.refresh();

        const provider = getProviderById("llama")!;
        const modelFilename = getFilename(filename);
        const existingModel = models.value.find(m => getFilename(m.name) === modelFilename && m.provider === "llama");

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
          persistModels();
        }
        serverStartFailed.value = false;
      } else {
        serverStartFailed.value = true;
        serverFailureReason.value = serverFailureReason.value || t("models.startFailedTimeout");
        serverError.value = serverFailureReason.value;
      }
      startingServer.value = false;
    };

    // Setup event listener BEFORE calling startLlamaServer
    let eventReceived = false;
    const { listen } = await import("@tauri-apps/api/event");
    const unlisten = await listen<number>("llama_server_started", (event) => {
      eventReceived = true;
      unlisten();
      onServerStarted(event.payload);
    });

    const port = await startLlamaServer(filename);

    if (port > 0 && runningServerPort.value > 0) {
      unlisten();
      onServerStarted(port);
      return;
    }

    if (port > 0 && !eventReceived) {
      setTimeout(() => {
        if (!eventReceived) {
          onServerStarted(port);
        }
      }, 120000);
    }
  } catch (err: any) {
    console.error("Failed to start server:", err);
    serverError.value = err.message || err || t("models.startFailed");
    startingServer.value = false;
  }
}

async function handleStopServer() {
  try {
    await stopLlamaServer();
    runningServerPort.value = 0;
    await llamaStore.refresh();
  } catch (err: any) {
    console.error("Failed to stop server:", err);
  }
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

function getStatusText(): string {
  if (llamaServerStatus.value.startsWith("running")) {
    return t("models.running");
  }
  if (llamaServerAvailable.value) {
    return t("models.available");
  }
  return t("models.notAvailable");
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

const recommendedModelFilenames = ['ornith-1.0-9b-Q4_K_M.gguf', 'Qwen3-14B.Q4_K_M.gguf', 'qwen2.5-7b-instruct-q4_k_m-00001-of-00002.gguf', 'Qwen3-Coder-30B-A3B-Instruct-Q4_K_M.gguf'];
const userAddedModels = computed(() => {
  return models.value.filter(m => m.provider === "llama" && !recommendedModelFilenames.includes(m.name));
});
</script>

<template>
  <div class="max-w-3xl mx-auto p-8">
    <div class="mb-6 flex items-center justify-between">
      <div>
        <h2 class="text-[18px] font-semibold text-gray-900 tracking-tight">{{ $t("settings.localModels") }}</h2>
        <p class="text-[13px] text-gray-500 mt-0.5">{{ $t("models.subtitle") }}</p>
      </div>
      <button
        @click="refreshModels"
        :disabled="refreshing"
        class="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-medium bg-gray-50 border border-gray-200 text-gray-600 hover:bg-gray-100 disabled:opacity-50 transition-all duration-150 cursor-pointer active:scale-[0.98]"
      >
        <RefreshCw :size="14" :class="{ 'animate-spin': refreshing }" />
        {{ refreshing ? $t('models.refreshing') : $t('models.refresh') }}
      </button>
    </div>

    <div v-if="!llamaServerAvailable" class="mb-6">
      <div class="flex items-center gap-3 bg-amber-50 border border-amber-200 rounded-xl px-4 py-3">
        <div class="text-amber-500">💡</div>
        <div class="flex-1">
          <p class="text-[13px] font-medium text-amber-800">{{ $t("models.llamaNotAvailable") }}</p>
          <p class="text-[12px] text-amber-600 mt-0.5">{{ $t("models.llamaNotAvailableDesc") }}</p>
        </div>
        <button @click="openUrl('https://github.com/ggerganov/llama.cpp')" class="flex items-center gap-1 px-3 py-1.5 rounded-lg bg-amber-500 text-white text-[12px] font-medium hover:bg-amber-600 active:scale-[0.98] transition-all duration-150 cursor-pointer shadow-sm">
          {{ $t("models.download") }} <ExternalLink :size="12" />
        </button>
      </div>
    </div>

    <div v-if="llamaServerAvailable && recommendedLocalModels.length > 0" class="mb-6">
      <div class="flex items-center justify-between mb-4">
        <div class="flex items-center gap-2">
          <img :src="getProviderLogo('llama')" alt="Llama.cpp" class="w-5 h-5 object-contain" />
          <span class="text-[15px] font-semibold text-gray-900">{{ $t("models.localModels") }}</span>
          <span :class="['w-2 h-2 rounded-full', getStatusColor()]"></span>
          <span class="text-[12px] text-gray-500">{{ getStatusText() }}</span>
        </div>
        <div class="flex items-center gap-2">
          <button @click="openLlamaModelsDir" class="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-medium bg-gray-50 border border-gray-200 text-gray-600 hover:bg-gray-100 active:scale-[0.98] transition-all duration-150 cursor-pointer">
            <FolderOpen :size="14" /> {{ $t("models.openFolder") }}
          </button>
          <button @click="showAddLocalModel = true"
            class="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-semibold bg-indigo-600 text-white hover:bg-indigo-700 active:scale-[0.98] transition-all duration-150 cursor-pointer shadow-sm">
            <Plus :size="12" /> {{ $t("models.addModel") }}
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
                <span class="w-1.5 h-1.5 bg-emerald-500 rounded-full animate-pulse"></span> {{ $t("models.running") }}
              </span>
              <a
                v-if="isModelRunning(rm.filename)"
                @click.prevent="openLocalServer"
                class="flex items-center gap-0.5 text-[11px] text-blue-600 bg-blue-50 hover:bg-blue-100 px-2 py-0.5 rounded-full transition-all duration-150 cursor-pointer"
                :title="$t('models.openInBrowser')"
              >
                <ExternalLink :size="11" /> {{ runningServerUrl }}
              </a>
              <span v-else-if="isModelInstalled(rm.filename)" class="flex items-center gap-0.5 text-[11px] text-emerald-600 bg-emerald-50 px-2 py-0.5 rounded-full">
                <Check :size="11" /> {{ $t("models.installed") }}
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
                    {{ $t("models.starting") }}
                  </span>
                </template>
                <template v-else-if="isModelRunning(rm.filename)">
                  <button
                    @click="handleStopServer"
                    class="flex items-center gap-1 px-3 py-1.5 rounded-lg text-[12px] font-medium bg-red-500 text-white hover:bg-red-600 active:scale-[0.98] transition-all duration-150 cursor-pointer shadow-sm"
                  >
                    <Square :size="12" /> {{ $t("models.stop") }}
                  </button>
                </template>
                <template v-else>
                  <button
                    v-if="!runningServerPort"
                    @click="handleStartServer(rm.filename)"
                    class="flex items-center gap-1 px-3 py-1.5 rounded-lg text-[12px] font-medium bg-blue-50 border border-blue-200 text-blue-700 hover:bg-blue-100 active:scale-[0.98] transition-all duration-150 cursor-pointer"
                  >
                    <Play :size="12" /> {{ $t("models.start") }}
                  </button>
                </template>
                <span v-if="models.some(m => getFilename(m.name) === rm.filename && m.provider === 'llama')" class="flex items-center gap-1 text-[12px] text-emerald-600">
                  <Check :size="12" /> {{ $t("models.added") }}
                </span>
              </div>
            </template>
            <template v-else>
              <button
                @click="downloadModel(rm.hfRepo, rm.filename)"
                class="flex items-center gap-1 px-3 py-1.5 rounded-lg text-[12px] font-semibold bg-indigo-600 text-white hover:bg-indigo-700 active:scale-[0.98] transition-all duration-150 cursor-pointer shadow-sm"
              >
                <Download :size="12" /> {{ $t("models.download") }}
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
                  <span class="w-1.5 h-1.5 bg-emerald-500 rounded-full animate-pulse"></span> {{ $t("models.running") }}
                </span>
                <a
                  v-if="isModelRunning(model.name)"
                  @click.prevent="openLocalServer"
                  class="flex items-center gap-0.5 text-[11px] text-blue-600 bg-blue-50 hover:bg-blue-100 px-2 py-0.5 rounded-full transition-all duration-150 cursor-pointer"
                  :title="$t('models.openInBrowser')"
                >
                  <ExternalLink :size="11" /> {{ runningServerUrl }}
                </a>
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
                  {{ $t("models.starting") }}
                </span>
              </template>
              <template v-else-if="isModelRunning(model.name)">
                <button
                  @click="handleStopServer"
                  class="flex items-center gap-1 px-3 py-1.5 rounded-lg text-[12px] font-medium bg-red-500 text-white hover:bg-red-600 active:scale-[0.98] transition-all duration-150 cursor-pointer shadow-sm"
                >
                  <Square :size="12" /> {{ $t("models.stop") }}
                </button>
              </template>
              <template v-else>
                <button
                  v-if="!runningServerPort"
                  @click="handleStartServer(model.name)"
                  class="flex items-center gap-1 px-3 py-1.5 rounded-lg text-[12px] font-medium bg-blue-50 border border-blue-200 text-blue-700 hover:bg-blue-100 active:scale-[0.98] transition-all duration-150 cursor-pointer"
                >
                  <Play :size="12" /> {{ $t("models.start") }}
                </button>
                <button
                  @click="removeModel(model.id)"
                  class="p-1.5 rounded-lg text-gray-400 hover:text-red-500 hover:bg-red-50 active:scale-[0.98] transition-all duration-150 cursor-pointer"
                  :title="$t('models.removeModel')"
                >
                  <Trash2 :size="14" />
                </button>
              </template>
            </div>
          </div>
        </div>
      </div>
    </div>

    <div v-if="showAddLocalModel" class="fixed inset-0 z-50 flex items-center justify-center p-4">
      <div class="absolute inset-0 bg-black/40 backdrop-blur-sm transition-opacity" @click="showAddLocalModel = false"></div>
      <div class="relative w-full max-w-md bg-white rounded-2xl shadow-xl border border-gray-100 overflow-hidden animate-in fade-in zoom-in duration-200">
        <div class="px-6 py-4 border-b border-gray-100 bg-gray-50">
          <h3 class="text-[16px] font-semibold text-gray-900">{{ $t("models.addLocalModel") }}</h3>
        </div>
        <div class="p-6 space-y-4">
          <div>
            <label class="block text-[12px] font-medium text-gray-500 mb-2">{{ $t("models.modelName") }}</label>
            <input v-model="newLocalModelName" type="text" :placeholder="$t('models.modelNamePlaceholderLocal')"
              class="w-full px-3 py-2 rounded-xl border border-gray-200 bg-white text-[13px] text-gray-900 placeholder-gray-400 outline-none focus:ring-2 focus:ring-gray-600/20 focus:border-gray-400 transition-all" />
            <p class="text-[11px] text-gray-400 mt-1.5">{{ $t("models.modelNameHint") }}</p>
          </div>
          <div class="bg-blue-50 border border-blue-200 rounded-xl p-3">
            <p class="text-[12px] text-blue-700">💡 {{ $t("models.ggufHint1") }}<a href="https://huggingface.co/models" target="_blank" class="text-blue-600 underline hover:text-blue-800">{{ $t("models.huggingFace") }}</a>{{ $t("models.ggufHint2") }}</p>
            <button @click="openLlamaModelsDir" class="mt-2 text-[11px] text-blue-600 hover:text-blue-800 underline cursor-pointer">{{ $t("models.openModelsFolder") }}</button>
          </div>
        </div>
        <div class="px-6 py-4 border-t border-gray-100 bg-gray-50 flex gap-2 justify-end">
          <button @click="showAddLocalModel = false" class="px-4 py-2 rounded-xl text-[13px] font-medium text-gray-600 bg-gray-50 border border-gray-200 hover:bg-gray-100 active:scale-[0.98] transition-all duration-150 cursor-pointer">{{ $t("models.cancel") }}</button>
          <button @click="handleAddLocalModel" class="px-5 py-2 rounded-xl text-[13px] font-semibold text-white bg-indigo-600 hover:bg-indigo-700 active:scale-[0.98] transition-all duration-150 shadow-sm cursor-pointer">{{ $t("models.addModel") }}</button>
        </div>
      </div>
    </div>

    <Teleport to="body">
      <div v-if="startingServer || serverStartFailed" class="fixed inset-0 z-[9999] flex items-center justify-center p-4">
        <div class="absolute inset-0 bg-black/40 backdrop-blur-sm"></div>
        <div class="relative bg-white rounded-2xl shadow-xl border border-gray-100 p-6 w-full max-w-lg">
          <div v-if="startingServer" class="flex items-center gap-3 mb-4">
            <div class="w-8 h-8 border-4 border-gray-200 border-t-gray-700 rounded-full animate-spin"></div>
            <span class="text-[14px] font-medium text-gray-700">{{ $t("models.startingServer") }}</span>
          </div>
          <div v-else-if="serverStartFailed" class="flex items-center gap-3 mb-4">
            <div class="w-8 h-8 rounded-full bg-red-100 flex items-center justify-center">
              <span class="text-red-500 text-lg">✕</span>
            </div>
            <span class="text-[14px] font-medium text-red-700">{{ $t("models.startFailed") }}</span>
          </div>

          <p v-if="startingServer" class="text-[12px] text-gray-400 mb-3">{{ $t("models.startHint") }}</p>
          <p v-else-if="serverStartFailed" class="text-[12px] text-red-500 mb-3">{{ serverFailureReason }}</p>

          <div v-if="serverLogs.length > 0" class="bg-gray-900 rounded-lg p-3 max-h-48 overflow-y-auto dark:bg-zinc-900">
            <div v-for="(log, index) in serverLogs.slice(-20)" :key="index"
              :class="['text-[11px] font-mono leading-relaxed', log.includes('[ERROR]') ? 'text-red-400' : 'text-gray-300 dark:text-zinc-400']">
              {{ log }}
            </div>
          </div>

          <div v-if="serverStartFailed" class="flex gap-2 mt-4">
            <button @click="serverStartFailed = false; serverFailureReason = ''; serverLogs = []"
              class="flex-1 px-4 py-2 rounded-xl text-[13px] font-semibold bg-indigo-600 text-white hover:bg-indigo-700 active:scale-[0.98] transition-all duration-150 cursor-pointer shadow-sm">
              {{ $t("models.close") }}
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
          <span class="text-[14px] font-semibold text-gray-900">{{ $t("models.serverError") }}</span>
        </div>
        <p class="text-[13px] text-gray-600 mb-4">{{ serverError }}</p>
        <button @click="serverError = ''" class="w-full px-4 py-2 rounded-xl text-[13px] font-semibold bg-indigo-600 text-white hover:bg-indigo-700 active:scale-[0.98] transition-all duration-150 cursor-pointer shadow-sm">
          {{ $t("models.ok") }}
        </button>
      </div>
    </div>

    <ConfirmDialog
      :show="showDeleteDialog"
      :title="$t('models.deleteModel')"
      :message="$t('models.deleteModelMsg')"
      @confirm="confirmDelete"
      @cancel="cancelDelete"
    />
  </div>
</template>
