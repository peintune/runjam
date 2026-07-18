<script setup lang="ts">
defineProps<{
  name: string;
  version?: string;
  status: "installed" | "not-installed" | "outdated";
}>();
</script>

<template>
  <div class="flex items-center justify-between bg-white rounded-xl border border-gray-100 px-4 py-3.5">
    <div class="flex items-center gap-3">
      <div
        :class="[
          'w-2 h-2 rounded-full',
          status === 'installed' ? 'bg-emerald-500' : status === 'outdated' ? 'bg-amber-400' : 'bg-gray-300',
        ]"
      />
      <div>
        <p class="text-[14px] font-medium text-gray-900">{{ name }}</p>
        <p v-if="version" class="text-[12px] text-gray-400 mt-0.5">v{{ version }}</p>
      </div>
    </div>
    <span
      :class="[
        'text-[12px] font-medium',
        status === 'installed'
          ? 'text-emerald-600'
          : status === 'outdated'
            ? 'text-amber-600'
            : 'text-gray-400',
      ]"
    >
      {{ status === 'installed' ? 'Installed' : status === 'outdated' ? 'Update available' : 'Not installed' }}
    </span>
  </div>
</template>
