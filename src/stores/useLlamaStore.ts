import { defineStore } from "pinia";
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import {
  checkLlamaServerAvailable,
  getLlamaServerStatus,
  listLlamaModels,
  getDownloadStatus,
  type LlamaModel,
  type DownloadStatus,
} from "../api/models";

/** Server status shape returned by the Rust `get_server_status` command. */
interface ServerStatus {
  running: boolean;
  port: number;
  model: string | null;
}

/**
 * Global cache for llama.cpp server info. `refresh()` is kicked off at app
 * startup so the local-models settings page renders instantly — probing the
 * server ports can take several seconds on Windows when nothing is running,
 * and blocking the page on that makes it feel broken.
 */
export const useLlamaStore = defineStore("llama", () => {
  const available = ref(false);
  const serverStatus = ref("");
  const models = ref<LlamaModel[]>([]);
  const runningPort = ref(0);
  const runningModel = ref<string | null>(null);
  const downloadStatus = ref<DownloadStatus | null>(null);
  const loading = ref(false);
  const loaded = ref(false);

  let inflight: Promise<void> | null = null;

  async function loadModels(): Promise<void> {
    try {
      models.value = await listLlamaModels();
    } catch {
      models.value = [];
    }
  }

  /** Refresh all llama.cpp info. Concurrent callers share the same request. */
  async function refresh(): Promise<void> {
    if (inflight) return inflight;
    inflight = (async () => {
      loading.value = true;
      try {
        const [isAvailable, status, serverStatusRes, dl] = await Promise.all([
          checkLlamaServerAvailable(),
          getLlamaServerStatus(),
          (async () => {
            await loadModels();
            try {
              return await invoke<ServerStatus>("get_server_status");
            } catch (err) {
              console.log("[DEBUG] get_server_status failed:", err);
              return null;
            }
          })(),
          getDownloadStatus().catch(() => null),
        ]);

        available.value = isAvailable;
        serverStatus.value = status;

        if (serverStatusRes?.running) {
          runningPort.value = serverStatusRes.port;
          runningModel.value = serverStatusRes.model ?? null;
        } else if (status.startsWith("running")) {
          // get_llama_server_status detected a running server on its own.
          const port = parseInt(status.split(":")[1] ?? "", 10);
          runningPort.value = Number.isFinite(port) ? port : 19090;
          runningModel.value = null;
        } else {
          runningPort.value = 0;
          runningModel.value = null;
        }

        if (dl?.downloading) {
          downloadStatus.value = dl;
        }
        loaded.value = true;
      } catch (err) {
        console.error("Failed to load Llama info:", err);
      } finally {
        loading.value = false;
        inflight = null;
      }
    })();
    return inflight;
  }

  return {
    available,
    serverStatus,
    models,
    runningPort,
    runningModel,
    downloadStatus,
    loading,
    loaded,
    refresh,
    loadModels,
  };
});
