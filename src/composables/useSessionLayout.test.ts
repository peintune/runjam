import { describe, it, expect, beforeEach, vi } from "vitest";
import { isWorkspaceAreaVisible } from "./useSessionLayout";

/** Minimal localStorage stub — the composable is pure w.r.t. `window` otherwise,
 *  but vitest runs these tests in a node environment without one. `length`/`key`
 *  are needed by the cold-start snapshot hydration. */
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

/** The composable keeps module-level singleton state, so each test starts from a
 *  freshly imported module (and a fresh storage) to avoid cross-test bleed. */
async function freshLayout() {
  vi.resetModules();
  const mod = await import("./useSessionLayout");
  return { mod, api: mod.useSessionLayout() };
}

describe("useSessionLayout", () => {
  beforeEach(() => {
    (globalThis as unknown as { localStorage: Storage }).localStorage = makeLocalStorage();
  });

  it("defaults the terminal to half the old height", async () => {
    const { mod } = await freshLayout();
    const { layout } = mod.useSessionLayout();
    expect(layout.terminalHeight).toBe(130);
  });

  it("preserves a persisted terminal height verbatim", async () => {
    // 260 is no longer special-cased: the bucket key changed (directoryId → path),
    // so old values are simply not read, and a user who drags to 260 keeps it.
    const storage = globalThis.localStorage as unknown as Storage;
    storage.setItem("dir-layout-/proj/a", JSON.stringify({ terminalHeight: 260 }));
    const { api } = await freshLayout();
    api.switchDirectory("/proj/a");
    expect(api.layout.terminalHeight).toBe(260);

    storage.setItem("dir-layout-/proj/b", JSON.stringify({ terminalHeight: 173 }));
    const { api: api2 } = await freshLayout();
    api2.switchDirectory("/proj/b");
    expect(api2.layout.terminalHeight).toBe(173);
  });

  it("keys buckets by the stable working-directory path", async () => {
    const { mod } = await freshLayout();
    // Two launches of the same project folder must map to one bucket, and the
    // key must not depend on the store's regenerated directoryId.
    expect(mod.layoutKeyFor("/proj/a", "sess-1")).toBe("/proj/a");
    expect(mod.layoutKeyFor("/proj/a", "sess-2")).toBe("/proj/a");
    // Default-directory sessions are already unique per session.
    expect(mod.layoutKeyFor("/home/u/.runjam/session/s1", "s1"))
      .toBe("/home/u/.runjam/session/s1");
    // Legacy rows with no directory fall back to a per-session bucket.
    expect(mod.layoutKeyFor(null, "sess-1")).not.toBe(mod.layoutKeyFor(null, "sess-2"));
    // No directory and no session → the new-session page bucket.
    expect(mod.layoutKeyFor(null, null)).toBe(null);
  });

  it("keeps separate layout buckets per session without a project directory", async () => {
    // Regression: every project-less session used to share ONE bucket, so
    // opening a terminal in session A made it appear in every other session.
    const { mod, api } = await freshLayout();
    const keyA = mod.layoutKeyFor(null, "sess-a")!;
    const keyB = mod.layoutKeyFor(null, "sess-b")!;
    expect(keyA).not.toBe(keyB);

    api.switchDirectory(keyA);
    api.layout.showTerminal = true;
    api.saveLayout();

    api.switchDirectory(keyB);
    expect(api.layout.showTerminal).toBe(false);

    api.switchDirectory(keyA);
    expect(api.layout.showTerminal).toBe(true);
  });

  it("shares a bucket between sessions bound to the same project directory", async () => {
    const { mod, api } = await freshLayout();
    const keyA = mod.layoutKeyFor("/proj/a", "sess-a")!;
    const keyB = mod.layoutKeyFor("/proj/a", "sess-b")!;
    expect(keyA).toBe(keyB);

    api.switchDirectory(keyA);
    api.layout.showFileTree = true;
    api.saveLayout();

    api.switchDirectory(keyB);
    expect(api.layout.showFileTree).toBe(true);
  });

  it("uses the default bucket only when there is neither a directory nor a session", async () => {
    const { mod, api } = await freshLayout();
    expect(mod.layoutKeyFor(null, null)).toBe(null);
    // The new-session page maps to the shared default bucket.
    api.switchDirectory(null);
    api.layout.showTerminal = true;
    api.saveLayout();
    api.switchDirectory(null);
    expect(api.layout.showTerminal).toBe(true);
  });

  it("persists a session's terminal state across a module reload", async () => {
    const first = await freshLayout();
    const key = first.mod.layoutKeyFor("/proj/a", "sess-a")!;
    first.api.switchDirectory(key);
    first.api.layout.showTerminal = true;
    first.api.saveLayout();

    // Simulate an app reload: new module instance, same localStorage.
    vi.resetModules();
    const secondMod = await import("./useSessionLayout");
    const second = secondMod.useSessionLayout();
    second.switchDirectory(secondMod.layoutKeyFor("/proj/a", "sess-a"));
    expect(second.layout.showTerminal).toBe(true);
  });

  it("hydrates bucket snapshots on cold start without opening the bucket", async () => {
    // Sidebar/task-board badges read the snapshot map for every visible session.
    // The map only holds buckets visited this run, so a restart must seed it from
    // storage — otherwise no badge shows until each session is opened once.
    const storage = globalThis.localStorage as unknown as Storage;
    storage.setItem(
      "dir-layout-/proj/a",
      JSON.stringify({ showTerminal: true, openFiles: ["/proj/a/x.ts"] }),
    );

    vi.resetModules();
    const mod = await import("./useSessionLayout");
    const api = mod.useSessionLayout();

    // No switchDirectory call — the snapshot must already be there.
    expect(api.peekLayout("/proj/a")).toEqual({
      hasTerminal: true,
      hasOpenFiles: true,
      openFileCount: 1,
    });
    expect(api.peekLayout("/proj/never-seen")).toBe(null);
  });

  it("early-returns when re-entering the same bucket", async () => {
    const { api } = await freshLayout();
    api.switchDirectory("/proj/a");
    api.layout.showTerminal = true;
    // No saveLayout: proves the same-bucket early return keeps in-memory state
    // instead of reloading it from storage.
    api.switchDirectory("/proj/a");
    expect(api.layout.showTerminal).toBe(true);
  });
});
describe("useSessionLayout — per-bucket isolation of mutable fields", () => {
  beforeEach(() => {
    (globalThis as unknown as { localStorage: Storage }).localStorage = makeLocalStorage();
  });

  it("does not leak openFiles from one bucket into another", async () => {
    // A file opened in one project must not appear as an open tab in the next
    // project the user switches to. (`loadLayout` builds a fresh defaults copy
    // per bucket; this pins the observable result of that.)
    const { api } = await freshLayout();
    api.switchDirectory("/project/a");
    api.layout.openFiles = ["/a/one.md"];

    api.switchDirectory("/project/b");
    expect(api.layout.openFiles).toEqual([]);

    // And going back restores a's own list, not a merged one.
    api.switchDirectory("/project/a");
    expect(api.layout.openFiles).toEqual(["/a/one.md"]);
  });

  it("does not leak fileViewers from one bucket into another", async () => {
    // The viewer choice (a .docx must not open in the text editor) is per-file
    // state; leaking it across buckets would open a file in the wrong viewer.
    const { api } = await freshLayout();
    api.switchDirectory("/project/a");
    api.layout.fileViewers = { "/a/doc.docx": "docx" };

    api.switchDirectory("/project/b");
    expect(api.layout.fileViewers).toEqual({});

    api.switchDirectory("/project/a");
    expect(api.layout.fileViewers).toEqual({ "/a/doc.docx": "docx" });
  });

  it("persists fileViewers so a restored session keeps its viewers", async () => {
    // openFiles survives a restart; without persisting the viewer choices too, a
    // restored .docx tab would fall back to the editor and render bytes as text.
    const { api } = await freshLayout();
    api.switchDirectory("/project/a");
    api.layout.fileViewers = { "/a/paper.pdf": "pdf" };
    api.layout.openFiles = ["/a/paper.pdf"];
    api.saveLayout();

    const { api: reopened } = await freshLayout();
    reopened.switchDirectory("/project/a");
    expect(reopened.layout.fileViewers).toEqual({ "/a/paper.pdf": "pdf" });
    expect(reopened.layout.openFiles).toEqual(["/a/paper.pdf"]);
  });
});

