<script setup lang="ts">
import { ref, watch } from "vue";
import { readFileBytes } from "../api/fs";
import { Loader, FileWarning, ExternalLink } from "lucide-vue-next";
import { openWithSystemApp } from "../api/office";

const props = defineProps<{
  filePath: string;
}>();

/** The only kinds this component renders. A PDF is deliberately absent — see
 *  `resolveViewer`, which sends PDFs to the system viewer instead. */
const IMAGE_EXT = new Set(["png", "jpg", "jpeg", "gif", "svg", "webp", "ico", "bmp", "avif"]);

const loading = ref(true);
const error = ref("");
const ext = ref("");
const imgSrc = ref("");
const fileName = ref("");

function getExtension(path: string) {
  return path.split(".").pop()?.toLowerCase() || "";
}

function arrayBufferToBase64(bytes: number[]): string {
  // Chunked: a spread over a multi-megabyte array exceeds the argument limit.
  const CHUNK = 0x8000;
  let binary = "";
  for (let i = 0; i < bytes.length; i += CHUNK) {
    binary += String.fromCharCode(...bytes.slice(i, i + CHUNK));
  }
  return btoa(binary);
}

function openExternal() {
  openWithSystemApp(props.filePath).catch((e) => console.error("Failed to open file:", e));
}

async function loadFile() {
  if (!props.filePath) return;
  loading.value = true;
  error.value = "";
  imgSrc.value = "";
  ext.value = getExtension(props.filePath);
  fileName.value = props.filePath.split("/").pop() || "";

  // A format with no in-app viewer (pdf, zip, video) has no bytes worth fetching
  // — show the fallback action immediately rather than base64-encoding a large
  // file only to fail to render it. Images are the only thing this component
  // renders; everything else reaches the `v-else` branch with the system-app
  // button.
  if (!IMAGE_EXT.has(ext.value)) {
    loading.value = false;
    return;
  }

  try {
    const bytes = await readFileBytes(props.filePath);
    const mimeTypes: Record<string, string> = {
      png: "image/png",
      jpg: "image/jpeg",
      jpeg: "image/jpeg",
      gif: "image/gif",
      svg: "image/svg+xml",
      webp: "image/webp",
      ico: "image/x-icon",
      bmp: "image/bmp",
      avif: "image/avif",
    };

    const mime = mimeTypes[ext.value] || "application/octet-stream";
    const base64 = arrayBufferToBase64(bytes);
    imgSrc.value = `data:${mime};base64,${base64}`;
  } catch (err: any) {
    error.value = String(err);
  } finally {
    loading.value = false;
  }
}

watch(() => props.filePath, loadFile, { immediate: true });
</script>

<template>
  <div class="h-full flex flex-col bg-white">
    <!-- toolbar -->
    <div class="flex items-center justify-between px-3 py-1.5 border-b border-gray-100 flex-shrink-0 bg-gray-50/50">
      <span class="text-[12px] font-medium text-gray-700 truncate">{{ fileName }}</span>
      <button
        class="flex items-center gap-1 px-2 py-0.5 rounded-md text-[11px] text-gray-500 hover:text-indigo-600 hover:bg-indigo-50 transition-colors cursor-pointer"
        :title="$t('editor.openInSystemApp')"
        @click="openExternal"
      >
        <ExternalLink :size="12" />
        {{ $t("editor.openExternal") }}
      </button>
    </div>

    <!-- preview body -->
    <div class="flex-1 min-h-0 overflow-auto flex items-center justify-center bg-[#f8f9fb] dark:bg-[#101015]">
      <div v-if="loading" class="flex items-center gap-2 text-gray-400">
        <Loader :size="16" class="animate-spin" />
        <span class="text-[13px]">{{ $t("editor.loading") }}</span>
      </div>
      <div v-else-if="error" class="flex flex-col items-center gap-2 text-gray-400">
        <FileWarning :size="32" class="text-red-300" />
        <span class="text-[13px] text-red-500">{{ error }}</span>
      </div>
      <img
        v-else-if="imgSrc"
        :src="imgSrc"
        :alt="fileName"
        class="max-w-full max-h-full object-contain"
      />
      <!-- No in-app renderer for this type: offer the system app instead of a
           dead end, since the user got here by clicking the file. -->
      <div v-else class="flex flex-col items-center gap-3">
        <FileWarning :size="32" class="text-gray-300" />
        <span class="text-[13px] text-gray-400">{{ $t("editor.cannotPreview") }}</span>
        <button
          class="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-medium text-white bg-indigo-600 hover:bg-indigo-700 active:scale-[0.98] transition-all cursor-pointer"
          @click="openExternal"
        >
          <ExternalLink :size="13" />
          {{ $t("editor.openExternal") }}
        </button>
      </div>
    </div>
  </div>
</template>
