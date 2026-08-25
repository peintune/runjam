<script setup lang="ts">
import { ref, onMounted } from "vue";
import { ExternalLink, Github } from "lucide-vue-next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { checkUpdateUi } from "@/api/telemetry";
import type { UpdateCheckResult } from "@/api/telemetry";
import UpdatePrompt from "@/components/UpdatePrompt.vue";
import { t } from "../../i18n";

const version = ref("v0.1.0");

onMounted(async () => {
  try {
    const { getVersion } = await import("@tauri-apps/api/app");
    version.value = await getVersion();
  } catch {
    // keep default
  }
});

function openLink(url: string) {
  openUrl(url).catch(() => {});
}

const checking = ref(false);
const updateResult = ref<UpdateCheckResult | null>(null);
const checkError = ref("");

async function checkForUpdate() {
  checking.value = true;
  checkError.value = "";
  try {
    const res = await checkUpdateUi(version.value);
    if (res.updateAvailable) {
      updateResult.value = res;
    } else {
      checkError.value = t("about.upToDate");
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
    <div class="max-w-2xl w-full">
      <h2 class="text-[18px] font-semibold text-gray-900 tracking-tight mb-6">{{ $t("about.title") }}</h2>

      <div class="bg-white rounded-xl border border-gray-100 divide-y divide-gray-100">
        <!-- Version & Update -->
        <div class="flex items-center justify-between px-5 py-4">
          <div>
            <p class="text-[14px] font-medium text-gray-900">{{ $t("about.version") }}</p>
            <p class="text-[12px] text-gray-400 mt-0.5">RunJam {{ version }}</p>
          </div>
          <div class="flex items-center gap-2">
            <p v-if="checkError" class="text-[12px] text-gray-400">{{ checkError }}</p>
            <button
              class="rounded-lg bg-indigo-600 px-4 py-2 text-[13px] font-medium text-white hover:bg-indigo-700 active:scale-[0.98] transition-all duration-150 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer shadow-sm"
              :disabled="checking"
              @click="checkForUpdate"
            >
              {{ checking ? $t("about.checking") : $t("about.checkForUpdates") }}
            </button>
          </div>
        </div>

        <!-- Links -->
        <div class="px-5 py-4">
          <p class="text-[14px] font-medium text-gray-900 mb-3">{{ $t("about.links") }}</p>
          <div class="space-y-2">
            <button
              class="w-full flex items-center justify-between px-3 py-2.5 rounded-lg text-[13px] text-gray-600 hover:bg-gray-50 hover:text-gray-900 active:scale-[0.99] transition-all duration-150 cursor-pointer"
              @click="openLink('https://github.com/peintune/runjam')"
            >
              <div class="flex items-center gap-2">
                <Github :size="16" />
                {{ $t("about.github") }}
              </div>
              <ExternalLink :size="14" class="text-gray-300" />
            </button>
            <button
              class="w-full flex items-center justify-between px-3 py-2.5 rounded-lg text-[13px] text-gray-600 hover:bg-gray-50 hover:text-gray-900 active:scale-[0.99] transition-all duration-150 cursor-pointer"
              @click="openLink('https://github.com/peintune/runjam/issues')"
            >
              <div class="flex items-center gap-2">
                <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
                </svg>
                {{ $t("about.reportIssue") }}
              </div>
              <ExternalLink :size="14" class="text-gray-300" />
            </button>
            <button
              class="w-full flex items-center justify-between px-3 py-2.5 rounded-lg text-[13px] text-gray-600 hover:bg-gray-50 hover:text-gray-900 active:scale-[0.99] transition-all duration-150 cursor-pointer"
              @click="openLink('https://x.com/hans_jimmy52900')"
            >
              <div class="flex items-center gap-2">
                <svg class="w-4 h-4" viewBox="0 0 24 24" fill="currentColor">
                  <path d="M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-5.214-6.817L4.99 21.75H1.68l7.73-8.835L1.254 2.25H8.08l4.713 6.231zm-1.161 17.52h1.833L7.084 4.126H5.117z" />
                </svg>
                {{ $t("about.twitter") }}
              </div>
              <ExternalLink :size="14" class="text-gray-300" />
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