<script setup lang="ts">
import { ref, onMounted } from "vue";
import { FolderOpen } from "lucide-vue-next";
import { getDataDir, openDataDir } from "@/api/app";
import { currentLocale, setLocale, type Locale } from "@/i18n";
import {
  getTelemetryStatus,
  setTelemetryEnabled,
  getProxyConfig,
  setProxyConfig,
  testProxy,
} from "@/api/telemetry";

const locale = ref<Locale>(currentLocale());

function changeLocale(l: Locale) {
  setLocale(l);
  locale.value = l;
}

const dataDir = ref("~/.runjam");
const telemetryEnabled = ref(true);
const proxyUrl = ref("");
const proxyState = ref<"idle" | "saving" | "saved" | "testing" | "ok" | "error">("idle");
const proxyError = ref("");

onMounted(async () => {
  try {
    dataDir.value = await getDataDir();
  } catch {
    // keep default
  }
  try {
    const status = await getTelemetryStatus();
    telemetryEnabled.value = status.enabled;
  } catch {
    // backend without telemetry support
  }
  try {
    proxyUrl.value = await getProxyConfig();
  } catch {
    // proxy support missing
  }
});

async function handleOpen() {
  try {
    await openDataDir();
  } catch (e) {
    console.error("Failed to open data directory:", e);
  }
}

async function toggleTelemetry() {
  const next = !telemetryEnabled.value;
  telemetryEnabled.value = next;
  try {
    await setTelemetryEnabled(next);
  } catch (e) {
    console.error("Failed to update telemetry setting:", e);
    telemetryEnabled.value = !next;
  }
}

async function handleSaveProxy() {
  if (proxyState.value === "saving" || proxyState.value === "testing") return;
  proxyState.value = "saving";
  proxyError.value = "";
  try {
    await setProxyConfig(proxyUrl.value.trim());
    proxyState.value = "saved";
  } catch (e) {
    proxyState.value = "error";
    proxyError.value = String(e);
  }
}

async function handleTestProxy() {
  if (proxyState.value === "saving" || proxyState.value === "testing") return;
  proxyState.value = "testing";
  proxyError.value = "";
  try {
    await testProxy(proxyUrl.value.trim());
    proxyState.value = "ok";
  } catch (e) {
    proxyState.value = "error";
    proxyError.value = String(e);
  }
}
</script>

