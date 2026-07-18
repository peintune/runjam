<script setup lang="ts">
import { AlertTriangle, X } from "lucide-vue-next";

defineProps<{
  show: boolean;
  title: string;
  message: string;
}>();

const emit = defineEmits<{
  (e: "confirm"): void;
  (e: "cancel"): void;
}>();
</script>

<template>
  <Teleport to="body">
    <div v-if="show" class="fixed inset-0 z-50 flex items-center justify-center p-4">
      <div class="absolute inset-0 bg-black/20 backdrop-blur-[2px] transition-opacity" @click="emit('cancel')"></div>
      <div class="relative w-full max-w-sm bg-white rounded-2xl shadow-2xl border border-gray-100 overflow-hidden animate-in fade-in zoom-in-95 duration-200">
        <div class="absolute top-3 right-3">
          <button @click="emit('cancel')" class="p-1.5 rounded-lg text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors duration-150 cursor-pointer">
            <X :size="16" />
          </button>
        </div>
        
        <div class="px-6 pt-8 pb-4">
          <div class="w-12 h-12 rounded-2xl bg-red-50 flex items-center justify-center mx-auto mb-4">
            <AlertTriangle :size="24" class="text-red-500" />
          </div>
          <h3 class="text-[16px] font-semibold text-gray-900 text-center">{{ title }}</h3>
          <p class="text-[13px] text-gray-500 text-center mt-2 leading-relaxed">{{ message }}</p>
        </div>
        
        <div class="px-6 py-4 bg-gray-50 flex gap-2">
          <button @click="emit('cancel')" class="flex-1 px-4 py-2.5 rounded-xl text-[13px] font-medium text-gray-600 bg-white border border-gray-200 hover:bg-gray-100 transition-all duration-150 cursor-pointer">Cancel</button>
          <button @click="emit('confirm')" class="flex-1 px-4 py-2.5 rounded-xl text-[13px] font-medium text-white bg-red-500 hover:bg-red-600 active:scale-[0.98] transition-all duration-150 shadow-sm cursor-pointer">Delete</button>
        </div>
      </div>
    </div>
  </Teleport>
</template>
