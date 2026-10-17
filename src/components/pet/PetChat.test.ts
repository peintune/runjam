import { beforeEach, describe, expect, it, vi } from "vitest";
import { createSSRApp, h } from "vue";
import { renderToString } from "@vue/server-renderer";
import { createPinia } from "pinia";

const state = vi.hoisted(() => ({chatMode: "agent"}));

vi.mock("../../composables/usePetChat", async () => {
  const { ref } = await import("vue");
  return {
    usePetChat: () => ({
      messages: ref([]), input: ref(""), busy: ref(false), starting: ref(false),
      agentName: ref("Codex"), modelName: ref("Test model"), canSend: ref(false),
      send: vi.fn(), stop: vi.fn(), newConversation: vi.fn(), openInMainWindow: vi.fn(),
      ensureSession: vi.fn(), greet: vi.fn(), dispose: vi.fn(),
      config: ref({ chatMode: state.chatMode, agentId: "codex-cli", modelId: "model", directory: "", permissionMode: "approve_for_me", skills: [], reasoning: false }),
      availableAgents: ref([]), availableModels: ref([]), availableSkills: ref([]), recentDirs: ref([]),
      loadOptions: vi.fn(), applyConfig: vi.fn(), toggleSkill: vi.fn(), toggleReasoning: vi.fn(),
      setDirectory: vi.fn(), refreshRecentDirs: vi.fn(), isUnavailableLocalModel: vi.fn(() => false),
      refreshLocalServer: vi.fn(), refreshModels: vi.fn(), openLocalModelsSettings: vi.fn(),
      attachedFiles: ref([]), attachFiles: vi.fn(), removeAttachedFile: vi.fn(),
    }),
  };
});
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ startDragging: vi.fn() }) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

const g = globalThis as unknown as Record<string, unknown>;
g.window = globalThis;
g.addEventListener = () => {};
g.removeEventListener = () => {};
g.requestAnimationFrame = (cb: FrameRequestCallback) => setTimeout(() => cb(0), 0);

import PetChat from "./PetChat.vue";

async function render(): Promise<string> {
  const app = createSSRApp({ render: () => h(PetChat) });
  app.use(createPinia());
  app.config.globalProperties.$t = (key: string) => key;
  return renderToString(app);
}

describe("PetChat composer", () => {
  beforeEach(() => { vi.clearAllMocks(); state.chatMode = "agent"; });

  it("shows a clear Q&A mode switch and hides Agent-only controls in Q&A", async () => {
    state.chatMode = "qa";
    const html = await render();
    expect(html).toContain('aria-pressed="true"');
    expect(html).toContain('Quick Q&amp;A');
    expect(html).not.toContain('title="Skills"');
    expect(html).not.toContain('title="Permissions"');
  });

  it("keeps Skills immediately to the right of Attachments in the bottom action row", async () => {
    const html = await render();
    const attachment = html.indexOf('title="Attach file"');
    const skills = html.indexOf('title="Skills"');

    expect(attachment).toBeGreaterThan(-1);
    expect(skills).toBeGreaterThan(attachment);
  });

  it("starts with a three-line input area", async () => {
    const html = await render();
    expect(html).toContain('rows="3"');
  });

  it("has an accessible pin toggle which starts off", async () => {
    const html = await render();
    expect(html).toContain('aria-pressed="false"');
    expect(html).toContain('title="Keep on top"');
  });
});
