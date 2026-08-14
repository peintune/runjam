<script setup lang="ts">
import { ref } from "vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import { installUpdate } from "../api/telemetry";
import type { UpdateCheckResult } from "../api/telemetry";

const props = defineProps<{ result: UpdateCheckResult }>();
defineEmits<{ close: [] }>();

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
      error.value = String(e);
      installing.value = false;
    }
  } else if (props.result.downloadUrl) {
    try {
      await openUrl(props.result.downloadUrl);
    } catch {
      error.value = "无法打开下载链接";
    }
  }
}
</script>

<template>
  <div class="fixed inset-0 z-[99999] flex items-center justify-center bg-black/40 px-6">
    <div class="w-full max-w-md rounded-2xl bg-white p-6 shadow-2xl">
      <div class="flex items-start justify-between gap-4">
        <h2 class="text-[18px] font-semibold text-gray-900 tracking-tight">
          发现新版本 {{ result.latestVersion }}
        </h2>
        <button
          class="rounded-md p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600"
          aria-label="关闭"
          @click="$emit('close')"
        >
          ✕
        </button>
      </div>
      <p v-if="result.notes" class="mt-3 text-[13px] leading-relaxed text-gray-500 whitespace-pre-line">
        {{ result.notes }}
      </p>
      <p v-if="error" class="mt-3 text-[12px] text-red-500">{{ error }}</p>
      <div class="mt-6 flex items-center justify-end gap-3">
        <button
          class="rounded-md px-4 py-2 text-[13px] font-medium text-gray-500 hover:text-gray-700 transition-colors"
          :disabled="installing"
          @click="$emit('close')"
        >
          稍后
        </button>
        <button
          class="rounded-md bg-blue-600 px-4 py-2 text-[13px] font-medium text-white hover:bg-blue-700 transition-colors disabled:opacity-50"
          :disabled="installing"
          @click="onPrimaryAction"
        >
          {{ installing ? "正在下载…" : result.action === "install" ? "下载并安装" : "前往下载" }}
        </button>
      </div>
    </div>
  </div>
</template>
