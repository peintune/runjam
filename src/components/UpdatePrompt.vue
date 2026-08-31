<script setup lang="ts">
import { ref, computed } from "vue";
import { Github, CloudDownload, ExternalLink } from "lucide-vue-next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { installUpdate } from "../api/telemetry";
import type { UpdateCheckResult } from "../api/telemetry";
import { useMarkdown } from "../composables/useMarkdown";
import { t } from "../i18n";

const props = defineProps<{ result: UpdateCheckResult }>();
defineEmits<{ close: [] }>();

const { render } = useMarkdown();
const notesHtml = computed(() => {
  if (!props.result.notes) return "";
  return render(props.result.notes);
});

// 备用下载源列表：GitHub 官方 + 国内 OSS 镜像
const altSources = computed(() => {
  const urls = props.result.downloadUrls;
  if (!urls) return [];
  const items: { key: string; label: string; icon: string; url: string }[] = [];
  if (urls.github) {
    items.push({ key: "github", label: t("update.githubDownload"), icon: "github", url: urls.github });
  }
  if (urls.cn) {
    items.push({ key: "cn", label: t("update.cnDownload"), icon: "cn", url: urls.cn });
  }
  return items;
});

const installing = ref(false);
const error = ref("");

async function onPrimaryAction() {
  if (props.result.action === "install") {
    installing.value = true;
    error.value = "";
    try {
      await installUpdate();
      // On success the app restarts itself; nothing more to do here.
    } catch (e) {
      // Windows 上自动下载走 GitHub，国内常常失败；若已列出手动下载地址，
      // 提示用户改用下面的源（含国内镜像）。
      error.value = altSources.value.length > 0
        ? `${String(e)} · ${t("update.installFailedUseManual")}`
        : String(e);
      installing.value = false;
    }
  } else if (props.result.downloadUrl) {
    try {
      await openUrl(props.result.downloadUrl);
    } catch {
      error.value = t("update.openDownloadFailed");
    }
  }
}

async function openAltSource(url: string) {
  try {
    await openUrl(url);
  } catch {
    error.value = t("update.openDownloadFailed");
  }
}
</script>

<template>
  <div class="fixed inset-0 z-[99999] flex items-center justify-center bg-black/40 px-6">
    <div class="w-full max-w-md rounded-2xl bg-white p-6 shadow-2xl">
      <div class="flex items-start justify-between gap-4">
        <h2 class="text-[18px] font-semibold text-gray-900 tracking-tight">
          {{ $t("update.newVersion", { version: result.latestVersion ?? "" }) }}
        </h2>
        <button
          class="rounded-md p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600"
          :aria-label="$t('common.close')"
          @click="$emit('close')"
        >
          ✕
        </button>
      </div>
      <div
        v-if="notesHtml"
        class="mt-3 max-h-60 overflow-y-auto text-[13px] leading-relaxed text-gray-600 markdown-body"
        v-html="notesHtml"
      />
      <!-- 备用下载源（GitHub / 国内镜像），手动选择 -->
      <div v-if="altSources.length > 0" class="mt-4 rounded-xl border border-gray-100 bg-gray-50/60 p-3">
        <p class="text-[12px] font-medium text-gray-500 mb-2">{{ $t("update.downloadSource") }}</p>
        <div class="space-y-1.5">
          <button
            v-for="src in altSources"
            :key="src.key"
            class="w-full flex items-center justify-between px-3 py-2 rounded-lg text-[13px] text-gray-700 hover:bg-white hover:text-gray-900 active:scale-[0.99] transition-all duration-150 cursor-pointer border border-transparent hover:border-gray-200"
            @click="openAltSource(src.url)"
          >
            <span class="flex items-center gap-2">
              <Github v-if="src.icon === 'github'" :size="15" class="text-gray-500" />
              <CloudDownload v-else :size="15" class="text-gray-500" />
              {{ src.label }}
            </span>
            <ExternalLink :size="13" class="text-gray-300" />
          </button>
        </div>
      </div>
      <p v-if="error" class="mt-3 text-[12px] text-red-500">{{ error }}</p>
      <div class="mt-6 flex items-center justify-end gap-3">
        <button
          class="rounded-md px-4 py-2 text-[13px] font-medium text-gray-500 hover:text-gray-700 transition-colors"
          :disabled="installing"
          @click="$emit('close')"
        >
          {{ $t("update.later") }}
        </button>
        <button
          class="rounded-md bg-blue-600 px-4 py-2 text-[13px] font-medium text-white hover:bg-blue-700 transition-colors disabled:opacity-50"
          :disabled="installing"
          @click="onPrimaryAction"
        >
          {{ installing ? $t("update.downloading") : result.action === "install" ? $t("update.downloadInstall") : $t("update.goToDownload") }}
        </button>
      </div>
    </div>
  </div>
</template>
