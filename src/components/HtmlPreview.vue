<script setup lang="ts">
// Previews an HTML file by rendering it in a sandboxed iframe.
//
// The page is fed through `srcdoc` rather than being pointed at the file path,
// for two reasons: loading the file URL directly would give the page access to
// RunJam's own origin, and Tauri's asset protocol is not enabled in this app.
//
// SECURITY — the sandbox is deliberately empty of the capabilities a generated
// page does not need, since the agent writes these files:
//   * no `allow-same-origin` — the document gets an opaque origin, so even if it
//     tried, it cannot read RunJam's origin, storage or cookies;
//   * no `allow-top-navigation` / `allow-popups` — it cannot navigate the app
//     window away or open windows;
//   * scripts ARE allowed (`allow-scripts`) — a preview of a page whose JS did
//     not run would misrepresent it, and an opaque origin already contains the
//     blast radius.
// Relative assets (a sibling .css or image) will not resolve under `srcdoc`; that
// is an accepted limitation, and the toolbar's "open with the system app" action
// covers the case where fidelity matters.
import { ref, watch } from "vue";
import { readFileText } from "../api/fs";
import { Loader, FileWarning, ExternalLink } from "lucide-vue-next";
import { openWithSystemApp } from "../api/office";

const props = defineProps<{ filePath: string }>();

const loading = ref(true);
const error = ref("");
const doc = ref("");
const fileName = ref("");

async function load() {
  if (!props.filePath) return;
  loading.value = true;
  error.value = "";
  doc.value = "";
  fileName.value = props.filePath.split(/[/\\]/).pop() || "";
  try {
    doc.value = await readFileText(props.filePath);
  } catch (err: any) {
    error.value = String(err);
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
      <div v-if="loading" class="h-full flex items-center justify-center gap-2 text-gray-400">
        <Loader :size="16" class="animate-spin" />
        <span class="text-[13px]">{{ $t("editor.loading") }}</span>
      </div>
      <div v-else-if="error" class="h-full flex flex-col items-center justify-center gap-2 text-gray-400">
        <FileWarning :size="32" class="text-red-300" />
        <span class="text-[13px] text-red-500">{{ error }}</span>
      </div>
      <!-- The sandbox attributes are the security boundary; see the comment above. -->
      <iframe
        v-else
        :srcdoc="doc"
        class="w-full h-full border-0 bg-white"
        sandbox="allow-scripts allow-forms"
        referrerpolicy="no-referrer"
      />
    </div>
  </div>
</template>