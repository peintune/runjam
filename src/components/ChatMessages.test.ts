/**
 * Regression guard for the transcript renderer.
 *
 * `ChatMessages` has an `active` prop: `false` means "background session" and
 * renders only a cheap placeholder (the main window keeps several sessions alive
 * at once, and re-rendering every hidden one on each streamed chunk is
 * expensive). Vue's Boolean-prop rule makes an OMITTED Boolean prop `false`, so
 * a caller that forgot to pass it silently got the placeholder — the desktop-pet
 * popup showed an empty transcript for exactly this reason. These tests pin the
 * contract: content renders unless `active` is explicitly false.
 *
 * Rendered with `@vue/server-renderer` so the assertion is about real output
 * rather than a mock. The component is DOM-heavy, so the few browser globals its
 * setup touches are stubbed below (vitest runs these in a node environment).
 */
import { describe, it, expect } from "vitest";

const g = globalThis as unknown as Record<string, unknown>;
g.window = globalThis;
g.addEventListener = () => {};
g.removeEventListener = () => {};
g.localStorage = { getItem: () => null, setItem() {}, removeItem() {} };
g.document = {
  documentElement: { classList: { toggle() {}, add() {}, remove() {} }, dataset: {}, style: {} },
  addEventListener() {},
  removeEventListener() {},
  querySelector: () => null,
  createElement: () => ({ style: {}, classList: { add() {} }, appendChild() {} }),
  body: { appendChild() {}, removeChild() {} },
};
g.requestAnimationFrame = (cb: () => void) => setTimeout(cb, 0);
g.cancelAnimationFrame = (h: unknown) => clearTimeout(h as never);
g.IntersectionObserver = class { observe() {} unobserve() {} disconnect() {} };
g.ResizeObserver = class { observe() {} disconnect() {} };

import { createSSRApp, h } from "vue";
import { renderToString } from "@vue/server-renderer";
import { createPinia } from "pinia";
import ChatMessages from "./ChatMessages.vue";

/** Render the component with the given props and return the HTML. */
async function render(props: Record<string, unknown>): Promise<string> {
  const app = createSSRApp({ render: () => h(ChatMessages as never, props) });
  app.use(createPinia());
  // The main window installs `$t` globally; ChatMessages' template uses it.
  app.config.globalProperties.$t = (key: string) => key;
  return renderToString(app);
}

const messages = [
  { role: "user" as const, content: "HELLO_USER_XYZ" },
  { role: "agent" as const, content: "REPLY_AGENT_ABC" },
];

describe("ChatMessages transcript rendering", () => {
  it("renders both the user's message and the agent's reply", async () => {
    const html = await render({ messages, agentId: "claude-code", active: true });
    expect(html).toContain("HELLO_USER_XYZ");
    expect(html).toContain("REPLY_AGENT_ABC");
  });

  it("renders content when `active` is omitted", async () => {
    // The pet popup passes no `active`; this must NOT fall back to the
    // placeholder (see the module comment for why that happened before).
    const html = await render({ messages, agentId: "claude-code" });
    expect(html).toContain("HELLO_USER_XYZ");
    expect(html).toContain("REPLY_AGENT_ABC");
  });

  it("renders the placeholder only when `active` is explicitly false", async () => {
    const html = await render({ messages, agentId: "claude-code", active: false });
    expect(html).not.toContain("HELLO_USER_XYZ");
  });
});

describe("activity timeline collapsing", () => {
  // A long task stacks many thinking blocks / tool calls inside one bubble. Only
  // the LATEST activity is rendered while collapsed; the rest are revealed by the
  // "show all" button. These tests pin that so a complex answer cannot push the
  // visible text off screen again.
  const stacked = [
    { role: "agent" as const, content: "", isProcessing: true, toolCalls: [{ toolName: "TOOL_EARLY", input: "", status: "completed" }] },
    { role: "agent" as const, content: "", isProcessing: true, toolCalls: [{ toolName: "TOOL_LATEST", input: "", status: "running" }] },
  ];

  it("shows only the latest activity of a multi-step group", async () => {
    const html = await render({ messages: stacked, agentId: "claude-code", active: true });
    expect(html).toContain("TOOL_LATEST");
    expect(html).not.toContain("TOOL_EARLY");
  });

  it("offers a control to reveal the full record", async () => {
    const html = await render({ messages: stacked, agentId: "claude-code", active: true });
    // The i18n key is echoed back by the `$t` stub used in this file.
    expect(html).toContain("chat.toolShowAll");
  });

  it("counts thinking blocks as activity", async () => {
    const html = await render({
      messages: [
        { role: "agent", content: "", thinking: "THINK_EARLY", isProcessing: true },
        ...stacked,
      ],
      agentId: "claude-code",
      active: true,
    });
    expect(html).not.toContain("THINK_EARLY");
    expect(html).toContain("TOOL_LATEST");
  });

  it("never collapses a group with a single activity", async () => {
    const html = await render({
      messages: [{ role: "agent", content: "", toolCalls: [{ toolName: "ONLY_TOOL", input: "", status: "completed" }] }],
      agentId: "claude-code",
      active: true,
    });
    expect(html).toContain("ONLY_TOOL");
    expect(html).not.toContain("chat.toolShowAll");
  });

  it("keeps the answer text visible next to collapsed activity", async () => {
    const html = await render({
      messages: [...stacked, { role: "agent", content: "THE_ANSWER", isProcessing: false }],
      agentId: "claude-code",
      active: true,
    });
    expect(html).toContain("THE_ANSWER");
  });
});

describe("collapsed activity keeps the line count low", () => {
  it("does not auto-expand a thinking body inside a collapsed group", async () => {
    // `shouldAutoExpandThinking` normally opens a thought that has no content
    // yet. While collapsed that must be suppressed, or the thought's full text
    // would render and defeat the one-line goal.
    const html = await render({
      messages: [
        { role: "agent", content: "", thinking: "THINK_BODY_TEXT", isProcessing: true },
        { role: "agent", content: "", isProcessing: true, toolCalls: [{ toolName: "TOOL_LATEST", input: "", status: "running" }] },
      ],
      agentId: "claude-code",
      active: true,
    });
    expect(html).not.toContain("THINK_BODY_TEXT");
  });

  it("still renders a lone thinking body (nothing to collapse)", async () => {
    const html = await render({
      messages: [{ role: "agent", content: "", thinking: "LONE_THOUGHT", isProcessing: true }],
      agentId: "claude-code",
      active: true,
    });
    expect(html).toContain("LONE_THOUGHT");
  });
});
