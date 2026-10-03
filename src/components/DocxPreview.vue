<script setup lang="ts">
// Renders a .docx as formatted HTML via mammoth.
//
// mammoth maps Word's semantic styles onto plain HTML elements (Headings, lists,
// tables, bold/italic) instead of trying to reproduce Word's pixel layout. That
// is the right trade-off for a preview: the text and structure are faithful and
// readable, while exact fonts/margins are not — matching what the file *says*
// rather than how a particular printer would render it.
//
// Images embedded in the document are converted by mammoth into data URIs, so no
// asset resolving is needed.
import { ref, watch } from "vue";
import { readFileBytes } from "../api/fs";
import { Loader, FileWarning, ExternalLink } from "lucide-vue-next";
import { openWithSystemApp } from "../api/office";

const props = defineProps<{ filePath: string }>();

const loading = ref(true);
const error = ref("");
const html = ref("");
const fileName = ref("");
const bodyRef = ref<HTMLElement | null>(null);

/** Only the fields used, so the type does not depend on mammoth's own typings. */
type MammothLike = {
  convertToHtml(input: { arrayBuffer: ArrayBuffer }): Promise<{ value: string; messages: unknown[] }>;
};

// Loaded on demand: mammoth is only needed once a .docx is actually opened, and
// pulling it into the initial bundle would slow every launch for a feature most
// sessions never use.
let mammothPromise: Promise<MammothLike | null> | null = null;
function loadMammoth(): Promise<MammothLike | null> {
  if (!mammothPromise) {
    mammothPromise = import("mammoth")
      .then((m) => (m as unknown as MammothLike))
      .catch((e) => {
        console.error("Failed to load mammoth:", e);
        return null;
      });
  }
  return mammothPromise;
}

async function load() {
  if (!props.filePath) return;
  loading.value = true;
  error.value = "";
  html.value = "";
  fileName.value = props.filePath.split(/[/\\]/).pop() || "";
  try {
    const bytes = await readFileBytes(props.filePath);
    const mammoth = await loadMammoth();
    if (!mammoth) throw new Error("Failed to load the .docx renderer");
    const buf = new Uint8Array(bytes).buffer;
    const result = await mammoth.convertToHtml({ arrayBuffer: buf });
    html.value = result.value;
  } catch (err: any) {
    error.value = String(err?.message || err);
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
    <div class="flex-1 min-h-0 overflow-auto bg-[#f8f9fb] dark:bg-[#101015]">
      <div v-if="loading" class="h-full flex items-center justify-center gap-2 text-gray-400">
        <Loader :size="16" class="animate-spin" />
        <span class="text-[13px]">{{ $t("editor.loading") }}</span>
      </div>
      <div v-else-if="error" class="h-full flex flex-col items-center justify-center gap-2 text-gray-400">
        <FileWarning :size="32" class="text-red-300" />
        <span class="text-[13px] text-red-500">{{ error }}</span>
      </div>
      <!-- The generated HTML is presented on a white "page" so it reads like a
           document rather than a web page. -->
      <div class="min-h-full flex justify-center py-6 px-4">
        <div
          ref="bodyRef"
          class="docx-page bg-white w-full max-w-[820px] px-12 py-10 shadow-[0_1px_6px_rgba(0,0,0,0.08)] rounded-sm text-[14px] leading-[1.7] text-[#1e1e2e]"
          v-html="html"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
/* `v-html` content cannot be reached by scoped styles, so these are unscoped in
   effect — kept here, next to the markup they style, via :deep(). */
.docx-page :deep(h1) { font-size: 22px; font-weight: 600; margin: 0.6em 0 0.4em; }
.docx-page :deep(h2) { font-size: 18px; font-weight: 600; margin: 1em 0 0.4em; }
.docx-page :deep(h3) { font-size: 15px; font-weight: 600; margin: 1em 0 0.3em; }
.docx-page :deep(p) { margin: 0.6em 0; }
.docx-page :deep(ul), .docx-page :deep(ol) { margin: 0.6em 0; padding-left: 1.5em; }
.docx-page :deep(ul) { list-style: disc; }
.docx-page :deep(ol) { list-style: decimal; }
.docx-page :deep(li) { margin: 0.2em 0; }
.docx-page :deep(table) { border-collapse: collapse; margin: 0.8em 0; width: 100%; }
.docx-page :deep(th), .docx-page :deep(td) { border: 1px solid #e4e7ed; padding: 6px 10px; text-align: left; }
.docx-page :deep(th) { background: #f8f9fc; font-weight: 600; }
.docx-page :deep(img) { max-width: 100%; height: auto; }
.docx-page :deep(a) { color: #6366f1; text-decoration: underline; }
</style>