<script setup lang="ts">
defineProps<{
  status: "running" | "waiting" | "stopped" | "error" | "completed";
}>();
</script>

<template>
  <span class="inline-flex items-center gap-1.5 text-[12px] font-medium text-gray-600">
    <span
      :class="[
        'w-1.5 h-1.5 rounded-full',
        status === 'running'
          ? 'bg-emerald-500'
          : status === 'waiting'
            ? 'bg-amber-400'
            : status === 'error'
              ? 'bg-red-500'
              : status === 'completed'
                ? 'bg-blue-500'
                : 'bg-gray-300',
      ]"
    />
    {{ status.charAt(0).toUpperCase() + status.slice(1) }}
  </span>
</template>
