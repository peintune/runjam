import { ref } from "vue";

export interface ToastConfig {
  id: number;
  message: string;
  type: "error" | "success" | "warning" | "info";
  duration?: number;
}

const toasts = ref<ToastConfig[]>([]);
let toastId = 0;

export function useToast() {
  function showToast(message: string, type: ToastConfig["type"] = "info", duration?: number) {
    const id = ++toastId;
    toasts.value.push({
      id,
      message,
      type,
      duration,
    });
    return id;
  }

  function showError(message: string, duration?: number) {
    return showToast(message, "error", duration);
  }

  function showSuccess(message: string, duration?: number) {
    return showToast(message, "success", duration);
  }

  function showWarning(message: string, duration?: number) {
    return showToast(message, "warning", duration);
  }

  function showInfo(message: string, duration?: number) {
    return showToast(message, "info", duration);
  }

  function removeToast(id: number) {
    const index = toasts.value.findIndex((t: ToastConfig) => t.id === id);
    if (index !== -1) {
      toasts.value.splice(index, 1);
    }
  }

  return {
    toasts,
    showToast,
    showError,
    showSuccess,
    showWarning,
    showInfo,
    removeToast,
  };
}
