<script setup lang="ts">
import { computed } from "vue";

const props = defineProps<{ agentId: string; size?: number }>();

const agentLogos = import.meta.glob('/src/assets/agent-logos/*.svg', { eager: true, import: 'default' });

const logoPath = computed(() => {
  const path = `/src/assets/agent-logos/${props.agentId}.svg`;
  return (agentLogos[path] as string) || null;
});

const displaySize = computed(() => props.size || 20);
</script>

<template>
  <img v-if="logoPath" :src="logoPath" :alt="agentId" :width="displaySize" :height="displaySize" class="object-contain" />
  
  <svg v-else :width="displaySize" :height="displaySize" viewBox="0 0 24 24" fill="none">
    <circle cx="12" cy="12" r="11" fill="#6B7280" stroke="#4B5563" stroke-width="1"/>
    <text x="12" y="17" text-anchor="middle" fill="white" font-size="13" font-weight="700" font-family="system-ui">?</text>
  </svg>
</template>