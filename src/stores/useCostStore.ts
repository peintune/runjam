import { defineStore } from "pinia";
import { ref } from "vue";
import {
  getCostSummary,
  getCostByAgent,
  getCostByDay,
  getCostBySession,
  getCostByDirectory,
  type CostSummary,
  type AgentCost,
  type DailyCost,
  type SessionCost,
  type DirectoryCost,
} from "../api/costs";

export const useCostStore = defineStore("cost", () => {
  const summary = ref<CostSummary | null>(null);
  const byAgent = ref<AgentCost[]>([]);
  const byDay = ref<DailyCost[]>([]);
  const bySession = ref<SessionCost[]>([]);
  const byDirectory = ref<DirectoryCost[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);
  const selectedDays = ref(30);

  async function loadAll() {
    loading.value = true;
    error.value = null;
    try {
      const [s, a, d, se, di] = await Promise.all([
        getCostSummary(),
        getCostByAgent(),
        getCostByDay(selectedDays.value),
        getCostBySession(50),
        getCostByDirectory(),
      ]);
      summary.value = s;
      byAgent.value = a;
      byDay.value = d;
      bySession.value = se;
      byDirectory.value = di;
    } catch (e) {
      error.value = String(e);
      console.error("Failed to load cost data:", e);
    } finally {
      loading.value = false;
    }
  }

  async function setDays(days: number) {
    selectedDays.value = days;
    byDay.value = await getCostByDay(days);
  }

  return { summary, byAgent, byDay, bySession, byDirectory, loading, error, selectedDays, loadAll, setDays };
});
