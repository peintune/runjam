import { defineStore } from "pinia";
import { ref } from "vue";

export const useSessionStore = defineStore("session", () => {
  const sessions = ref([]);
  const loading = ref(false);

  return { sessions, loading };
});
