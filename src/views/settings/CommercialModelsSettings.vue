<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed } from "vue";
import { useRoute } from "vue-router";
import { Plus, Trash2, Eye, EyeOff, HelpCircle, Users, Check, ChevronDown, X, RefreshCw } from "lucide-vue-next";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  getModels, saveModels, providers, getProviderById, getProviderByName, maskApiKey,
  getAgentModelMap, assignModelToAgent, removeModelFromAgent,
  type AgentModelInfo, type ProtocolType,
} from "../../api/models";
import { getProviderLogo } from "../../utils/providerIcons";
import { getAgentStatuses } from "../../api/agents";
import type { AgentInfo } from "../../api/agents";
import AgentIcon from "../../components/AgentIcon.vue";
import ConfirmDialog from "../../components/ConfirmDialog.vue";
import { t } from "../../i18n";

const route = useRoute();

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
const agents = ref<AgentInfo[]>([]);
const agentModelMap = ref<Record<string, AgentModelInfo[]>>({});

const showAdd = ref(false);
const newModel = ref({
  provider: providers[0].id,
  name: "",
  alias: "",
  apiBase: providers[0].defaultBase,
  apiKey: "",
});
const selectedProvider = ref(providers[0]);
const showDeleteDialog = ref(false);
const deletingModelId = ref<string | null>(null);
const showAgentDropdown = ref<string | null>(null);
const agentDropdownPosition = ref({ x: 0, y: 0 });
const nameError = ref("");
const apiKeyError = ref("");

const commercialModels = computed(() => models.value.filter(m => m.provider !== "llama"));

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
    await Promise.all([loadAgentModelMap()]);
  } catch {} finally {
    refreshing.value = false;
  }
}

