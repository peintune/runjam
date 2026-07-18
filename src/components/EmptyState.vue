<script setup lang="ts">
import { useWorkspaceStore } from "../stores/useWorkspaceStore";
import { Sparkles } from "lucide-vue-next";

const store = useWorkspaceStore();
</script>

<template>
  <div class="flex-1 flex items-center justify-center">
    <div class="flex flex-col items-center text-center max-w-[340px]">
      <div class="w-14 h-14 rounded-2xl bg-gray-100 flex items-center justify-center mb-5">
        <Sparkles :size="24" class="text-gray-400" />
      </div>
      <h2 class="text-[18px] font-semibold text-gray-900 tracking-tight mb-1.5">
        Welcome to RunJam
      </h2>
      <p class="text-[13px] text-gray-500 leading-relaxed mb-6">
        Select an agent, type a prompt, and press <strong>Enter</strong> to start.
      </p>
      <button
        @click="store.activeSessionId = null"
        class="px-5 py-2.5 rounded-xl text-[13px] font-medium text-white bg-gray-900 hover:bg-gray-800 active:scale-[0.98] transition-all duration-150"
      >
        New Session
      </button>
    </div>
  </div>
</template>
