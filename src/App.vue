<script setup lang="ts">
import { useAgentStore } from "./stores/useAgentStore";
import { useWorkspaceStore } from "./stores/useWorkspaceStore";
import { getAgentStatuses } from "./api/agents";
import { getModels } from "./api/models";
import { useToast } from "./composables/useToast";
import Toast from "./components/Toast.vue";

const agentStore = useAgentStore();
const workspaceStore = useWorkspaceStore();
const { toasts, removeToast } = useToast();

Promise.all([
  (async () => {
    try { agentStore.agents = await getAgentStatuses(); } catch {}
  })(),
  (async () => {
    try { agentStore.models = await getModels(); } catch {}
  })(),
  (async () => {
    try { await workspaceStore.loadSessions(); } catch {}
  })(),
]);
</script>

<template>
  <router-view v-slot="{ Component }">
    <keep-alive include="WorkspaceLayout">
      <component :is="Component" />
    </keep-alive>
  </router-view>
  
  <div class="fixed top-4 right-4 z-[9999] flex flex-col gap-2 w-80">
    <Toast
      v-for="toast in toasts"
      :key="toast.id"
      :toast="toast"
      @remove="removeToast"
    />
  </div>
</template>
