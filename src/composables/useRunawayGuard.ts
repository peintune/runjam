/**
 * Runaway-tool-call detection.
 *
 * A CLI agent can get stuck repeating the SAME failing command forever: if a
 * command errors or returns nothing and the agent does not vary it, its "keep
 * trying" loop has no built-in step limit. Real case: an agent ran
 * `grep -rn "x" .2>/dev/null | grep -v "\."` (a broken command — a missing space
 * turned `. 2>/dev/null` into `.2>`, and `-v "\."` filtered out nearly every
 * file) and repeated it hundreds of times, each run taking minutes.
 *
 * runjam cannot fix the agent's reasoning — it does not run the tools, it only
 * observes them over ACP. What it CAN do is notice the repetition and stop the
 * turn, which is strictly better than burning an hour and the user's context
 * budget on a loop that is not converging.
 *
 * This module holds the pure decision logic so it can be unit-tested; the caller
 * owns the actual abort.
 */

/** How many times one identical call may repeat before the turn is stopped.
 *
 * Set very high (800) at the product owner's request: this guard exists only to
 * catch a loop that will NEVER converge, and erring toward letting a long task
 * run is preferable to interrupting work that is merely slow. The reported
 * runaway repeated one broken command several hundred times, so this still
 * catches it — just later than a tighter limit would. */
export const RUNAWAY_REPEAT_LIMIT = 800;

/** Total tool calls in a single turn before it is treated as runaway.
 *
 * MUST stay above RUNAWAY_REPEAT_LIMIT: otherwise this rule would trip first and
 * the repeat limit would be unreachable, replacing the specific "repeated this
 * command" message with a vaguer one. It is a broad backstop for a turn that
 * never converges through many DIFFERENT calls. */
export const RUNAWAY_TOTAL_LIMIT = 1000;

/**
 * Normalise a tool call into a comparable signature.
 *
 * Whitespace is collapsed because an agent rephrasing only the spacing of the
 * same command is still the same loop; the tool name is included so a `grep` and
 * a `bash` call with identical-looking input are not conflated.
 */
export function toolCallSignature(toolName: string, input: string): string {
  const name = (toolName || "").trim().toLowerCase();
  const body = (input || "").replace(/\s+/g, " ").trim();
  return `${name}::${body}`;
}

/** Per-turn repetition tracker. */
export interface RunawayTracker {
  /** Signature → how many times it has been seen this turn. */
  counts: Map<string, number>;
  /** Total calls seen this turn. */
  total: number;
}

/** Start a fresh tracker (call at the beginning of every turn). */
export function newRunawayTracker(): RunawayTracker {
  return { counts: new Map(), total: 0 };
}

/** Why a turn was judged runaway, or `null` when it is fine. */
export type RunawayReason = "repeated" | "too-many";

export interface RunawayVerdict {
  reason: RunawayReason;
  /** The signature that repeated (only for `reason: "repeated"`). */
  signature: string;
  /** How many times it was seen (only for `reason: "repeated"`). */
  count: number;
  /** Total tool calls this turn (only for `reason: "too-many"`). */
  total: number;
}

/**
 * Record one tool call and decide whether the turn has run away.
 *
 * Returns the verdict on the call that crosses a limit, so the caller stops the
 * turn exactly once; subsequent calls keep reporting until the tracker is reset,
 * but a caller that only stops once is the intended usage.
 */
export function recordToolCall(
  tracker: RunawayTracker,
  toolName: string,
  input: string,
  repeatLimit: number = RUNAWAY_REPEAT_LIMIT,
  totalLimit: number = RUNAWAY_TOTAL_LIMIT,
): RunawayVerdict | null {
  const signature = toolCallSignature(toolName, input);
  const count = (tracker.counts.get(signature) ?? 0) + 1;
  tracker.counts.set(signature, count);
  tracker.total += 1;

  if (count >= repeatLimit) {
    return { reason: "repeated", signature, count, total: tracker.total };
  }
  if (tracker.total >= totalLimit) {
    return { reason: "too-many", signature, count, total: tracker.total };
  }
  return null;
}