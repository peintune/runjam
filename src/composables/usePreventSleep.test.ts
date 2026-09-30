import { describe, it, expect, beforeEach } from "vitest";
import {
  shouldPreventSleep,
  loadPreventSleepPref,
  savePreventSleepPref,
  PREVENT_SLEEP_KEY,
} from "./usePreventSleep";

function makeLocalStorage() {
  const store = new Map<string, string>();
  return {
    get length() { return store.size; },
    key: (i: number) => Array.from(store.keys())[i] ?? null,
    getItem: (k: string) => (store.has(k) ? (store.get(k) as string) : null),
    setItem: (k: string, v: string) => { store.set(k, String(v)); },
    removeItem: (k: string) => { store.delete(k); },
    clear: () => store.clear(),
  } as unknown as Storage;
}

describe("shouldPreventSleep", () => {
  it("stays off when the user has not opted in", () => {
    // Even with work in flight, the default must never hold the machine awake.
    expect(shouldPreventSleep([{ status: "running" }], false)).toBe(false);
    expect(shouldPreventSleep([{ status: "idle" }], false)).toBe(false);
  });

  it("holds while a session is running or idle", () => {
    expect(shouldPreventSleep([{ status: "running" }], true)).toBe(true);
    // `idle` is the between-turns state of an unfinished task, so it counts.
    expect(shouldPreventSleep([{ status: "idle" }], true)).toBe(true);
    expect(
      shouldPreventSleep([{ status: "stopped" }, { status: "running" }], true),
    ).toBe(true);
  });

  it("releases once nothing is in flight", () => {
    expect(shouldPreventSleep([], true)).toBe(false);
    expect(shouldPreventSleep([{ status: "stopped" }], true)).toBe(false);
    expect(shouldPreventSleep([{ status: "error" }], true)).toBe(false);
  });
});

describe("prevent-sleep preference storage", () => {
  beforeEach(() => {
    (globalThis as unknown as { localStorage: Storage }).localStorage = makeLocalStorage();
  });

  it("defaults to off when nothing was stored", () => {
    expect(loadPreventSleepPref()).toBe(false);
  });

  it("round-trips the preference", () => {
    savePreventSleepPref(true);
    expect(loadPreventSleepPref()).toBe(true);
    expect(localStorage.getItem(PREVENT_SLEEP_KEY)).toBe("1");

    savePreventSleepPref(false);
    expect(loadPreventSleepPref()).toBe(false);
    expect(localStorage.getItem(PREVENT_SLEEP_KEY)).toBe("0");
  });

  it("treats a corrupt stored value as off", () => {
    localStorage.setItem(PREVENT_SLEEP_KEY, "yes");
    expect(loadPreventSleepPref()).toBe(false);
  });
});