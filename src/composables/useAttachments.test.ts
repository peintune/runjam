import { describe, it, expect, vi, beforeEach } from "vitest";

// `useAttachments` imports the Tauri dialog/core bindings at module load; stub
// them so the pure helpers can be exercised in a plain Node test.
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../api/fs", () => ({ parseFile: vi.fn() }));

import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { parseFile } from "../api/fs";
function parsed(content: string, over: Partial<import("../api/fs").ParsedFile> = {}) {
  return { name: "f", path: "/tmp/f", content, size: content.length, truncated: false, error: null, ...over };
}

import {
  buildAttachmentPayload,
  attachmentDisplaySuffix,
  formatFileSize,
  pickAttachedFiles,
  MAX_ATTACH_TOTAL_CHARS,
  type AttachedFile,
} from "./useAttachments";

function file(name: string, over: Partial<AttachedFile> = {}): AttachedFile {
  return {
    path: `/tmp/${name}`,
    name,
    size: 100,
    ext: name.split(".").pop() ?? "",
    parsedContent: "",
    truncated: false,
    error: "",
    ...over,
  };
}

describe("formatFileSize", () => {
  it("scales bytes → KB → MB", () => {
    expect(formatFileSize(512)).toBe("512 B");
    expect(formatFileSize(2048)).toBe("2.0 KB");
    expect(formatFileSize(3 * 1024 * 1024)).toBe("3.0 MB");
  });
});

describe("buildAttachmentPayload", () => {
  it("returns the user's text unchanged when nothing is attached", async () => {
    const r = await buildAttachmentPayload([], "hello");
    expect(r.text).toBe("hello");
    expect(r.failures).toEqual([]);
  });

  it("appends parsed file content after the user's message", async () => {
    vi.mocked(parseFile).mockResolvedValue(parsed("FILE BODY"));
    const f = file("a.txt");
    const r = await buildAttachmentPayload([f], "question");
    expect(r.text).toBe("question\n\n[Attached File: a.txt]\nFILE BODY");
    // The struct is mutated in place so the UI can show parse state.
    expect(f.parsedContent).toBe("FILE BODY");
  });

  it("uses the file body alone when the user typed nothing", async () => {
    vi.mocked(parseFile).mockResolvedValue(parsed("BODY"));
    const r = await buildAttachmentPayload([file("a.txt")], "");
    expect(r.text).toBe("[Attached File: a.txt]\nBODY");
  });

  it("reports parse failures and sends only what succeeded", async () => {
    vi.mocked(parseFile)
      .mockResolvedValueOnce(parsed("", { error: "unsupported" }))
      .mockResolvedValueOnce(parsed("OK"));
    const r = await buildAttachmentPayload([file("bad.xyz"), file("good.txt")], "q");
    expect(r.failures).toEqual(["bad.xyz: unsupported"]);
    expect(r.text).toBe("q\n\n[Attached File: good.txt]\nOK");
  });

  it("keeps the message when a parse throws", async () => {
    vi.mocked(parseFile).mockRejectedValue(new Error("boom"));
    const r = await buildAttachmentPayload([file("a.txt")], "q");
    expect(r.failures[0]).toContain("boom");
    // No content made it through → the original text is sent as-is.
    expect(r.text).toBe("q");
  });

  it("caps the total attached text", async () => {
    vi.mocked(parseFile).mockResolvedValue(parsed("x".repeat(MAX_ATTACH_TOTAL_CHARS + 5000)));
    const r = await buildAttachmentPayload([file("big.txt")], "q");
    expect(r.text).toContain("[Attachments truncated:");
    // Cap + a little room for the user text and the truncation notice.
    expect(r.text.length).toBeLessThan(MAX_ATTACH_TOTAL_CHARS + 500);
  });
});

describe("attachmentDisplaySuffix", () => {
  it("lists only the file names — never the bodies", () => {
    expect(attachmentDisplaySuffix([file("a.txt"), file("b.md")])).toBe(
      "\n\n📎 a.txt\n📎 b.md",
    );
  });

  it("is empty with no attachments", () => {
    expect(attachmentDisplaySuffix([])).toBe("");
  });
});

describe("pickAttachedFiles", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
    vi.mocked(open).mockReset();
  });

  it("returns nothing when the picker is cancelled", async () => {
    vi.mocked(open).mockResolvedValue(null);
    expect(await pickAttachedFiles(new Set())).toEqual([]);
  });

  it("skips paths already attached and reads each size", async () => {
    vi.mocked(open).mockResolvedValue(["/p/kept.txt", "/p/dup.txt"]);
    vi.mocked(invoke).mockResolvedValue(1234);
    const out = await pickAttachedFiles(new Set(["/p/dup.txt"]));
    expect(out.map((f) => f.path)).toEqual(["/p/kept.txt"]);
    expect(out[0].size).toBe(1234);
    expect(out[0].name).toBe("kept.txt");
    expect(out[0].ext).toBe("txt");
  });

  it("still attaches a file whose size cannot be read", async () => {
    vi.mocked(open).mockResolvedValue("/p/a.txt");
    vi.mocked(invoke).mockRejectedValue(new Error("nope"));
    const out = await pickAttachedFiles(new Set());
    expect(out).toHaveLength(1);
    expect(out[0].size).toBe(0);
  });
});