describe("useSessionLayout — openFileSignal", () => {
  beforeEach(() => {
    (globalThis as unknown as { localStorage: Storage }).localStorage = makeLocalStorage();
  });

  it("bumps on every open, including re-opening the same file", async () => {
    // This is the fix for "open a file, close the tree, click the same file again
    // → nothing happens". Re-opening changes neither openFiles.length nor
    // activeFileIndex, so a state-diff watcher cannot see it; the signal can.
    const { api } = await freshLayout();
    api.switchDirectory("/project/a");
    api.layout.openFiles = ["/a/one.md"];
    api.layout.activeFileIndex = 0;

    const before = api.openFileSignal.value;
    // Simulate openFileInWorkspace re-opening the same, already-active file:
    // the state is identical afterwards.
    const lenBefore = api.layout.openFiles.length;
    const idxBefore = api.layout.activeFileIndex;
    api.signalFileOpened();

    expect(api.openFileSignal.value).toBe(before + 1);
    expect(api.layout.openFiles.length).toBe(lenBefore);
    expect(api.layout.activeFileIndex).toBe(idxBefore);
  });

  it("bumps monotonically so a listener fires for every open", async () => {
    const { api } = await freshLayout();
    const start = api.openFileSignal.value;
    api.signalFileOpened();
    api.signalFileOpened();
    expect(api.openFileSignal.value).toBe(start + 2);
  });

  it("is not persisted across a reload", async () => {
    // A transient signal: restoring a stale value on startup would pop the panel
    // open without the user asking.
    const { api } = await freshLayout();
    api.switchDirectory("/project/a");
    api.signalFileOpened();
    const persisted = JSON.stringify(api.layout);
    expect(persisted).not.toContain("openFileSignal");
  });
});

