<script setup lang="ts">
import { ref, computed, onMounted, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  ArrowLeft, Download, Trash2, ExternalLink, Terminal,
  Loader2, ToggleLeft, ToggleRight, Save,
  Database, Plus, CheckCircle2, XCircle, AlertCircle, RotateCcw, Archive,
} from "lucide-vue-next";
import AgentIcon from "../../components/AgentIcon.vue";
import { useAgentStore } from "../../stores/useAgentStore";
import {
  getAgentStatuses, installAgent, uninstallAgent, setAgentEnabled,
  checkNodejs, getNodejsInstallGuide, readAgentConfig, writeAgentConfig,
  getAgentDirInfo,
  testAgent,
  type AgentInfo, type AgentStatus,
} from "../../api/agents";
import { getModels, getAgentModels, assignModelToAgent, removeModelFromAgent, getNativeModels, restoreNativeConfig, snapshotAgentConfig, getProviderById, getProviderByName, type NativeModel } from "../../api/models";
import { getProviderLogo } from "../../utils/providerIcons";
import { t } from "../../i18n";

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
const configSaveMsg = ref<Record<string, string>>({});

const allModels = ref<any[]>([]);
const agentModels = ref<Record<string, any[]>>({});
const nativeModels = ref<Record<string, NativeModel[]>>({});
const nativeRestoring = ref<Record<string, boolean>>({});
const nativeMsg = ref<Record<string, string>>({});
const addingModel = ref<Record<string, boolean>>({});
const selectedModelId = ref<Record<string, string>>({});

const providerColorMap: Record<string, string> = {
  anthropic: 'border-orange-300 text-orange-700 bg-orange-50',
  openai: 'border-emerald-300 text-emerald-700 bg-emerald-50',
  google: 'border-blue-300 text-blue-700 bg-blue-50',
  gemini: 'border-blue-300 text-blue-700 bg-blue-50',
  deepseek: 'border-violet-300 text-violet-700 bg-violet-50',
  llama: 'border-gray-300 text-gray-700 bg-gray-50',
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

/** The config file RunJam actually edits for an agent.
 *
 * Asked of the backend rather than assembled here: the isolated layout is the
 * backend's business (`agent_isolation` — note the CLI short name, e.g.
 * `<data>/agent-config/claude/settings.json`, not `claude-code`). A
 * hand-written path would send the user looking in the wrong directory when
 * they want to adjust permissions themselves.
 */
const isolatedConfigPath = ref<Record<string, string>>({});

const agentMeta: Record<string, { website: string; installManual: string; configPath: string; description: string }> = {
  // NOTE: `configPath` is the path the CLI uses on its own. RunJam drives each
  // agent with `CLAUDE_CONFIG_DIR` / `CODEX_HOME` / `GEMINI_CLI_HOME` pointed at
  // its own directory (see `agent_isolation`), so this is NOT the file RunJam
  // edits — `isolatedConfigPath()` above is. Never show this one as "the config
  // RunJam manages", or it invites the user to edit a file that a RunJam session
  // will never read.
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
      return { label: t("agents.statusNotInstalled"), color: "text-gray-500", bg: "bg-gray-100", border: "border-gray-200", icon: XCircle };
    case "connection_failed":
      return { label: t("agents.statusConnectionFailed"), color: "text-red-600", bg: "bg-red-50", border: "border-red-200", icon: AlertCircle };
    case "available":
      return { label: t("agents.statusAvailable"), color: "text-emerald-600", bg: "bg-emerald-50", border: "border-emerald-200", icon: CheckCircle2 };
    default:
      return { label: t("agents.statusUnknown"), color: "text-gray-500", bg: "bg-gray-100", border: "border-gray-200", icon: AlertCircle };
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
      await loadNativeModels(agent.value.id);
    }
  } catch (e) {
    console.error("loadAgent error", e);
  }
}

async function loadNativeModels(agentIdStr: string) {
  try {
    nativeModels.value[agentIdStr] = await getNativeModels(agentIdStr);
  } catch (e) {
    nativeModels.value[agentIdStr] = [];
  }
}

async function restoreToNative(agentIdStr: string) {
  nativeRestoring.value[agentIdStr] = true;
  nativeMsg.value[agentIdStr] = "";
  try {
    await restoreNativeConfig(agentIdStr);
    nativeMsg.value[agentIdStr] = t("models.nativeRestored");
    // 还原后该 agent 不再被 runjam 接管，重新加载三方状态
    await Promise.all([loadAgentModels(agentIdStr), loadNativeModels(agentIdStr), loadConfig(agentIdStr)]);
  } catch (e) {
    nativeMsg.value[agentIdStr] = `${t("models.nativeRestoreFailed")}: ${e}`;
  }
  nativeRestoring.value[agentIdStr] = false;
}

