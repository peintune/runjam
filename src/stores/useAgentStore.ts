import { defineStore } from "pinia";
import { ref } from "vue";

export const useAgentStore = defineStore("agent", () => {
  const agents = ref<any[]>([]);
  const models = ref<any[]>([]);
  const loading = ref(false);

  return { agents, models, loading };
});
