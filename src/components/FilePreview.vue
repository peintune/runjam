<script setup lang="ts">
import { ref, watch } from "vue";
import { readFileBytes } from "../api/fs";
import { Loader, FileWarning } from "lucide-vue-next";

const props = defineProps<{
  filePath: string;
}>();

const loading = ref(true);
const error = ref("");
const ext = ref("");
const imgSrc = ref("");
const fileName = ref("");

function getExtension(path: string) {
  return path.split(".").pop()?.toLowerCase() || "";
}

function arrayBufferToBase64(bytes: number[]): string {
  const binary = String.fromCharCode(...bytes);
  return btoa(binary);
}

async function loadFile() {
  if (!props.filePath) return;
  loading.value = true;
  error.value = "";
  imgSrc.value = "";
  ext.value = getExtension(props.filePath);
  fileName.value = props.filePath.split("/").pop() || "";

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
      pdf: "application/pdf",
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
    <div class="flex items-center px-3 py-1.5 border-b border-gray-100 flex-shrink-0 bg-gray-50/50">
      <span class="text-[12px] font-medium text-gray-700 truncate">{{ fileName }}</span>
    </div>

    <!-- preview body -->
    <div class="flex-1 min-h-0 overflow-auto flex items-center justify-center bg-[#f8f9fb]">
      <div v-if="loading" class="flex items-center gap-2 text-gray-400">
        <Loader :size="16" class="animate-spin" />
        <span class="text-[13px]">Loading...</span>
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
      <div v-else class="text-[13px] text-gray-400">
        Cannot preview this file type.
      </div>
    </div>
  </div>
</template>
