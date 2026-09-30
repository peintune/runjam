/**
 * Keep the machine awake while an agent is working.
 *
 * When the user enables "prevent sleep while a session is running", a running
 * session holds a system idle-sleep assertion so closing the laptop lid does not
 * freeze the agent mid-task. The assertion is released as soon as no session is
 * running, so an idle app never keeps the machine up.
 *
 * The preference is stored in localStorage so it is known synchronously at boot
 * (before the first `watch` runs).
 */
import { watch, unref } from "vue";
import type { Ref } from "vue";
import { setPreventSleep } from "../api/power";

/**
 * A session list in any of the shapes callers actually hold: a plain array
 * (Pinia unwraps its state refs), a `Ref`, or a getter. Resolved through
 * `unref`/invocation so the composable works from a store, a component ref, or
 * a computed alike.
 */
type SessionsSource =
  | readonly { status: string }[]
  | Ref<readonly { status: string }[]>
  | (() => readonly { status: string }[]);

function readSessions(source: SessionsSource): readonly { status: string }[] {
  if (typeof source === "function" && !("value" in source)) {
    return (source as () => readonly { status: string }[])();
  }
  return unref(source as Ref<readonly { status: string }[]>);
}

/** localStorage key for the user preference. */
export const PREVENT_SLEEP_KEY = "runjam.preventSleepWhileRunning";

/** Read the stored preference (default: off — never surprise the user). */
export function loadPreventSleepPref(): boolean {
  try {
    return localStorage.getItem(PREVENT_SLEEP_KEY) === "1";
  } catch {
    return false;
  }
}

/** Live re-evaluation hooks registered by `usePreventSleep`. */
const _listeners = new Set<() => void>();

/** Persist the preference. */
export function savePreventSleepPref(enabled: boolean): void {
  try {
    localStorage.setItem(PREVENT_SLEEP_KEY, enabled ? "1" : "0");
  } catch {
    // ignore storage errors
  }
  // Nudge live watchers: the preference lives in localStorage (not a reactive
  // ref), so flipping it in Settings would otherwise not re-evaluate the
  // assertion until the session list happened to change.
  _listeners.forEach((fn) => {
    try {
      fn();
    } catch {
      // a listener must never break the caller
    }
  });
}

/**
 * Whether a session counts as "actively working" — i.e. worth keeping the
 * machine awake for.
 *
 * `idle` is included on purpose: that is the state an agent sits in between
 * turns, mid-task. It has not finished, so letting the machine sleep would
 * freeze it exactly like `running` would.
 */
export function shouldPreventSleep(
  sessions: readonly { status: string }[],
  prefEnabled: boolean,
): boolean {
  if (!prefEnabled) return false;
  return sessions.some((s) => s.status === "running" || s.status === "idle");
}

/**
 * Wire the preference + session list to the backend assertion.
 *
 * Returns a stop function that releases the assertion and removes the watcher —
 * call it from `onBeforeUnmount`.
 */
export function usePreventSleep(sessions: SessionsSource): () => void {
  /** Last value pushed to the backend, so we only call it on real changes. */
  let applied: boolean | null = null;

  function apply() {
    // Read the preference fresh each time: the settings page can flip it while
    // this watcher is alive, and a value captured at creation would go stale.
    const want = shouldPreventSleep(readSessions(sessions), loadPreventSleepPref());
    if (want === applied) return;
    applied = want;
    setPreventSleep(want).catch((err) => {
      console.error("[power] failed to set prevent-sleep:", err);
    });
  }

  // Re-evaluate when the session list changes OR when Settings flips the
  // preference (`savePreventSleepPref` pings `_listeners`).
  const stopWatch = watch(
    () => readSessions(sessions),
    apply,
    { deep: true, immediate: true },
  );
  _listeners.add(apply);

  return () => {
    stopWatch();
    _listeners.delete(apply);
    // Never leave an assertion behind on teardown.
    setPreventSleep(false).catch(() => {});
  };
}

/** Re-apply after the settings toggle changes, using the same session list. */
export function setPreventSleepPref(
  sessions: SessionsSource,
  enabled: boolean,
): void {
  savePreventSleepPref(enabled);
  setPreventSleep(shouldPreventSleep(readSessions(sessions), enabled)).catch(() => {});
}