onMounted(() => {
  if (route.query.action === "add") {
    showAdd.value = true;
    nameError.value = "";
    apiKeyError.value = "";
  }

  refreshModels();

  getAgentStatuses().then(list => { agents.value = list; }).catch(() => {});

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

function onProviderChange(id: string) {
  const p = providers.find(x => x.id === id);
  if (!p) return;
  newModel.value.provider = p.id;
  newModel.value.apiBase = p.defaultBase;
  newModel.value.name = "";
  newModel.value.alias = "";
  selectedProvider.value = p;
}

function openProviderHomepage() {
  if (selectedProvider.value?.homepage) {
    openUrl(selectedProvider.value.homepage);
  }
}

async function addModel() {
  nameError.value = "";
  apiKeyError.value = "";

  if (!newModel.value.name.trim()) {
    nameError.value = t("models.modelNameRequired");
    return;
  }

  if (!newModel.value.apiKey.trim()) {
    apiKeyError.value = t("models.apiKeyRequired");
    return;
  }

  models.value.push({
    id: `${newModel.value.provider}-${newModel.value.name.trim()}-${Date.now().toString(36)}`,
    name: newModel.value.name.trim(),
    alias: newModel.value.alias.trim() || newModel.value.name.trim(),
    provider: newModel.value.provider,
    apiBase: newModel.value.apiBase || selectedProvider.value.defaultBase,
    apiKey: newModel.value.apiKey.trim(),
    protocol: selectedProvider.value.protocol,
    showKey: false,
    assignedAgents: [],
    useProxy: {},
  });

  await persistModels();
  showAdd.value = false;
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
  await loadAgentModelMap();
}

function getAgentDisplayName(agentId: string): string {
  const agent = agents.value.find(a => a.id === agentId);
  return agent?.display_name || agentId;
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

  showAgentDropdown.value = showAgentDropdown.value === modelId ? null : modelId;
}
</script>

<template>
  <div class="max-w-3xl mx-auto p-8">
    <div class="mb-6 flex items-center justify-between">
      <div>
        <h2 class="text-[18px] font-semibold text-gray-900 tracking-tight">{{ $t("settings.commercialModels") }}</h2>
        <p class="text-[13px] text-gray-500 mt-0.5">{{ $t("models.subtitle") }}</p>
      </div>
      <div class="flex items-center gap-2">
        <button
          @click="refreshModels"
          :disabled="refreshing"
          class="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-medium bg-gray-50 border border-gray-200 text-gray-600 hover:bg-gray-100 disabled:opacity-50 transition-all duration-150 cursor-pointer active:scale-[0.98]"
        >
          <RefreshCw :size="14" :class="{ 'animate-spin': refreshing }" />
          {{ refreshing ? $t('models.refreshing') : $t('models.refresh') }}
        </button>
        <button @click="showAdd = true; nameError = ''; apiKeyError = ''"
          class="flex items-center gap-1.5 px-4 py-2 rounded-xl text-[13px] font-semibold bg-indigo-600 text-white hover:bg-indigo-700 active:scale-[0.98] transition-all duration-150 shadow-sm cursor-pointer">
          <Plus :size="15" /> {{ $t("models.addModel") }}
        </button>
      </div>
    </div>

    <div v-if="commercialModels.length > 0" class="space-y-2">
      <div class="flex items-center justify-between mb-4">
        <div class="flex items-center gap-2">
          <span class="text-[15px] font-semibold text-gray-900">{{ $t("models.commercialModels") }}</span>
          <span class="text-[12px] text-gray-400">({{ commercialModels.length }})</span>
        </div>
      </div>

      <div v-for="model in commercialModels" :key="model.id"
        class="group bg-white rounded-xl border border-gray-100 shadow-sm hover:shadow-md transition-shadow duration-200 overflow-hidden">
        <div class="flex items-center justify-between px-5 py-2.5 bg-gray-700 border-b border-gray-600 dark:bg-zinc-800 dark:border-zinc-700">
          <div class="flex items-center gap-2">
            <div class="w-6 h-6 rounded-lg flex items-center justify-center overflow-hidden bg-gray-200">
              <img :src="getProviderLogo(getProviderById(model.provider)?.id || getProviderByName(model.provider)?.id || 'custom')" :alt="model.provider" class="w-4 h-4 object-contain" />
            </div>
            <span class="text-[11px] font-medium text-gray-300 dark:text-zinc-400">{{ getProviderById(model.provider)?.name || model.provider }}</span>
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
                {{ model.protocol === 'openai_chat' ? $t('models.protocolChat') : model.protocol === 'openai_responses' ? $t('models.protocolResponses') : model.protocol }}
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
              {{ $t("models.agentsCount", { count: model.assignedAgents.length }) }}
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

    <div v-else class="text-center py-20">
      <div class="w-14 h-14 rounded-2xl bg-gray-100 flex items-center justify-center mx-auto mb-4 text-[24px]">
        ⚡
      </div>
      <h3 class="text-[15px] font-medium text-gray-700 mb-1">{{ $t("models.noModels") }}</h3>
      <p class="text-[13px] text-gray-400 mb-4">{{ $t("models.noModelsDesc") }}</p>
      <button @click="showAdd = true"
        class="px-4 py-2 rounded-xl text-[13px] font-semibold bg-indigo-600 text-white hover:bg-indigo-700 active:scale-[0.98] transition-all duration-150 shadow-sm cursor-pointer">
        {{ $t("models.addFirstModel") }}
      </button>
    </div>

    <Teleport to="body">
      <div v-if="showAgentDropdown" class="fixed z-[60]">
        <div
          class="agent-dropdown-panel bg-white rounded-xl shadow-xl border border-gray-200 p-2 min-w-[280px] max-h-[320px] overflow-y-auto"
          :style="{ left: `${agentDropdownPosition.x}px`, top: `${agentDropdownPosition.y}px`, position: 'fixed', zIndex: 1000 }"
        >
          <div class="flex items-center justify-between px-3 py-2 border-b border-gray-100 mb-2">
            <span class="text-[13px] font-semibold text-gray-900">{{ $t("models.assignToAgents") }}</span>
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
                    ? 'border-gray-700 bg-gray-700 dark:border-zinc-700 dark:bg-zinc-700'
                    : 'border-gray-300'
                ]">
                <Check v-if="models.find(m => m.id === showAgentDropdown)?.assignedAgents.includes(agent.id)" :size="12" class="text-white" />
              </div>
            </div>
            <div v-if="agents.filter(a => a.installed).length === 0" class="text-center py-4">
              <p class="text-[12px] text-gray-400">{{ $t("models.noAgentsInstalled") }}</p>
            </div>
          </div>
        </div>
      </div>
    </Teleport>

    <div v-if="showAdd" class="fixed inset-0 z-50 flex items-center justify-center p-4">
      <div class="absolute inset-0 bg-black/40 backdrop-blur-sm transition-opacity" @click="showAdd = false"></div>
      <div class="relative w-full max-w-lg bg-white rounded-2xl shadow-xl border border-gray-100 overflow-hidden animate-in fade-in zoom-in duration-200">
        <div class="px-6 py-4 border-b border-gray-100 bg-gray-50">
          <h3 class="text-[16px] font-semibold text-gray-900">{{ $t("models.addNewModel") }}</h3>
        </div>
        <div class="p-6 space-y-5 max-h-[70vh] overflow-y-auto">
          <div>
            <label class="block text-[12px] font-medium text-gray-500 mb-2">{{ $t("models.provider") }}</label>
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
            <label class="block text-[12px] font-medium text-gray-500 mb-2">{{ $t("models.alias") }}</label>
            <input v-model="newModel.alias" type="text" :placeholder="$t('models.aliasPlaceholder')"
              class="w-full px-3 py-2 rounded-xl border border-gray-200 bg-white text-[13px] text-gray-900 placeholder-gray-400 outline-none focus:ring-2 focus:ring-gray-600/20 focus:border-gray-400 transition-all" />
          </div>

          <div>
            <label class="flex items-center gap-1 text-[12px] font-medium text-gray-500 mb-2">
              {{ $t("models.modelName") }}
              <button v-if="selectedProvider.homepage" @click="openProviderHomepage"
                class="text-gray-300 hover:text-gray-500 transition-colors cursor-pointer" :title="$t('models.openHomepageModelNames')">
                <HelpCircle :size="13" />
              </button>
            </label>
            <input v-model="newModel.name" type="text" :placeholder="$t('models.modelNamePlaceholder')"
              class="w-full px-3 py-2 rounded-xl border border-gray-200 bg-white text-[13px] text-gray-900 placeholder-gray-400 outline-none focus:ring-2 focus:ring-gray-600/20 focus:border-gray-400 transition-all" />
            <p v-if="nameError" class="text-[12px] text-red-500 mt-1.5">{{ nameError }}</p>
          </div>

          <div>
            <label class="block text-[12px] font-medium text-gray-500 mb-2">{{ $t("models.apiBaseUrl") }}</label>
            <input v-model="newModel.apiBase" type="text" :placeholder="selectedProvider.defaultBase"
              class="w-full px-3 py-2 rounded-xl border border-gray-200 bg-white text-[13px] text-gray-900 placeholder-gray-400 outline-none focus:ring-2 focus:ring-gray-600/20 focus:border-gray-400 transition-all" />
          </div>

          <div>
            <label class="flex items-center gap-1 text-[12px] font-medium text-gray-500 mb-2">
              {{ $t("models.apiKey") }}
              <button v-if="selectedProvider.homepage" @click="openProviderHomepage"
                class="text-gray-300 hover:text-gray-500 transition-colors cursor-pointer" :title="$t('models.openHomepageApiKey')">
                <HelpCircle :size="13" />
              </button>
            </label>
            <input v-model="newModel.apiKey" type="password" placeholder="sk-..."
              class="w-full px-3 py-2 rounded-xl border border-gray-200 bg-white text-[13px] text-gray-900 placeholder-gray-400 outline-none focus:ring-2 focus:ring-gray-600/20 focus:border-gray-400 transition-all" />
            <p v-if="apiKeyError" class="text-[12px] text-red-500 mt-1.5">{{ apiKeyError }}</p>
          </div>
        </div>
        <div class="px-6 py-4 border-t border-gray-100 bg-gray-50 flex gap-2 justify-end">
          <button @click="showAdd = false" class="px-4 py-2 rounded-xl text-[13px] font-medium text-gray-600 bg-gray-50 border border-gray-200 hover:bg-gray-100 active:scale-[0.98] transition-all duration-150 cursor-pointer">{{ $t("models.cancel") }}</button>
          <button @click="addModel" class="px-5 py-2 rounded-xl text-[13px] font-semibold text-white bg-indigo-600 hover:bg-indigo-700 active:scale-[0.98] transition-all duration-150 shadow-sm cursor-pointer">{{ $t("models.addModel") }}</button>
        </div>
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
