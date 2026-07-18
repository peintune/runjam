<script setup lang="ts">
import { useWorkspaceStore } from "../stores/useWorkspaceStore";
import { Bot } from "lucide-vue-next";

const store = useWorkspaceStore();

const agentTabs = [
  { id: "claude-code" as const, name: "Claude Code", color: "data-[active=true]:text-orange-600 data-[active=true]:border-orange-500" },
  { id: "codex-cli" as const, name: "Codex CLI", color: "data-[active=true]:text-emerald-600 data-[active=true]:border-emerald-500" },
  { id: "gemini-cli" as const, name: "Gemini CLI", color: "data-[active=true]:text-blue-600 data-[active=true]:border-blue-500" },
];

function isActive(id: string) {
  return store.activeSession?.cli === id;
}
</script>

<template>
  <div class="flex-shrink-0 flex items-center gap-1 px-4 py-2 border-b border-gray-100 bg-white">
    <button
      v-for="tab in agentTabs"
      :key="tab.id"
      :data-active="isActive(tab.id)"
      :class="[
        'flex items-center gap-2 px-3 py-1.5 rounded-lg text-[13px] font-medium border-b-2 transition-colors duration-150 cursor-pointer',
        isActive(tab.id)
          ? tab.color + ' border-current -mb-[2px]'
          : 'text-gray-400 border-transparent hover:text-gray-600 hover:bg-gray-50',
      ]"
    >
      <Bot :size="15" />
      {{ tab.name }}
    </button>
  </div>
</template>
