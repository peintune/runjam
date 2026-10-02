import { describe, it, expect } from "vitest";
import {
  newRunawayTracker,
  recordToolCall,
  toolCallSignature,
  RUNAWAY_REPEAT_LIMIT,
  RUNAWAY_TOTAL_LIMIT,
} from "./useRunawayGuard";

describe("toolCallSignature", () => {
  it("includes the tool name so different tools never collide", () => {
    expect(toolCallSignature("Bash", "ls")).not.toBe(toolCallSignature("Read", "ls"));
  });

  it("ignores whitespace differences", () => {
    // An agent reflowing only the spacing of the same command is still the same
    // loop, so it must not evade detection.
    expect(toolCallSignature("Bash", 'grep -rn "x"  .')).toBe(
      toolCallSignature("Bash", 'grep   -rn "x" .'),
    );
    expect(toolCallSignature("Bash", "grep -rn x .\n")).toBe(
      toolCallSignature("Bash", "  grep -rn x .  "),
    );
  });

  it("is case-insensitive on the tool name", () => {
    expect(toolCallSignature("BASH", "ls")).toBe(toolCallSignature("bash", "ls"));
  });
});

describe("recordToolCall", () => {
  it("stays quiet while calls differ", () => {
    const t = newRunawayTracker();
    for (let i = 0; i < 20; i++) {
      expect(recordToolCall(t, "Bash", `echo ${i}`)).toBeNull();
    }
  });

  it("flags the call that crosses the repeat limit", () => {
    const t = newRunawayTracker();
    // Reproduces the reported loop: one broken grep repeated verbatim.
    const cmd = 'grep -rn "checkout-placeholder" .2>/dev/null | grep -v "\\."';
    let verdict = null;
    for (let i = 1; i <= RUNAWAY_REPEAT_LIMIT; i++) {
      verdict = recordToolCall(t, "Bash", cmd);
      if (i < RUNAWAY_REPEAT_LIMIT) expect(verdict).toBeNull();
    }
    expect(verdict?.reason).toBe("repeated");
    expect(verdict?.count).toBe(RUNAWAY_REPEAT_LIMIT);
  });

  it("does not flag repetition below the limit", () => {
    const t = newRunawayTracker();
    const cmd = "same command";
    for (let i = 1; i < RUNAWAY_REPEAT_LIMIT; i++) {
      expect(recordToolCall(t, "Bash", cmd)).toBeNull();
    }
  });

  it("flags a turn that makes far too many distinct calls", () => {
    const t = newRunawayTracker();
    let verdict = null;
    for (let i = 0; i < RUNAWAY_TOTAL_LIMIT; i++) {
      verdict = recordToolCall(t, "Bash", `step-${i}`);
      if (verdict) break;
    }
    expect(verdict?.reason).toBe("too-many");
    expect(verdict?.total).toBe(RUNAWAY_TOTAL_LIMIT);
  });

  it("counts whitespace-only variants of one command as repeats", () => {
    const t = newRunawayTracker();
    let verdict = null;
    for (let i = 0; i < RUNAWAY_REPEAT_LIMIT; i++) {
      verdict = recordToolCall(t, "Bash", i % 2 ? "grep a b" : "grep  a   b ");
    }
    expect(verdict?.reason).toBe("repeated");
  });

  it("a fresh tracker starts clean (per-turn reset)", () => {
    const t1 = newRunawayTracker();
    for (let i = 0; i < RUNAWAY_REPEAT_LIMIT - 1; i++) recordToolCall(t1, "Bash", "cmd");
    // A new turn must not inherit the previous turn's counts: the same command
    // legitimately appearing in each turn is not a loop.
    const t2 = newRunawayTracker();
    expect(recordToolCall(t2, "Bash", "cmd")).toBeNull();
  });
});
describe("thresholds", () => {
  it("trips only after 800 repeats (product-owner-set threshold)", () => {
    // Pinned deliberately: the owner chose this value, so a change here is a
    // deliberate product decision, not an incidental edit. The guard exists to
    // catch a loop that will NEVER converge — the reported case repeated one
    // broken command several hundred times — so it must never interrupt merely
    // slow (but progressing) work.
    expect(RUNAWAY_REPEAT_LIMIT).toBe(800);
    const t = newRunawayTracker();
    for (let i = 1; i < 800; i++) {
      expect(recordToolCall(t, "Bash", "same")).toBeNull();
    }
    expect(recordToolCall(t, "Bash", "same")?.reason).toBe("repeated");
  });

  it("keeps the total limit above the repeat limit", () => {
    // Otherwise the step-count rule would shadow the more informative
    // "repeated this command" verdict.
    expect(RUNAWAY_TOTAL_LIMIT).toBeGreaterThan(RUNAWAY_REPEAT_LIMIT);
  });
});
