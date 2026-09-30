import { describe, it, expect, beforeEach, vi } from "vitest";

/** Minimal localStorage stub — the composable's session persistence is pure
 *  w.r.t. `window` otherwise, and vitest runs these tests in a node env. */
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

/**
 * `usePetChat` imports the Tauri event/command APIs at module scope. Those
 * touch `window.__TAURI_INTERNALS__` only when a command is actually invoked,
 * so stubbing the modules keeps the pure session-reuse logic testable.
 */
async function freshPetChat() {
  vi.resetModules();
  vi.doMock("@tauri-apps/api/event", () => ({
    listen: vi.fn(async () => () => {}),
    emit: vi.fn(async () => {}),
  }));
  const mod = await import("./usePetChat");
  return mod;
}

describe("isPetSessionFresh", () => {
  beforeEach(() => {
    (globalThis as unknown as { localStorage: Storage }).localStorage = makeLocalStorage();
  });

  it("treats a just-created conversation as fresh", async () => {
    const { isPetSessionFresh } = await freshPetChat();
    const now = 1_700_000_000_000;
    expect(isPetSessionFresh(now, now)).toBe(true);
    expect(isPetSessionFresh(now - 1_000, now)).toBe(true);
  });

  it("keeps a conversation fresh just under the 60-minute idle window", async () => {
    const { isPetSessionFresh, PET_IDLE_TIMEOUT_MS } = await freshPetChat();
    const now = 1_700_000_000_000;
    // One millisecond short of the boundary is still the same conversation.
    expect(isPetSessionFresh(now - (PET_IDLE_TIMEOUT_MS - 1), now)).toBe(true);
    expect(PET_IDLE_TIMEOUT_MS).toBe(60 * 60 * 1000);
  });

  it("expires a conversation exactly at and past the idle window", async () => {
    const { isPetSessionFresh, PET_IDLE_TIMEOUT_MS } = await freshPetChat();
    const now = 1_700_000_000_000;
    // At exactly 60 minutes the next question must start a fresh session.
    expect(isPetSessionFresh(now - PET_IDLE_TIMEOUT_MS, now)).toBe(false);
    expect(isPetSessionFresh(now - PET_IDLE_TIMEOUT_MS * 2, now)).toBe(false);
  });

  it("rejects a negative or non-finite age (clock skew / corrupt storage)", async () => {
    const { isPetSessionFresh } = await freshPetChat();
    const now = 1_700_000_000_000;
    // createdAt in the future → negative age → not fresh.
    expect(isPetSessionFresh(now + 5_000, now)).toBe(false);
    expect(isPetSessionFresh(Number.NaN, now)).toBe(false);
    expect(isPetSessionFresh(now, Number.NaN)).toBe(false);
  });

  it("honors a caller-supplied timeout", async () => {
    const { isPetSessionFresh } = await freshPetChat();
    const now = 1_700_000_000_000;
    expect(isPetSessionFresh(now - 500, now, 1_000)).toBe(true);
    expect(isPetSessionFresh(now - 1_500, now, 1_000)).toBe(false);
  });
});

describe("resolvePetPermissionMode", () => {
  it("falls back to approve_for_me when nothing was saved", async () => {
    const { resolvePetPermissionMode } = await freshPetChat();
    // The popup cannot answer permission prompts, so `ask_approval` (the
    // backend default) would deadlock the turn — never return it by omission.
    expect(resolvePetPermissionMode("")).toBe("approve_for_me");
    expect(resolvePetPermissionMode("   ")).toBe("approve_for_me");
    expect(resolvePetPermissionMode(null)).toBe("approve_for_me");
    expect(resolvePetPermissionMode(undefined)).toBe("approve_for_me");
  });

  it("reuses the agent's saved mode, trimmed", async () => {
    const { resolvePetPermissionMode } = await freshPetChat();
    expect(resolvePetPermissionMode("full_access")).toBe("full_access");
    expect(resolvePetPermissionMode(" read_only ")).toBe("read_only");
    expect(resolvePetPermissionMode("ask_approval")).toBe("ask_approval");
  });
});

describe("mergeStreamedText", () => {
  it("appends deltas (Codex semantics)", async () => {
    const { mergeStreamedText } = await freshPetChat();
    expect(mergeStreamedText("Hel", "lo")).toBe("Hello");
    expect(mergeStreamedText("", "Hi")).toBe("Hi");
  });

  it("replaces with full snapshots (Claude/Gemini semantics)", async () => {
    const { mergeStreamedText } = await freshPetChat();
    // Each chunk restates the whole message; naive appending would double it.
    expect(mergeStreamedText("Hello", "Hello world")).toBe("Hello world");
    expect(mergeStreamedText("Hello world", "Hello world!")).toBe("Hello world!");
  });

  it("detects snapshots despite inconsistent trailing whitespace", async () => {
    const { mergeStreamedText } = await freshPetChat();
    // Claude can send "properly. " then "properly.I'll" — a naive startsWith
    // check would miss it and duplicate the text.
    expect(mergeStreamedText("properly. ", "properly.I'll go")).toBe("properly.I'll go");
  });

  it("treats a non-prefix chunk as an append (new paragraph / tool boundary)", async () => {
    const { mergeStreamedText } = await freshPetChat();
    expect(mergeStreamedText("Done.", "\n\nNext:")).toBe("Done.\n\nNext:");
    // Same length is never a snapshot (not strictly longer).
    expect(mergeStreamedText("abc", "abd")).toBe("abcabd");
  });

  it("ignores empty chunks and empty accumulated text", async () => {
    const { mergeStreamedText } = await freshPetChat();
    expect(mergeStreamedText("Hello", "")).toBe("Hello");
    expect(mergeStreamedText("", "")).toBe("");
  });
});

