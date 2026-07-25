<script setup lang="ts">
import { ref, onMounted } from "vue";
import { AlertCircle, CheckCircle, X, Info, AlertTriangle } from "lucide-vue-next";
import type { ToastConfig } from "../composables/useToast";

const props = defineProps<{
  toast: ToastConfig;
}>();

const emit = defineEmits<{
  (e: "remove", id: number): void;
}>();

const isVisible = ref(false);

onMounted(() => {
  setTimeout(() => {
    isVisible.value = true;
  }, 10);
  
  const duration = props.toast.duration || 3000;
  setTimeout(() => {
    isVisible.value = false;
    setTimeout(() => {
      emit("remove", props.toast.id);
    }, 300);
  }, duration);
});

function close() {
  isVisible.value = false;
  setTimeout(() => {
    emit("remove", props.toast.id);
  }, 300);
}

const icons = {
  error: AlertCircle,
  success: CheckCircle,
  warning: AlertTriangle,
  info: Info,
};

const styles = {
  error: "bg-red-50 border-red-200 text-red-800",
  success: "bg-emerald-50 border-emerald-200 text-emerald-800",
  warning: "bg-amber-50 border-amber-200 text-amber-800",
  info: "bg-blue-50 border-blue-200 text-blue-800",
};

const iconStyles = {
  error: "text-red-500",
  success: "text-emerald-500",
  warning: "text-amber-500",
  info: "text-blue-500",
};

const Icon = icons[props.toast.type];
</script>

<template>
  <div
    :class="[
      'flex items-center gap-3 px-4 py-3 rounded-xl border shadow-lg backdrop-blur-sm',
      styles[toast.type],
      isVisible ? 'translate-x-0 opacity-100' : 'translate-x-full opacity-0'
    ]"
    style="transition: all 0.3s ease-out;"
  >
    <component :is="Icon" :size="18" :class="iconStyles[toast.type]" />
    <span class="text-[14px] font-medium flex-1">{{ toast.message }}</span>
    <button
      @click="close"
      class="p-1 rounded-lg hover:bg-black/5 transition-colors cursor-pointer"
    >
      <X :size="14" />
    </button>
  </div>
</template>
