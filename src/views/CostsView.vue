<script setup lang="ts">
</script>

<template>
  <div class="flex flex-col h-screen bg-[#f8f9fb]">
    <div data-tauri-drag-region class="flex-shrink-0 h-8 w-full" style="-webkit-app-region: drag" />
    <div class="flex-1 overflow-auto">
      <div class="max-w-3xl mx-auto p-8">
      <h2 class="text-[20px] font-semibold text-gray-900 tracking-tight mb-8">Cost Tracking</h2>

      <!-- summary -->
      <div class="grid grid-cols-3 gap-4 mb-8">
        <div class="bg-white rounded-xl border border-gray-100 p-5">
          <p class="text-[12px] text-gray-500 mb-1">Today</p>
          <p class="text-[24px] font-semibold text-gray-900 tracking-tight">$0.00</p>
          <p class="text-[12px] text-gray-400 mt-1">0 tokens</p>
        </div>
        <div class="bg-white rounded-xl border border-gray-100 p-5">
          <p class="text-[12px] text-gray-500 mb-1">This Week</p>
          <p class="text-[24px] font-semibold text-gray-900 tracking-tight">$0.00</p>
          <p class="text-[12px] text-gray-400 mt-1">0 tokens</p>
        </div>
        <div class="bg-white rounded-xl border border-gray-100 p-5">
          <p class="text-[12px] text-gray-500 mb-1">All Time</p>
          <p class="text-[24px] font-semibold text-gray-900 tracking-tight">$0.00</p>
          <p class="text-[12px] text-gray-400 mt-1">0 tokens</p>
        </div>
      </div>

      <!-- chart -->
      <div class="bg-white rounded-xl border border-gray-100 p-5 mb-8">
        <h3 class="text-[14px] font-medium text-gray-700 mb-5">Daily Cost Trend</h3>
        <div class="h-48 flex items-end gap-1.5">
          <div
            v-for="h in 7"
            :key="h"
            class="flex-1 rounded-md bg-gray-100"
            :style="{ height: (8 + Math.random() * 55) + '%' }"
          />
        </div>
        <div class="flex justify-between mt-2 text-[11px] text-gray-400">
          <span v-for="d in 7" :key="d">Jun {{ 29 + d }}</span>
        </div>
      </div>

      <!-- table -->
      <div class="bg-white rounded-xl border border-gray-100 overflow-hidden">
        <div class="px-5 py-4 border-b border-gray-100">
          <h3 class="text-[14px] font-medium text-gray-700">Session Breakdown</h3>
        </div>
        <div class="p-8 text-center text-[13px] text-gray-400">
          Start a session to see cost data here.
        </div>
      </div>
    </div>
    </div>
  </div>
</template>
