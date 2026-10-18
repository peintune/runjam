import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRenderer, createSSRApp, h, markRaw, nextTick, ssrContextKey } from "vue";
import { renderToString } from "@vue/server-renderer";
import { createPinia } from "pinia";

const state = vi.hoisted(() => ({
  chatMode: "agent", visible: true, focused: true,
  focusOnNativeRequest: false,
  windowActive: false,
  requiresWindowActivation: false,
  nativeWindowFocus: vi.fn(),
  nativeFocus: vi.fn(),
  listeners: new Map<string, (event: {payload: unknown}) => void>(),
}));

vi.mock("../../composables/usePetChat", async () => {
  const { ref } = await import("vue");
  return {
    usePetChat: () => ({
      messages: ref([]), input: ref(""), busy: ref(false), starting: ref(false),
      agentName: ref("Codex"), modelName: ref("Test model"), canSend: ref(false),
      send: vi.fn(), stop: vi.fn(), newConversation: vi.fn(), openInMainWindow: vi.fn(),
      ensureSession: vi.fn(async () => true), greet: vi.fn(), dispose: vi.fn(),
      config: ref({ chatMode: state.chatMode, agentId: "codex-cli", modelId: "model", directory: "", permissionMode: "approve_for_me", skills: [], reasoning: false }),
      availableAgents: ref([]), availableModels: ref([]), availableSkills: ref([]), recentDirs: ref([]),
      loadOptions: vi.fn(), applyConfig: vi.fn(), toggleSkill: vi.fn(), toggleReasoning: vi.fn(),
      setDirectory: vi.fn(), refreshRecentDirs: vi.fn(), isUnavailableLocalModel: vi.fn(() => false),
      refreshLocalServer: vi.fn(), refreshModels: vi.fn(), openLocalModelsSettings: vi.fn(),
      attachedFiles: ref([]), attachFiles: vi.fn(), removeAttachedFile: vi.fn(),
    }),
  };
});
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async () => false) }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    startDragging: vi.fn(), isVisible: async () => state.visible,
    setFocus: async () => { state.nativeWindowFocus(); state.windowActive = true; },
  }),
}));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ setFocus: async () => {
    state.nativeFocus();
    if (state.focusOnNativeRequest && (!state.requiresWindowActivation || state.windowActive)) state.focused = true;
  } }),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (event: {payload: unknown}) => void) => {
    state.listeners.set(name, handler);
    return () => {state.listeners.delete(name);};
  }),
}));
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

// Run the real setup and lifecycle with an inert renderer. Vitest compiles Vue
// templates for SSR; these tests exercise the composer's focus policy directly.
interface ComposerElement {
  focus: (options?: FocusOptions) => void;
  closest: () => null;
}
interface ComposerSetup {
  inputEl: ComposerElement | null;
  onComposerPointerDown: (event: {target: ComposerElement}) => void;
  cancelFocusRetries: () => void;
}

describe("PetChat keyboard focus", () => {
  let activeElement: ComposerElement | null;
  let events: EventTarget;
  let unmount: (() => void) | undefined;

  async function mount() {
    const renderer = createRenderer({
      createElement: () => ({}), createText: () => ({}), createComment: () => ({}),
      insert: () => {}, remove: () => {}, setText: () => {}, setElementText: () => {},
      parentNode: () => null, nextSibling: () => null, patchProp: () => {},
    });
    const app = renderer.createApp({...PetChat, render: () => null});
    app.use(createPinia());
    app.provide(ssrContextKey, {});
    app.mount({});
    const setup = (app._instance as unknown as {setupState: ComposerSetup}).setupState;
    const textarea: ComposerElement = {
      focus: () => {if (state.focused) activeElement = textarea;},
      closest: () => null,
    };
    setup.inputEl = markRaw(textarea);
    unmount = () => app.unmount();
    await settle();
    return {setup, textarea};
  }

  async function settle() {
    for (let i = 0; i < 12; i++) await nextTick();
  }

  beforeEach(() => {
    vi.useFakeTimers();
    vi.clearAllMocks();
    state.listeners.clear();
    state.chatMode = "qa";
    state.visible = true;
    state.focused = false;
    state.focusOnNativeRequest = false;
    state.windowActive = false;
    state.requiresWindowActivation = false;
    activeElement = null;
    events = new EventTarget();
    vi.stubGlobal("addEventListener", events.addEventListener.bind(events));
    vi.stubGlobal("removeEventListener", events.removeEventListener.bind(events));
    vi.stubGlobal("cancelAnimationFrame", clearTimeout);
    vi.stubGlobal("document", {
      visibilityState: "visible", hasFocus: () => state.focused,
      get activeElement() {return activeElement;},
      addEventListener: () => {}, removeEventListener: () => {},
    });
  });

  afterEach(() => {
    unmount?.();
    unmount = undefined;
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("focuses the composer when native focus arrives after the opening retries", async () => {
    const {textarea} = await mount();
    await vi.advanceTimersByTimeAsync(250);
    expect(activeElement).toBeNull();
    state.focused = true;
    events.dispatchEvent(new Event("focus"));
    await settle();
    expect(activeElement).toBe(textarea);
  });

  it("restores WebView keyboard focus on the first composer click", async () => {
    const {setup, textarea} = await mount();
    await vi.advanceTimersByTimeAsync(250);
    state.focusOnNativeRequest = true;
    setup.onComposerPointerDown({target: textarea});
    await settle();
    expect(activeElement).toBe(textarea);
  });

  it("activates an inactive native panel before restoring its WebView responder", async () => {
    const {setup, textarea} = await mount();
    await vi.advanceTimersByTimeAsync(250);
    vi.clearAllMocks();
    // Reproduced state: the textarea is selected and the WebView is already
    // first responder, but the application has no key window. Refocusing the
    // WebView alone cannot deliver text editing events to an inactive panel.
    activeElement = textarea;
    state.windowActive = false;
    state.focused = false;
    state.requiresWindowActivation = true;
    state.focusOnNativeRequest = true;

    setup.onComposerPointerDown({target: textarea});
    await settle();

    expect(state.nativeWindowFocus).toHaveBeenCalledTimes(1);
    expect(state.nativeFocus).toHaveBeenCalledTimes(1);
    expect(state.nativeWindowFocus.mock.invocationCallOrder[0])
      .toBeLessThan(state.nativeFocus.mock.invocationCallOrder[0]);
    expect(state.focused).toBe(true);
    expect(activeElement).toBe(textarea);
  });

  it("does not steal focus after the user interacts with another control", async () => {
    const {setup} = await mount();
    setup.cancelFocusRetries();
    state.focused = true;
    events.dispatchEvent(new Event("focus"));
    await vi.advanceTimersByTimeAsync(250);
    expect(activeElement).toBeNull();
  });

  it("cancels pending composer focus when the popup is hidden", async () => {
    await mount();
    state.listeners.get("pet:closed")?.({payload: undefined});
    state.focused = true;
    events.dispatchEvent(new Event("focus"));
    await vi.advanceTimersByTimeAsync(250);
    expect(activeElement).toBeNull();
  });

  it("does not request native keyboard focus while mounting hidden", async () => {
    state.visible = false;
    await mount();
    await vi.advanceTimersByTimeAsync(250);
    expect(state.nativeFocus).not.toHaveBeenCalled();
    expect(state.nativeWindowFocus).not.toHaveBeenCalled();
    expect(activeElement).toBeNull();
  });
});
