<script setup lang="ts">
import { ref, onMounted } from "vue";
import { FolderOpen } from "lucide-vue-next";
import { getDataDir, openDataDir } from "@/api/app";

const dataDir = ref("~/.runjam");

onMounted(async () => {
  try {
    dataDir.value = await getDataDir();
  } catch {
    // keep default
  }
});

async function handleOpen() {
  try {
    await openDataDir();
  } catch (e) {
    console.error("Failed to open data directory:", e);
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
          <div>
            <p class="text-[14px] font-medium text-gray-900">Version</p>
            <p class="text-[12px] text-gray-400 mt-0.5">RunJam release</p>
          </div>
          <span class="text-[13px] text-gray-400">v0.1.0</span>
        </div>
      </div>
    </div>
  </div>
</template>
