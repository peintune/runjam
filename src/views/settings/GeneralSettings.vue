<script setup lang="ts">
import { ref, onMounted } from "vue";
import { FolderOpen } from "lucide-vue-next";
import { getDataDir, openDataDir } from "@/api/app";
import {
  getTelemetryStatus,
  setTelemetryEnabled,
  getProxyConfig,
  setProxyConfig,
  testProxy,
  checkUpdateUi,
} from "@/api/telemetry";
import type { UpdateCheckResult } from "@/api/telemetry";
import UpdatePrompt from "@/components/UpdatePrompt.vue";

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

const checking = ref(false);
const updateResult = ref<UpdateCheckResult | null>(null);
const checkError = ref("");

async function checkForUpdate() {
  checking.value = true;
  checkError.value = "";
  try {
    const res = await checkUpdateUi("0.1.0");
    if (res.updateAvailable) {
      updateResult.value = res;
    } else {
      checkError.value = "已是最新版本";
    }
  } catch (e) {
    checkError.value = String(e);
  } finally {
    checking.value = false;
  }
}
</script>

<template>
  <div class="p-6 flex justify-center">
    <div class="max-w-lg w-full">
      <h2 class="text-[18px] font-semibold text-gray-900 tracking-tight mb-6">General</h2>

      <div class="bg-white rounded-xl border border-gray-100 divide-y divide-gray-100">
        <div class="flex items-center justify-between px-5 py-4">
          <div>
            <p class="text-[14px] font-medium text-gray-900">Appearance</p>
            <p class="text-[12px] text-gray-400 mt-0.5">Light mode only</p>
          </div>
          <span class="text-[13px] text-gray-400">Light</span>
        </div>

        <div class="flex items-center justify-between px-5 py-4">
          <div>
            <p class="text-[14px] font-medium text-gray-900">Data Directory</p>
            <p class="text-[12px] text-gray-400 mt-0.5">Where logs and database are stored</p>
          </div>
          <div class="flex items-center gap-2">
            <span class="text-[13px] text-gray-400 font-mono">{{ dataDir }}</span>
            <button
              class="inline-flex items-center gap-1 px-2.5 py-1 text-[12px] font-medium text-blue-600 bg-blue-50 hover:bg-blue-100 rounded-md transition-colors"
              @click="handleOpen"
            >
              <FolderOpen :size="14" />
              Open
            </button>
          </div>
        </div>

        <div class="flex items-center justify-between px-5 py-4">
          <div class="pr-4">
            <p class="text-[14px] font-medium text-gray-900">Anonymous Usage Data</p>
            <p class="text-[12px] text-gray-400 mt-0.5">
              Help improve RunJam: app version, feature usage and sanitized error logs.
              No code, conversations or IP addresses.
            </p>
          </div>
          <button
            role="switch"
            :aria-checked="telemetryEnabled"
            class="relative h-6 w-11 shrink-0 rounded-full transition-colors disabled:opacity-50"
            :class="telemetryEnabled ? 'bg-blue-600' : 'bg-gray-300'"
            @click="toggleTelemetry"
          >
            <span
              class="absolute top-0.5 h-5 w-5 rounded-full bg-white shadow transition-all"
              :class="telemetryEnabled ? 'left-[22px]' : 'left-0.5'"
            />
          </button>
        </div>

        <div class="px-5 py-4">
          <p class="text-[14px] font-medium text-gray-900">Outbound Proxy</p>
          <p class="text-[12px] text-gray-400 mt-0.5">
            HTTP/SOCKS5 proxy used for telemetry reporting when direct connection fails.
            Leave empty to connect directly.
          </p>
          <div class="mt-3 flex items-center gap-2">
            <input
              v-model="proxyUrl"
              type="text"
              spellcheck="false"
              placeholder="http://127.0.0.1:7890 or socks5://127.0.0.1:1080"
              class="flex-1 h-9 px-3 rounded-lg border text-[13px] text-gray-900 placeholder:text-gray-300 bg-white focus:outline-none transition-colors disabled:opacity-50"
              :class="proxyState === 'error' ? 'border-red-300 focus:border-red-400' : 'border-gray-200 focus:border-blue-400'"
              :disabled="proxyState === 'saving' || proxyState === 'testing'"
              @keyup.enter="handleSaveProxy"
            />
            <button
              class="h-9 px-3 rounded-lg text-[13px] font-medium border transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
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
              {{ proxyState === "testing" ? "Testing…" : "Test" }}
            </button>
            <button
              class="h-9 px-4 rounded-lg text-[13px] font-medium text-white bg-blue-600 hover:bg-blue-700 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              :disabled="proxyState === 'saving' || proxyState === 'testing'"
              @click="handleSaveProxy"
            >
              {{ proxyState === "saving" ? "Saving…" : "Save" }}
            </button>
          </div>
          <p v-if="proxyState === 'saved'" class="mt-2 text-[12px] text-green-600">
            Saved — will apply on the next telemetry flush.
          </p>
          <p v-else-if="proxyState === 'ok'" class="mt-2 text-[12px] text-green-600">
            Proxy connection works.
          </p>
          <p v-else-if="proxyState === 'error'" class="mt-2 text-[12px] text-red-600">
            {{ proxyError }}
          </p>
        </div>

        <div class="flex items-center justify-between px-5 py-4">
          <div>
            <p class="text-[14px] font-medium text-gray-900">Version</p>
            <p class="text-[12px] text-gray-400 mt-0.5">RunJam release</p>
          </div>
          <span class="text-[13px] text-gray-400">v0.1.0</span>
        </div>

        <div class="mt-8 border-t border-gray-100 pt-6">
          <h3 class="text-[14px] font-semibold text-gray-900 mb-3">更新</h3>
          <div class="flex items-center justify-between">
            <div>
              <p class="text-[13px] text-gray-600">检查是否有新版本可用</p>
              <p v-if="checkError" class="mt-1 text-[12px] text-gray-400">{{ checkError }}</p>
            </div>
            <button
              class="rounded-md bg-blue-600 px-4 py-2 text-[13px] font-medium text-white hover:bg-blue-700 transition-colors disabled:opacity-50"
              :disabled="checking"
              @click="checkForUpdate"
            >
              {{ checking ? "检查中…" : "检查更新" }}
            </button>
          </div>
        </div>
      </div>
    </div>
  </div>

  <UpdatePrompt
    v-if="updateResult"
    :result="updateResult"
    @close="updateResult = null"
  />
</template>