async function saveNativeSnapshot(agentIdStr: string) {
  try {
    await snapshotAgentConfig(agentIdStr);
    nativeMsg.value[agentIdStr] = t("models.nativeSnapshotted");
    await loadNativeModels(agentIdStr);
  } catch (e) {
    // 后端用固定标识表达拒绝原因，这里映射成可读文案
    const reason = String(e);
    if (reason.includes("snapshot_exists")) {
      nativeMsg.value[agentIdStr] = t("models.nativeSnapshotExists");
    } else if (reason.includes("config_already_overridden")) {
      nativeMsg.value[agentIdStr] = t("models.nativeSnapshotRefused");
    } else {
      nativeMsg.value[agentIdStr] = `${t("models.nativeRestoreFailed")}: ${e}`;
    }
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
    await assignModelToAgent(agentIdStr, modelId, true);
    await loadAgentModels(agentIdStr);
    await loadNativeModels(agentIdStr);
    await loadConfig(agentIdStr);
    selectedModelId.value[agentIdStr] = '';
  } catch {}
  addingModel.value[agentIdStr] = false;
}

async function removeModelFromAgentUI(agentIdStr: string, modelId: string) {
  try {
    await removeModelFromAgent(agentIdStr, modelId);
    await loadAgentModels(agentIdStr);
    await loadNativeModels(agentIdStr);
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
  // The exact file being edited, from the backend, so the path shown next to
  // the editor is one the user can actually open.
  try {
    const info = await getAgentDirInfo(id);
    const name = id === "codex-cli" ? "config.toml" : "settings.json";
    isolatedConfigPath.value[id] = info.path ? `${info.path}/${name}` : name;
  } catch { /* display-only; the editor still works without it */ }
}

async function saveConfig(id: string) {
  configSaving.value[id] = true;
  try { await writeAgentConfig(id, configContent.value[id] || ''); configDirty.value[id] = false; configSaveMsg.value[id] = t('agent.savedSuccess'); }
  catch (err) { console.error(err); configSaveMsg.value[id] = t('agent.saveFailed'); }
  configSaving.value[id] = false;
  setTimeout(() => { if (configSaveMsg.value[id] === t('agent.savedSuccess') || configSaveMsg.value[id] === t('agent.saveFailed')) configSaveMsg.value[id] = ''; }, 2500);
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
        {{ $t("agent.backToAgents") }}
      </button>
    </div>

    <!-- Loading -->
    <div v-if="!agent" class="flex flex-col items-center justify-center py-20 text-gray-400">
      <Loader2 :size="40" class="mb-3 animate-spin opacity-40" />
      <p class="text-sm">{{ $t("agent.loading") }}</p>
    </div>

    <!-- Main content -->
    <div v-else class="flex-1 overflow-y-auto no-scrollbar px-6 pb-8 space-y-5">

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
              {{ $t("agent.officialWebsite") }}
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
              {{ agent.enabled ? $t('agents.enabled') : $t('agents.disabled') }}
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
              {{ $t("agents.test") }}
            </button>
            <button
              v-if="!agent.installed"
              @click="doInstall(agent.id)"
              :disabled="installing === agent.id"
              class="flex items-center gap-1.5 px-5 py-2 rounded-xl text-[13px] font-semibold bg-indigo-600 text-white hover:bg-indigo-700 active:scale-[0.98] disabled:opacity-50 disabled:cursor-not-allowed transition-all duration-150 cursor-pointer shadow-sm"
            >
              <Loader2 v-if="installing === agent.id" :size="16" class="animate-spin" />
              <Download v-else :size="16" />
              {{ $t("agent.installAgent") }}
            </button>
            <button
              v-if="agent.installed"
              @click="doUninstall(agent.id)"
              :disabled="uninstalling === agent.id"
              class="flex items-center gap-1.5 px-4 py-2 rounded-xl text-[13px] font-medium bg-red-50 border border-red-200 text-red-700 hover:bg-red-100 active:scale-[0.98] disabled:opacity-50 disabled:cursor-not-allowed transition-all duration-150 cursor-pointer"
            >
              <Loader2 v-if="uninstalling === agent.id" :size="16" class="animate-spin" />
              <Trash2 v-else :size="16" />
              {{ $t("agent.uninstall") }}
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
            <h3 class="text-[14px] font-semibold text-gray-800 tracking-tight">{{ $t("agent.modelConfig") }}</h3>
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
              <button
                @click="addModelToAgent(agent.id, selectedModelId[agent.id])"
                :disabled="addingModel[agent.id] || !selectedModelId[agent.id]"
                class="flex items-center gap-1.5 px-5 py-2 rounded-xl text-[13px] font-semibold bg-gray-800 text-white hover:bg-gray-900 active:scale-[0.98] disabled:opacity-30 disabled:cursor-not-allowed transition-all duration-150 ml-auto cursor-pointer shadow-sm dark:bg-zinc-800 dark:hover:bg-zinc-700"
              >
                <Plus :size="14" />
                {{ $t("agent.apply") }}
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

      <!-- ========== NATIVE CONFIG (read-only) ========== -->
      <div class="bg-white rounded-2xl border border-gray-100 shadow-sm overflow-hidden">
        <div class="px-5 py-4 border-b border-gray-100">
          <div class="flex items-center gap-2.5">
            <div class="w-7 h-7 rounded-lg bg-amber-50 flex items-center justify-center">
              <Archive :size="14" class="text-amber-500" />
            </div>
            <h3 class="text-[14px] font-semibold text-gray-800 tracking-tight">{{ $t("models.nativeSection") }}</h3>
            <span
              v-if="(nativeModels[agent.id] || []).some(m => m.is_overridden)"
              class="text-[11px] px-2 py-0.5 rounded-md bg-amber-50 text-amber-700 border border-amber-200"
            >{{ $t("models.nativeOverridden") }}</span>
          </div>
          <p class="text-[12px] text-gray-400 mt-1.5">{{ $t("models.nativeHint") }}</p>
        </div>

        <div class="p-5">
          <!-- 未发现原生配置 -->
          <div v-if="(nativeModels[agent.id] || []).length === 0" class="text-[13px] text-gray-400">
            {{ $t("models.nativeNone") }}
            <p class="text-[12px] text-gray-300 mt-1">{{ $t("models.nativeNoneHint") }}</p>
          </div>

          <!-- 只读列表 -->
          <div v-else class="space-y-2">
            <div
              v-for="nm in nativeModels[agent.id]"
              :key="`${nm.agent_id}-${nm.name}-${nm.protocol}`"
              class="flex items-center justify-between bg-gray-50 rounded-xl border border-gray-100 px-4 py-3"
            >
              <div class="flex items-center gap-3 min-w-0">
                <div class="w-9 h-9 rounded-xl flex items-center justify-center overflow-hidden bg-white border border-gray-200 shadow-sm flex-shrink-0">
                  <img
                    :src="getProviderLogo(getProviderByName(nm.name)?.id || 'custom')"
                    :alt="nm.name"
                    class="w-5 h-5 object-contain"
                  />
                </div>
                <div class="min-w-0">
                  <div class="flex items-center gap-2 flex-wrap">
                    <span class="text-[14px] font-medium text-gray-900 truncate">{{ nm.alias || nm.name }}</span>
                    <span class="text-[11px] px-1.5 py-0.5 rounded bg-amber-50 text-amber-700 border border-amber-200 flex-shrink-0">{{ $t("models.nativeBadge") }}</span>
                    <span v-if="nm.is_current" class="text-[11px] px-1.5 py-0.5 rounded bg-emerald-50 text-emerald-700 border border-emerald-200 flex-shrink-0">{{ $t("models.nativeCurrent") }}</span>
                  </div>
                  <p class="text-[12px] text-gray-400 truncate">
                    {{ nm.name }}
                    <span v-if="nm.api_base" class="font-mono"> · {{ nm.api_base }}</span>
                  </p>
                  <p class="text-[11px] text-gray-300 truncate" :title="nm.source_path">
                    {{ $t("models.nativeSourceFrom") }}: <span class="font-mono">{{ nm.source_path }}</span>
                  </p>
                </div>
              </div>
              <div class="flex items-center gap-2 flex-shrink-0">
                <span
                  v-if="nm.is_overridden"
                  class="text-[11px] px-2 py-0.5 rounded-md bg-gray-100 text-gray-500 border border-gray-200 hidden sm:inline"
                >{{ $t("models.nativeOverridden") }}</span>
              </div>
            </div>

            <!-- 操作 -->
            <p
              v-if="(nativeModels[agent.id] || []).some(m => m.is_overridden) && !(nativeModels[agent.id] || []).some(m => m.has_snapshot)"
              class="text-[12px] text-amber-700 bg-amber-50 border border-amber-200 rounded-lg px-3 py-2"
            >{{ $t("models.nativeNoSnapshotWarn") }}</p>
            <div class="flex items-center gap-3 pt-1">
              <span v-if="nativeMsg[agent.id]" class="text-[12px] text-gray-500 flex-1 truncate">{{ nativeMsg[agent.id] }}</span>
              <div class="flex items-center gap-2 ml-auto">
                <button
                  @click="saveNativeSnapshot(agent.id)"
                  class="flex items-center gap-1.5 px-3 py-2 rounded-xl text-[12px] font-medium bg-gray-100 text-gray-600 hover:bg-gray-200 active:scale-[0.98] transition-all duration-150 cursor-pointer"
                  :title="$t('models.nativeSnapshotHint')"
                >
                  <Archive :size="13" />
                  {{ $t("models.nativeSnapshot") }}
                </button>
                <button
                  @click="restoreToNative(agent.id)"
                  :disabled="nativeRestoring[agent.id]"
                  class="flex items-center gap-1.5 px-4 py-2 rounded-xl text-[12px] font-semibold bg-amber-500 text-white hover:bg-amber-600 active:scale-[0.98] disabled:opacity-40 disabled:cursor-not-allowed transition-all duration-150 cursor-pointer shadow-sm"
                  :title="$t('models.nativeRestoreHint')"
                >
                  <Loader2 v-if="nativeRestoring[agent.id]" :size="13" class="animate-spin" />
                  <RotateCcw v-else :size="13" />
                  {{ nativeRestoring[agent.id] ? $t("models.nativeRestoring") : $t("models.nativeRestore") }}
                </button>
              </div>
            </div>
          </div>
        </div>
      </div>
      <div v-if="installLogs[agent.id]?.length" class="bg-white rounded-2xl border border-gray-100 shadow-sm overflow-hidden">
        <div class="px-5 py-3 border-b border-gray-100">
          <h3 class="text-[14px] font-semibold text-gray-800 tracking-tight">{{ $t("agent.operationLog") }}</h3>
        </div>
        <pre class="bg-gray-50 p-4 text-[12px] text-gray-600 font-mono leading-relaxed max-h-48 overflow-y-auto">{{ installLogs[agent.id]!.join('\n') }}</pre>
      </div>

      <!-- ========== CONFIGURATION FILE ========== -->
      <div class="bg-white rounded-2xl border border-gray-100 shadow-sm overflow-hidden">
        <div class="px-5 py-4 border-b border-gray-100">
          <div class="flex items-center justify-between">
            <div class="flex items-center gap-3">
              <h3 class="text-[14px] font-semibold text-gray-800 tracking-tight">{{ $t("agent.configFile") }}</h3>
              <!-- Show the path actually being edited: RunJam's isolated copy, not
                   the user's `~/.codex` etc. Displaying the user's path here would
                   mislead — edits would appear to change the user's own setup when
                   they only affect RunJam's sessions. -->
              <span
                class="text-[12px] text-gray-400 font-mono bg-gray-100 px-2.5 py-0.5 rounded-lg"
                :title="$t('agent.configIsolatedHint')"
              >{{ isolatedConfigPath[agent.id] }}</span>
            </div>
            <div class="flex items-center gap-3">
              <span class="text-[12px] text-gray-400 hidden sm:inline">{{ $t("agent.configEditHint") }}</span>
              <span
                v-if="configSaveMsg[agent.id]"
                :class="[
                  'text-[12px] font-medium',
                  configSaveMsg[agent.id] === 'Saved successfully' ? 'text-emerald-600' : 'text-red-500'
                ]"
              >{{ configSaveMsg[agent.id] }}</span>
              <button
                v-if="configDirty[agent.id]"
                @click="saveConfig(agent.id)"
                :disabled="configSaving[agent.id]"
                class="flex items-center gap-1.5 px-4 py-1.5 rounded-xl text-[12px] font-semibold text-white bg-indigo-600 hover:bg-indigo-700 active:scale-[0.98] disabled:opacity-50 transition-all duration-150 cursor-pointer shadow-sm"
              >
                <Save :size="13" />
                {{ configSaving[agent.id] ? $t('agent.saving') : $t('agent.saveChanges') }}
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
            :placeholder="$t('agent.configPlaceholder')"
          />
        </div>
      </div>

    </div>
  </div>
</template>
