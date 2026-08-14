<script setup lang="ts">
import type { Announcement } from "../api/telemetry";

defineProps<{ announcement: Announcement }>();
defineEmits<{ close: [] }>();
</script>

<template>
  <div class="fixed inset-0 z-[99999] flex items-center justify-center bg-black/40 px-6">
    <div class="w-full max-w-md rounded-2xl bg-white p-6 shadow-2xl">
      <div class="flex items-start justify-between gap-4">
        <h2 class="text-[18px] font-semibold text-gray-900 tracking-tight">
          {{ announcement.title }}
        </h2>
        <button
          class="rounded-md p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600"
          aria-label="关闭"
          @click="$emit('close')"
        >
          ✕
        </button>
      </div>
      <p class="mt-3 text-[13px] leading-relaxed text-gray-500 whitespace-pre-line">
        {{ announcement.body }}
      </p>
      <div class="mt-6 flex items-center justify-end">
        <button
          class="rounded-md bg-blue-600 px-4 py-2 text-[13px] font-medium text-white hover:bg-blue-700 transition-colors"
          @click="$emit('close')"
        >
          知道了
        </button>
      </div>
    </div>
  </div>
</template>
