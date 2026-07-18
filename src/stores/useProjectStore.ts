import { defineStore } from "pinia";
import { ref } from "vue";

export const useProjectStore = defineStore("project", () => {
  const projects = ref([]);
  const loading = ref(false);

  return { projects, loading };
});
