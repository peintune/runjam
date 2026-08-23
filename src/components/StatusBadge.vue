<script setup lang="ts">
defineProps<{
  status: "running" | "idle" | "stopped" | "error" | "completed";
}>();
</script>

<template>
  <span class="inline-flex items-center gap-1.5 text-[12px] font-medium text-gray-600">
    <span
      :class="[
        'w-1.5 h-1.5 rounded-full',
        status === 'running'
          ? 'bg-blue-500'
          : status === 'idle'
            ? 'bg-emerald-400'
            : status === 'error'
              ? 'bg-red-500'
              : status === 'completed'
                ? 'bg-emerald-500'
                : 'bg-gray-300',
      ]"
    />
    {{ status.charAt(0).toUpperCase() + status.slice(1) }}
  </span>
</template>
