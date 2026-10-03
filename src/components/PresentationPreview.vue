<script setup lang="ts">
// Previews a presentation (.pptx/.ppt/.odp) by converting it to PDF first.
//
// Why not render the pptx directly: there is no pure-JS renderer that reproduces
// a deck's layout faithfully, and a preview that mangles the slides is worse than
// no preview. LibreOffice — if the user has it — produces a PDF that the existing
// PDF viewer renders exactly.
//
// LibreOffice is not bundled, so the failure path matters as much as the happy
// one: when it is missing (or the conversion fails) the component offers to open
// the file with the system application instead of showing a dead end. That is why
// the empty state has two distinct messages — "not installed" and "conversion
// failed" need different fixes.
import { ref, watch } from "vue";
import { readFileBytes } from "../api/fs";
import { getDataDir } from "../api/app";
import { convertOfficeToPdf, openWithSystemApp } from "../api/office";
import { previewCacheDir } from "../composables/useFileOpener";
import { Loader, FileWarning, ExternalLink } from "lucide-vue-next";

const props = defineProps<{ filePath: string }>();

const loading = ref(true);
const error = ref("");
const pdfSrc = ref("");
const fileName = ref("");
/** True when the conversion failed because LibreOffice is absent, so the UI can
 *  say so instead of blaming the file. */
const missingConverter = ref(false);

function arrayBufferToBase64(bytes: number[]): string {
  // Chunked to avoid blowing the argument limit of String.fromCharCode on a
  // multi-megabyte PDF — a single spread call throws for large decks.
  const CHUNK = 0x8000;
  let binary = "";
  for (let i = 0; i < bytes.length; i += CHUNK) {
    binary += String.fromCharCode(...bytes.slice(i, i + CHUNK));
  }
  return btoa(binary);
}

async function load() {
  if (!props.filePath) return;
  loading.value = true;
  error.value = "";
  pdfSrc.value = "";
  missingConverter.value = false;
  fileName.value = props.filePath.split(/[/\\]/).pop() || "";
  try {
    const dataDir = await getDataDir();
    const pdfPath = await convertOfficeToPdf(props.filePath, previewCacheDir(dataDir));
    const bytes = await readFileBytes(pdfPath);
    pdfSrc.value = `data:application/pdf;base64,${arrayBufferToBase64(bytes)}`;
  } catch (err: any) {
    const msg = String(err?.message || err);
    // The backend reports the missing converter by name; surface that as its own
    // state so the user is told what to install.
    missingConverter.value = /LibreOffice is not installed/i.test(msg);
    error.value = msg;
  } finally {
    loading.value = false;
  }
}

function openExternal() {
  openWithSystemApp(props.filePath).catch((e) => console.error("Failed to open file:", e));
}

watch(() => props.filePath, load, { immediate: true });
</script>

<template>
  <div class="h-full flex flex-col bg-white">
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
    <div class="flex-1 min-h-0 bg-[#f8f9fb] dark:bg-[#101015]">
      <div v-if="loading" class="h-full flex flex-col items-center justify-center gap-2 text-gray-400">
        <Loader :size="16" class="animate-spin" />
        <span class="text-[13px]">{{ $t("editor.converting") }}</span>
      </div>
      <div v-else-if="error" class="h-full flex flex-col items-center justify-center gap-3 text-gray-400 px-6 text-center">
        <FileWarning :size="32" class="text-red-300" />
        <span class="text-[13px] text-red-500 max-w-[420px]">
          {{ missingConverter ? $t("editor.noOfficeConverter") : error }}
        </span>
        <button
          class="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-medium text-white bg-indigo-600 hover:bg-indigo-700 active:scale-[0.98] transition-all cursor-pointer"
          @click="openExternal"
        >
          <ExternalLink :size="13" />
          {{ $t("editor.openExternal") }}
        </button>
      </div>
      <iframe v-else-if="pdfSrc" :src="pdfSrc" class="w-full h-full border-0" :title="fileName" />
    </div>
  </div>
</template>