import { describe, it, expect } from "vitest";
import { ref } from "vue";
import { useContextSize, computeContextStats, CHARS_PER_TOKEN, FALLBACK_MAX_CHARS } from "./useContextSize";

const msg = (content: string) => ({ content });

describe("useContextSize", () => {
  it("counts only the message body text and the input text", () => {
    const messages = ref([msg("hello"), msg("world!")]);
    const input = ref("drafting...");
    const ctx = useContextSize(messages, input, ref(50_000));
    expect(ctx.totalChars.value).toBe(5 + 6 + 11);
  });

  it("uses model context_window × 4 as the max when provided", () => {
    const messages = ref([]);
    const input = ref("");
    const ctx = useContextSize(messages, input, ref(32_000));
    expect(ctx.maxChars.value).toBe(128_000);
  });

  it("falls back to 200_000 chars when no model context_window is available", () => {
    const ctx = useContextSize(ref([]), ref(""), ref(undefined));
    expect(ctx.maxChars.value).toBe(200_000);
  });

  it("returns red ring color when over limit", () => {
    // modelContextWindow=10 → maxChars=40; 500 chars blows the cap.
    const messages = ref([msg("a".repeat(500))]);
    const input = ref("");
    const ctx = useContextSize(messages, input, ref(10));
    expect(ctx.ringColor.value).toBe("#ef4444");
  });

  it("returns orange ring color at >=90% ratio", () => {
    // modelContextWindow=25 → maxChars=100; 95/100 = 0.95 ≥ 0.9.
    const messages = ref([msg("a".repeat(95))]);
    const input = ref("");
    const ctx = useContextSize(messages, input, ref(25));
    expect(ctx.ringColor.value).toBe("#f97316");
  });

  it("returns amber ring color at >=70% ratio", () => {
    // modelContextWindow=25 → maxChars=100; 80/100 = 0.80 ≥ 0.7.
    const messages = ref([msg("a".repeat(80))]);
    const input = ref("");
    const ctx = useContextSize(messages, input, ref(25));
    expect(ctx.ringColor.value).toBe("#f59e0b");
  });

  it("returns gray ring color below 70% ratio", () => {
    // modelContextWindow=25 → maxChars=100; 10/100 = 0.10.
    const messages = ref([msg("a".repeat(10))]);
    const input = ref("");
    const ctx = useContextSize(messages, input, ref(25));
    expect(ctx.ringColor.value).toBe("#9ca3af");
  });

  it("clamps fillRatio to [0, 1]", () => {
    // modelContextWindow=10 → maxChars=40; 500 chars → ratio 12.5 → clamps to 1.
    const messages = ref([msg("a".repeat(500))]);
    const input = ref("");
    const ctx = useContextSize(messages, input, ref(10));
    expect(ctx.fillRatio.value).toBe(1);
  });

  it("flags overLimit when total exceeds max", () => {
    // modelContextWindow=10 → maxChars=40; 200 chars > 40.
    const messages = ref([msg("a".repeat(200))]);
    const input = ref("");
    const ctx = useContextSize(messages, input, ref(10));
    expect(ctx.overLimit.value).toBe(true);
  });
});

describe("computeContextStats (pure, for the sidebar)", () => {
  it("matches useContextSize for the same inputs", () => {
    const msgs = [msg("hello"), msg("world!")];
    const draft = "typing...";
    const window = 50_000;
    const stats = computeContextStats(msgs, draft, window);
    const ctx = useContextSize(ref(msgs), ref(draft), ref(window));
    expect(stats.totalChars).toBe(ctx.totalChars.value);
    expect(stats.maxChars).toBe(ctx.maxChars.value);
    expect(stats.fillRatio).toBe(ctx.fillRatio.value);
    expect(stats.overLimit).toBe(ctx.overLimit.value);
    expect(stats.ringColor).toBe(ctx.ringColor.value);
  });

  it("uses CHARS_PER_TOKEN for the max when window provided", () => {
    expect(computeContextStats([], "", 32_000).maxChars).toBe(32_000 * CHARS_PER_TOKEN);
  });

  it("falls back to FALLBACK_MAX_CHARS when window undefined", () => {
    expect(computeContextStats([], "", undefined).maxChars).toBe(FALLBACK_MAX_CHARS);
  });

  it("handles empty messages and empty draft", () => {
    const stats = computeContextStats([], "", 20_000);
    expect(stats.totalChars).toBe(0);
    expect(stats.overLimit).toBe(false);
    expect(stats.ringColor).toBe("#9ca3af");
  });
});
