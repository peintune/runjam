import { defineStore } from "pinia";
import { ref } from "vue";

export const useCostStore = defineStore("cost", () => {
  const summary = ref(null);
  const loading = ref(false);

  return { summary, loading };
});
