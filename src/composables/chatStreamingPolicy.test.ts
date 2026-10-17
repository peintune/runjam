import { describe, expect, it } from "vitest";
import { shouldAnimateStream, streamingSeconds } from "./chatStreamingPolicy";

describe("chat streaming presentation", () => {
  it("shows Q&A chunks immediately, including very short answers", () => {
    expect(shouldAnimateStream({content: "hello"}, "instant-stream")).toBe(false);
  });
  it("keeps normal Agent rendering unchanged", () => {
    expect(shouldAnimateStream({content: "hello"}, "animated")).toBe(true);
    expect(shouldAnimateStream({content: "```code```"}, "animated")).toBe(false);
    expect(shouldAnimateStream({content: "a".repeat(1501)}, "animated")).toBe(false);
  });
  it("never displays a negative duration when a new stream starts after the idle clock tick", () => {
    expect(streamingSeconds(1000, 3000)).toBe(0);
    expect(streamingSeconds(4500, 1000)).toBe(3);
  });
});