describe("normalizePetConfig", () => {
  it("returns a default config for missing or non-object input", async () => {
    const { normalizePetConfig, defaultPetConfig } = await freshPetChat();
    expect(normalizePetConfig(null)).toEqual(defaultPetConfig());
    expect(normalizePetConfig(undefined)).toEqual(defaultPetConfig());
    expect(normalizePetConfig("nope")).toEqual(defaultPetConfig());
    expect(normalizePetConfig(42)).toEqual(defaultPetConfig());
  });

  it("keeps well-formed fields intact", async () => {
    const { normalizePetConfig } = await freshPetChat();
    expect(
      normalizePetConfig({
        agentId: "claude-code",
        modelId: "sonnet",
        directory: "/tmp/proj",
        permissionMode: "approve_for_me",
        skills: ["a", "b"],
        noThinking: true,
      }),
    ).toEqual({
      agentId: "claude-code",
      modelId: "sonnet",
      directory: "/tmp/proj",
      permissionMode: "approve_for_me",
      skills: ["a", "b"],
      noThinking: true,
    });
  });

  it("repairs mistyped fields instead of throwing", async () => {
    const { normalizePetConfig } = await freshPetChat();
    const result = normalizePetConfig({
      agentId: 7,
      modelId: null,
      directory: { path: "/x" },
      permissionMode: ["full_access"],
      skills: "claude-code",
      noThinking: "yes",
    });
    expect(result).toEqual({
      agentId: "",
      modelId: "",
      directory: "",
      permissionMode: "",
      skills: [],
      noThinking: false,
    });
  });

  it("drops non-string entries from a skills array", async () => {
    const { normalizePetConfig } = await freshPetChat();
    const result = normalizePetConfig({ skills: ["ok", 3, null, "also-ok", {}] });
    expect(result.skills).toEqual(["ok", "also-ok"]);
  });
});

describe("petConfigKey", () => {
  it("ignores skill order but notices any real change", async () => {
    const { petConfigKey, defaultPetConfig } = await freshPetChat();
    const base = { ...defaultPetConfig(), agentId: "codex-cli", modelId: "gpt-5" };
    expect(petConfigKey({ ...base, skills: ["a", "b"] })).toBe(
      petConfigKey({ ...base, skills: ["b", "a"] }),
    );
    expect(petConfigKey({ ...base, skills: ["a"] })).not.toBe(
      petConfigKey({ ...base, skills: ["a", "b"] }),
    );
  });

  it("changes when any toolbar knob changes", async () => {
    const { petConfigKey, defaultPetConfig } = await freshPetChat();
    const base = defaultPetConfig();
    const key = petConfigKey(base);
    expect(petConfigKey({ ...base, agentId: "claude-code" })).not.toBe(key);
    expect(petConfigKey({ ...base, modelId: "opus" })).not.toBe(key);
    expect(petConfigKey({ ...base, directory: "/tmp/x" })).not.toBe(key);
    expect(petConfigKey({ ...base, permissionMode: "full_access" })).not.toBe(key);
    expect(petConfigKey({ ...base, noThinking: true })).not.toBe(key);
  });
});

describe("isInternalStreamChunk", () => {
  it("filters the ACP session-id marker the popup must never render", async () => {
    const { isInternalStreamChunk, ACP_SESSION_ID_MARKER } = await freshPetChat();
    // Regression: the popup used to append this to the transcript, so every new
    // conversation showed "__ACP_SESSION_ID__<uuid>".
    expect(ACP_SESSION_ID_MARKER).toBe("__ACP_SESSION_ID__");
    expect(isInternalStreamChunk(`${ACP_SESSION_ID_MARKER}abc-123`)).toBe(true);
    expect(isInternalStreamChunk(ACP_SESSION_ID_MARKER)).toBe(true);
  });

  it("passes real model output through", async () => {
    const { isInternalStreamChunk } = await freshPetChat();
    expect(isInternalStreamChunk("Hello!")).toBe(false);
    expect(isInternalStreamChunk("")).toBe(false);
    // Only a PREFIX counts — the marker mid-text is ordinary content.
    expect(isInternalStreamChunk("see __ACP_SESSION_ID__")).toBe(false);
  });
});

describe("pet session idle window (问题: 配置切换不应新建会话)", () => {
  it("measures the idle window from the LAST message, not creation", async () => {
    const { isPetSessionFresh, PET_IDLE_TIMEOUT_MS } = await freshPetChat();
    const now = 1_700_000_000_000;
    // A conversation created two hours ago but used one minute ago must stay
    // alive — changing the agent/model mid-conversation must not discard it.
    const lastActiveAt = now - 60_000;
    expect(isPetSessionFresh(lastActiveAt, now)).toBe(true);

    // Only genuine inactivity past the window starts a fresh session.
    expect(isPetSessionFresh(now - PET_IDLE_TIMEOUT_MS, now)).toBe(false);
  });
});
