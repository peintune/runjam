<script setup lang="ts">
import { ref, computed } from "vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import { installUpdate } from "../api/telemetry";
import type { UpdateCheckResult } from "../api/telemetry";
import { useMarkdown } from "../composables/useMarkdown";

const props = defineProps<{ result: UpdateCheckResult }>();
defineEmits<{ close: [] }>();

const { render } = useMarkdown();
const notesHtml = computed(() => {
  if (!props.result.notes) return "";
  return render(props.result.notes);
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
      error.value = String(e);
      installing.value = false;
    }
  } else if (props.result.downloadUrl) {
    try {
      await openUrl(props.result.downloadUrl);
    } catch {
      error.value = "Failed to open download link";
    }
  }
}
</script>

<template>
  <div class="fixed inset-0 z-[99999] flex items-center justify-center bg-black/40 px-6">
    <div class="w-full max-w-md rounded-2xl bg-white p-6 shadow-2xl">
      <div class="flex items-start justify-between gap-4">
        <h2 class="text-[18px] font-semibold text-gray-900 tracking-tight">
          New version available {{ result.latestVersion }}
        </h2>
        <button
          class="rounded-md p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600"
          aria-label="Close"
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
      <p v-if="error" class="mt-3 text-[12px] text-red-500">{{ error }}</p>
      <div class="mt-6 flex items-center justify-end gap-3">
        <button
          class="rounded-md px-4 py-2 text-[13px] font-medium text-gray-500 hover:text-gray-700 transition-colors"
          :disabled="installing"
          @click="$emit('close')"
        >
          Later
        </button>
        <button
          class="rounded-md bg-blue-600 px-4 py-2 text-[13px] font-medium text-white hover:bg-blue-700 transition-colors disabled:opacity-50"
          :disabled="installing"
          @click="onPrimaryAction"
        >
          {{ installing ? "Downloading…" : result.action === "install" ? "Download and Install" : "Go to Download" }}
        </button>
      </div>
    </div>
  </div>
</template>
