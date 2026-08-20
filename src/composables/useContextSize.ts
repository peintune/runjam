import { computed, type ComputedRef, type Ref } from "vue";

/** Approximate characters per token, used to convert a model's
 *  `context_window` (in tokens) into a char budget for the UI. */
export const CHARS_PER_TOKEN = 4;

/** Fallback cap when the session's model has no `context_window` set. */
export const FALLBACK_MAX_CHARS = 200_000;

export interface ContextStats {
  totalChars: number;
  maxChars: number;
  fillRatio: number;
  overLimit: boolean;
  ringColor: string;
}

/** Pure, non-reactive version of the context-size computation. Used by the
 *  sidebar (SessionItem) where each row computes its own stats from the
 *  message store + the session's model — no refs needed.
 *
 *  `baseChars` seeds the total with a pre-computed char count (e.g. the
 *  session's `context_chars` persisted in the DB) for sessions whose
 *  messages aren't loaded into the frontend. */
export function computeContextStats(
  messages: Array<{ content?: string }>,
  inputText: string,
  modelContextWindow: number | undefined,
  baseChars = 0,
): ContextStats {
  let total = baseChars || 0;
  for (const m of messages) {
    total += m.content?.length || 0;
  }
  total += inputText?.length || 0;

  const maxChars = modelContextWindow
    ? modelContextWindow * CHARS_PER_TOKEN
    : FALLBACK_MAX_CHARS;

  const fillRatio = Math.min(total / maxChars, 1);
  const overLimit = total > maxChars;

  let ringColor = "#9ca3af"; // gray-400
  if (overLimit) ringColor = "#ef4444";        // red-500
  else if (total / maxChars >= 0.9) ringColor = "#f97316"; // orange-500
  else if (total / maxChars >= 0.7) ringColor = "#f59e0b"; // amber-500

  return { totalChars: total, maxChars, fillRatio, overLimit, ringColor };
}

export interface ContextSize {
  totalChars: ComputedRef<number>;
  maxChars: ComputedRef<number>;
  fillRatio: ComputedRef<number>;
  overLimit: ComputedRef<boolean>;
  ringColor: ComputedRef<string>;
}

/**
 * Compute the context-size indicator for a session. The numerator is the
 * accumulated character count of the conversation's message body text plus
 * the text currently typed in the input box. The denominator is the
 * selected model's `context_window` (in tokens) × CHARS_PER_TOKEN, falling
 * back to FALLBACK_MAX_CHARS when the model has no window configured.
 *
 * Only message body text counts: thinking blocks and tool inputs/outputs
 * are display-only in this app and are not sent to the LLM.
 */
export function useContextSize(
  messages: Ref<Array<{ content?: string }>>,
  inputText: Ref<string>,
  modelContextWindow: Ref<number | undefined>,
): ContextSize {
  const totalChars = computed(() => {
    let total = 0;
    for (const m of messages.value) {
      total += m.content?.length || 0;
    }
    return total + (inputText.value?.length || 0);
  });

  const maxChars = computed(() =>
    modelContextWindow.value
      ? modelContextWindow.value * CHARS_PER_TOKEN
      : FALLBACK_MAX_CHARS,
  );

  const fillRatio = computed(() => Math.min(totalChars.value / maxChars.value, 1));

  const overLimit = computed(() => totalChars.value > maxChars.value);

  const ringColor = computed(() => {
    if (overLimit.value) return "#ef4444"; // red-500
    const r = totalChars.value / maxChars.value;
    if (r >= 0.9) return "#f97316";        // orange-500
    if (r >= 0.7) return "#f59e0b";        // amber-500
    return "#9ca3af";                      // gray-400
  });

  return { totalChars, maxChars, fillRatio, overLimit, ringColor };
}
