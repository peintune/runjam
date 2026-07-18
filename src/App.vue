<script setup lang="ts">
import { useAgentStore } from "./stores/useAgentStore";
import { useWorkspaceStore } from "./stores/useWorkspaceStore";
import { getAgentStatuses } from "./api/agents";
import { getModels } from "./api/models";

const agentStore = useAgentStore();
const workspaceStore = useWorkspaceStore();

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
  <router-view />
</template>
