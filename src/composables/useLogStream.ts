import { ref, onUnmounted } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export function useLogStream(sessionId: string) {
  const lines = ref<string[]>([]);
  let unlisten: UnlistenFn | null = null;

  async function start() {
    try {
      unlisten = await listen<{ line: string; timestamp: string }>(
        `session-log:${sessionId}`,
        (event) => {
          lines.value.push(event.payload.line);
        },
      );
    } catch (err) {
      console.error(`Failed to listen for session-log:${sessionId}:`, err);
      // fallback: push a placeholder
      lines.value.push("> (log streaming unavailable in dev mode)");
    }
  }

  function stop() {
    if (unlisten) {
      unlisten();
      unlisten = null;
    }
  }

  onUnmounted(stop);

  return { lines, start, stop };
}