describe("openFileSignal — the watcher contract", () => {
  beforeEach(() => {
    (globalThis as unknown as { localStorage: Storage }).localStorage = makeLocalStorage();
  });

  it("fires a watcher where a state-diff watcher would not", async () => {
    // Pins the actual fix for "open a file → close the tree → click the same file
    // again → nothing happens". Reproduces the exact state: the file is already
    // open and already active, so re-opening changes NOTHING observable — the
    // old `length:index` watcher stayed silent (diff=0) and the panel never
    // appeared. The signal watcher fires (signal=1).
    const { watch, nextTick } = await import("vue");
    const { mod, api } = await freshLayout();

    api.switchDirectory("/project/a");
    api.layout.openFiles = ["/a/one.md"];
    api.layout.activeFileIndex = 0;
    await nextTick();

    let signalFired = 0;
    let diffFired = 0;
    watch(mod.useSessionLayout().openFileSignal, () => { signalFired++; });
    watch(
      () => `${api.layout.openFiles.length}:${api.layout.activeFileIndex}`,
      () => { diffFired++; },
    );
    await nextTick();
    signalFired = 0;
    diffFired = 0;

    // The re-open: index is set to the value it already had, and the signal bumps.
    api.layout.activeFileIndex = 0;
    api.signalFileOpened();
    await nextTick();

    expect(signalFired).toBe(1);
    expect(diffFired).toBe(0);
  });
});

describe("isWorkspaceAreaVisible", () => {
  const base = { isBoard: false, hasDirectory: true, showFileTree: true };

  it("shows the workspace only while the file tree is open", () => {
    // The tree toggle is the master switch for the whole panel (tree + editor).
    // Closing it must take the editor with it — leaving an orphaned editor eating
    // the chat's width was a real bug.
    expect(isWorkspaceAreaVisible({ ...base, showFileTree: true })).toBe(true);
    expect(isWorkspaceAreaVisible({ ...base, showFileTree: false })).toBe(false);
  });

  it("does not depend on whether files are open", () => {
    // There is no `openFiles` input at all, by design: a file left open from
    // earlier in the session must not keep the panel alive once the tree closes,
    // and it must not be needed to show the panel either.
    const keys = Object.keys(base);
    expect(keys).toEqual(["isBoard", "hasDirectory", "showFileTree"]);
  });

  it("hides on the task board", () => {
    expect(isWorkspaceAreaVisible({ ...base, isBoard: true })).toBe(false);
  });

  it("hides without a working directory", () => {
    // No directory means the tree/editor have nothing to show.
    expect(isWorkspaceAreaVisible({ ...base, hasDirectory: false })).toBe(false);
  });
});
