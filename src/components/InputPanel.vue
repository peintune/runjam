<script setup lang="ts">
import { ref } from "vue";
import { Send, Paperclip, Folder, X } from "lucide-vue-next";

const inputText = ref("");
const dirPath = ref("");

const emit = defineEmits<{
  (e: "send", text: string): void;
}>();

function handleSend() {
  if (!inputText.value.trim()) return;
  emit("send", inputText.value);
  inputText.value = "";
}
</script>

<template>
  <div class="flex-shrink-0 border-t border-gray-100 bg-white px-4 py-3">
    <div class="rounded-2xl border border-gray-200 bg-[#f8f9fb] focus-within:border-gray-300 focus-within:bg-white focus-within:shadow-sm transition-all duration-150">
      <!-- toolbar -->
      <div class="flex items-center gap-2 px-4 pt-3 pb-1">
        <select class="text-[12px] font-medium text-gray-600 bg-transparent border-none outline-none cursor-pointer appearance-none pr-4 bg-[length:8px_8px] bg-[right_0_center] bg-no-repeat" style="background-image: url('data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 width=%228%22 height=%228%22%3E%3Cpath d=%22M0 2l4 4 4-4%22 fill=%22none%22 stroke=%22%23999%22 stroke-width=%221.5%22/%3E%3C/svg%3E')">
          <option>Claude 3.5 Sonnet</option>
          <option>Claude 3 Opus</option>
          <option>Claude 3 Haiku</option>
        </select>
        <span class="text-gray-300 text-[12px]">·</span>
        <select class="text-[12px] font-medium text-gray-600 bg-transparent border-none outline-none cursor-pointer appearance-none pr-4 bg-[length:8px_8px] bg-[right_0_center] bg-no-repeat" style="background-image: url('data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 width=%228%22 height=%228%22%3E%3Cpath d=%22M0 2l4 4 4-4%22 fill=%22none%22 stroke=%22%23999%22 stroke-width=%221.5%22/%3E%3C/svg%3E')">
          <option>API Key</option>
          <option>OAuth</option>
        </select>
      </div>

      <!-- textarea -->
      <textarea
        v-model="inputText"
        placeholder="Ask anything..."
        rows="2"
        class="w-full px-4 pb-2 bg-transparent border-none outline-none resize-none text-[14px] text-gray-900 placeholder-gray-400 leading-relaxed"
        @keydown.enter.exact.prevent="handleSend"
      />

      <!-- bottom actions -->
      <div class="flex items-center justify-between px-4 pb-3">
        <button
          class="p-1.5 rounded-lg text-gray-400 hover:text-gray-600 hover:bg-gray-200/50 transition-colors duration-150 cursor-pointer"
          title="Attach file"
        >
          <Paperclip :size="15" />
        </button>
        <button
          @click="handleSend"
          :disabled="!inputText.trim()"
          class="p-1.5 rounded-lg transition-colors duration-150"
          :class="inputText.trim() ? 'bg-gray-900 text-white hover:bg-gray-800 cursor-pointer' : 'bg-gray-200 text-gray-400 cursor-not-allowed'"
        >
          <Send :size="15" />
        </button>
      </div>
    </div>

    <!-- directory bar -->
    <div class="flex items-center gap-2 mt-2">
      <button class="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-[12px] text-gray-500 hover:text-gray-700 hover:bg-gray-100 transition-colors duration-150">
        <Folder :size="13" />
        <span v-if="!dirPath" class="text-gray-400">Select project directory...</span>
        <span v-else class="text-gray-600">{{ dirPath.split('/').pop() }}</span>
      </button>
      <button
        v-if="dirPath"
        @click="dirPath = ''"
        class="p-1 rounded-md text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors duration-150"
      >
        <X :size="12" />
      </button>
    </div>
  </div>
</template>