<template>
  <div class="p-6 flex justify-center">
    <div class="max-w-lg w-full">
      <h2 class="text-[18px] font-semibold text-gray-900 tracking-tight mb-6">{{ $t("settings.general") }}</h2>

      <div class="bg-white rounded-xl border border-gray-100 divide-y divide-gray-100">
        <!-- Language -->
        <div class="flex items-center justify-between px-5 py-4">
          <div>
            <p class="text-[14px] font-medium text-gray-900">{{ $t("common.language") }}</p>
            <p class="text-[12px] text-gray-400 mt-0.5">{{ $t("settings.general.languageDesc") }}</p>
          </div>
          <div class="flex items-center gap-1 bg-gray-100 rounded-lg p-1">
            <button
              class="px-3 py-1.5 text-[12px] font-medium rounded-md transition-all duration-150 cursor-pointer"
              :class="locale === 'en-US' ? 'bg-white shadow-sm text-gray-900' : 'text-gray-500 hover:text-gray-700'"
              @click="changeLocale('en-US')"
            >
              English
            </button>
            <button
              class="px-3 py-1.5 text-[12px] font-medium rounded-md transition-all duration-150 cursor-pointer"
              :class="locale === 'zh-CN' ? 'bg-white shadow-sm text-gray-900' : 'text-gray-500 hover:text-gray-700'"
              @click="changeLocale('zh-CN')"
            >
              中文
            </button>
          </div>
        </div>

        <div class="flex items-center justify-between px-5 py-4">
          <div>
            <p class="text-[14px] font-medium text-gray-900">{{ $t("settings.general.appearance") }}</p>
            <p class="text-[12px] text-gray-400 mt-0.5">{{ $t("settings.general.lightOnly") }}</p>
          </div>
          <span class="text-[13px] text-gray-400">{{ $t("settings.general.light") }}</span>
        </div>

        <div class="flex items-center justify-between px-5 py-4">
          <div>
            <p class="text-[14px] font-medium text-gray-900">{{ $t("settings.general.dataDir") }}</p>
            <p class="text-[12px] text-gray-400 mt-0.5">{{ $t("settings.general.dataDirDesc") }}</p>
          </div>
          <div class="flex items-center gap-2">
            <span class="text-[13px] text-gray-400 font-mono">{{ dataDir }}</span>
            <button
              class="inline-flex items-center gap-1 px-2.5 py-1 text-[12px] font-medium text-blue-600 bg-blue-50 hover:bg-blue-100 active:scale-[0.98] rounded-lg transition-all duration-150 cursor-pointer"
              @click="handleOpen"
            >
              <FolderOpen :size="14" />
              {{ $t("common.open") }}
            </button>
          </div>
        </div>

        <div class="flex items-center justify-between px-5 py-4">
          <div class="pr-4">
            <p class="text-[14px] font-medium text-gray-900">{{ $t("settings.general.telemetry") }}</p>
            <p class="text-[12px] text-gray-400 mt-0.5">
              {{ $t("settings.general.telemetryDesc") }}
            </p>
          </div>
          <button
            role="switch"
            :aria-checked="telemetryEnabled"
            class="relative h-6 w-11 shrink-0 rounded-full transition-colors disabled:opacity-50"
            :class="telemetryEnabled ? 'bg-indigo-600' : 'bg-gray-300'"
            @click="toggleTelemetry"
          >
            <span
              class="absolute top-0.5 h-5 w-5 rounded-full bg-white shadow transition-all"
              :class="telemetryEnabled ? 'left-[22px]' : 'left-0.5'"
            />
          </button>
        </div>

        <div class="px-5 py-4">
          <p class="text-[14px] font-medium text-gray-900">{{ $t("settings.general.proxy") }}</p>
          <p class="text-[12px] text-gray-400 mt-0.5">
            {{ $t("settings.general.proxyDesc") }}
          </p>
          <div class="mt-3 flex items-center gap-2">
            <input
              v-model="proxyUrl"
              type="text"
              spellcheck="false"
              :placeholder="$t('settings.general.proxyPlaceholder')"
              class="flex-1 h-9 px-3 rounded-lg border text-[13px] text-gray-900 placeholder:text-gray-300 bg-white focus:outline-none transition-colors disabled:opacity-50"
              :class="proxyState === 'error' ? 'border-red-300 focus:border-red-400' : 'border-gray-200 focus:border-blue-400'"
              :disabled="proxyState === 'saving' || proxyState === 'testing'"
              @keyup.enter="handleSaveProxy"
            />
            <button
              class="h-9 px-3 rounded-lg text-[13px] font-medium border transition-all duration-150 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer active:scale-[0.98]"
              :class="proxyState === 'testing'
                ? 'text-gray-500 border-gray-200 bg-gray-50'
                : proxyState === 'ok'
                  ? 'text-green-600 border-green-200 bg-green-50 hover:bg-green-100'
                  : proxyState === 'error'
                    ? 'text-red-600 border-red-200 bg-red-50 hover:bg-red-100'
                    : 'text-blue-600 border-blue-200 bg-blue-50 hover:bg-blue-100'"
              :disabled="proxyState === 'saving' || proxyState === 'testing'"
              @click="handleTestProxy"
            >
              {{ proxyState === "testing" ? $t("common.testing") : $t("common.test") }}
            </button>
            <button
              class="h-9 px-4 rounded-lg text-[13px] font-medium text-white bg-indigo-600 hover:bg-indigo-700 active:scale-[0.98] transition-all duration-150 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer shadow-sm"
              :disabled="proxyState === 'saving' || proxyState === 'testing'"
              @click="handleSaveProxy"
            >
              {{ proxyState === "saving" ? $t("common.saving") : $t("common.save") }}
            </button>
          </div>
          <p v-if="proxyState === 'saved'" class="mt-2 text-[12px] text-green-600">
            {{ $t("settings.general.proxySaved") }}
          </p>
          <p v-else-if="proxyState === 'ok'" class="mt-2 text-[12px] text-green-600">
            {{ $t("settings.general.proxyOk") }}
          </p>
          <p v-else-if="proxyState === 'error'" class="mt-2 text-[12px] text-red-600">
            {{ proxyError }}
          </p>
        </div>

        <div class="flex items-center justify-between px-5 py-4">
          <div>
            <p class="text-[14px] font-medium text-gray-900">{{ $t("settings.general.version") }}</p>
            <p class="text-[12px] text-gray-400 mt-0.5">{{ $t("settings.general.versionDesc") }}</p>
          </div>
          <span class="text-[13px] text-gray-400">v0.1.0</span>
        </div>
      </div>
    </div>
  </div>
</template>
