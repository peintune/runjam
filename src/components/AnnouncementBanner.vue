<script setup lang="ts">
import type { Announcement } from "../api/telemetry";

defineProps<{ announcement: Announcement }>();
defineEmits<{ close: [] }>();
</script>

<template>
  <div
    class="flex w-80 items-start gap-3 rounded-xl border border-gray-200 bg-white p-4 shadow-lg"
  >
    <div class="min-w-0 flex-1">
      <h3 class="text-[13px] font-semibold text-gray-900">{{ announcement.title }}</h3>
      <p class="mt-1 text-[12px] leading-relaxed text-gray-500 whitespace-pre-line">
        {{ announcement.body }}
      </p>
    </div>
    <button
      class="shrink-0 rounded-md p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600"
      :aria-label="$t('common.close')"
      @click="$emit('close')"
    >
      ✕
    </button>
  </div>
</template>
