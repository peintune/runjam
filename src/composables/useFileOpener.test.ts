import { describe, it, expect } from "vitest";
import { extensionOf, resolveViewer, canPreviewInApp, previewCacheDir, resolveAgainstBase, isAbsolutePath } from "./useFileOpener";

describe("extensionOf", () => {
  it("lowercases the extension", () => {
    expect(extensionOf("/a/B.PPTX")).toBe("pptx");
  });

  it("reads the last dot when a name has several", () => {
    expect(extensionOf("/a/archive.tar.gz")).toBe("gz");
  });

  it("treats a leading dot as part of the name, not an extension", () => {
    // ".gitignore" must not be read as having extension "gitignore".
    expect(extensionOf("/repo/.gitignore")).toBe("");
    expect(extensionOf(".env")).toBe("");
  });

  it("handles a name with no dot", () => {
    expect(extensionOf("/a/Makefile")).toBe("");
  });

  it("handles Windows separators", () => {
    expect(extensionOf("C:\\work\\deck.pptx")).toBe("pptx");
  });
});

describe("resolveViewer", () => {
  it("routes only the rich document types to a dedicated viewer", () => {
    // The policy is deliberately narrow: an Office/web document RunJam can render
    // properly gets its own viewer, and nothing else does.
    expect(resolveViewer("/a/x.docx")).toBe("docx");
    expect(resolveViewer("/a/x.xlsx")).toBe("sheet");
    expect(resolveViewer("/a/x.csv")).toBe("sheet");
    expect(resolveViewer("/a/x.pptx")).toBe("presentation");
    expect(resolveViewer("/a/x.html")).toBe("html");
    expect(resolveViewer("/a/x.htm")).toBe("html");
    expect(resolveViewer("/a/x.png")).toBe("image");
  });

  it("opens everything else in the text editor, like the file tree does", () => {
    // Markdown included: it is a text file, and the editor is where the file tree
    // opens it too — so the two entry points agree. Enumerating every text
    // extension would be a losing game, so this is the default, not a list.
    for (const p of [
      "/a/main.rs", "/a/app.ts", "/a/conf.toml", "/a/log.txt", "/a/Makefile",
      "/a/notes.md", "/a/doc.markdown", "/a/readme.mdx", "/a/style.css",
      "/a/data.json", "/a/spec.yaml", "/a/run.sh",
    ]) {
      expect(resolveViewer(p)).toBe("editor");
    }
  });

  it("sends a PDF to the system viewer instead of a preview", () => {
    // A browser cannot render a PDF in an `<img>` (the old preview came up blank)
    // and Tauri's WKWebView does not render one inline either, so RunJam hands it
    // to the system viewer rather than showing an empty pane.
    expect(resolveViewer("/a/x.pdf")).toBe("unsupported");
    expect(canPreviewInApp("/a/x.pdf")).toBe(false);
  });

  it("marks formats with no in-app viewer as unsupported", () => {
    // These must NOT fall through to the editor, or Monaco would render raw
    // bytes as text. Regression guard: a cleanup pass once removed the binary
    // check and every one of these silently became "editor".
    for (const p of ["/a/x.zip", "/a/x.mp4", "/a/x.bin", "/a/x.exe", "/a/x.ttf", "/a/x.sqlite", "/a/x.doc", "/a/x.pdf"]) {
      expect(resolveViewer(p)).toBe("unsupported");
    }
    // .xls is a legacy binary; it is routed to the sheet viewer (SheetJS reads it).
    expect(resolveViewer("/a/x.xls")).toBe("sheet");
  });

  it("routes every Office format to a real viewer instead of the editor", () => {
    // Each of these must have its OWN branch; none may fall through to `editor`,
    // which would show binary bytes as text.
    expect(resolveViewer("/a/x.docx")).toBe("docx");
    expect(resolveViewer("/a/x.xlsx")).toBe("sheet");
    expect(resolveViewer("/a/x.ods")).toBe("sheet");
    expect(resolveViewer("/a/x.pptx")).toBe("presentation");
    expect(resolveViewer("/a/x.ppt")).toBe("presentation");
    expect(resolveViewer("/a/x.odp")).toBe("presentation");
    // .odt has no in-app viewer, so it must be unsupported — never `editor`.
    expect(resolveViewer("/a/x.odt")).toBe("unsupported");
  });

  it("is case-insensitive", () => {
    expect(resolveViewer("/a/DECK.PPTX")).toBe("presentation");
    expect(resolveViewer("/a/IMG.JPEG")).toBe("image");
  });

  it("does not treat an extensionless file as unsupported", () => {
    // A file with no extension is most likely text; opening it in the editor is
    // far more useful than offering "open with the system app".
    expect(resolveViewer("/a/LICENSE")).toBe("editor");
  });
});

describe("canPreviewInApp", () => {
  it("is true for everything with a viewer, false only for genuinely unsupported", () => {
    expect(canPreviewInApp("/a/x.pptx")).toBe(true);
    expect(canPreviewInApp("/a/x.zip")).toBe(false);
  });
});

describe("previewCacheDir", () => {
  it("nests under the app data dir", () => {
    expect(previewCacheDir("/data/RunJam")).toBe("/data/RunJam/office-preview");
  });

  it("does not double the separator when the input has a trailing slash", () => {
    expect(previewCacheDir("/data/RunJam/")).toBe("/data/RunJam/office-preview");
  });
});
describe("resolveAgainstBase", () => {
  const BASE = "/Users/me/project";

  it("anchors a relative path to the session directory", () => {
    // The bug this fixes: the agent writes `src/main.rs`, the backend resolved it
    // against RunJam's own cwd, and the file could not be opened at all.
    expect(resolveAgainstBase("src/main.rs", BASE)).toBe("/Users/me/project/src/main.rs");
  });

  it("treats a leading ./ as relative", () => {
    expect(resolveAgainstBase("./a/b.md", BASE)).toBe("/Users/me/project/a/b.md");
  });

  it("leaves an absolute path alone", () => {
    expect(resolveAgainstBase("/abs/x.md", BASE)).toBe("/abs/x.md");
  });

  it("leaves a home path alone (the backend expands ~)", () => {
    expect(resolveAgainstBase("~/n.md", BASE)).toBe("~/n.md");
  });

  it("leaves a Windows absolute path alone", () => {
    expect(resolveAgainstBase("C:\\work\\a.md", BASE)).toBe("C:\\work\\a.md");
  });

  it("collapses . and .. segments", () => {
    expect(resolveAgainstBase("a/../b.md", BASE)).toBe("/Users/me/project/b.md");
    expect(resolveAgainstBase("./x/./y.md", BASE)).toBe("/Users/me/project/x/y.md");
  });

  it("does not walk above the root when .. overflows", () => {
    // A stray extra `..` must not turn into a filesystem-root escape.
    expect(resolveAgainstBase("weird/../../up.md", BASE)).toBe("/Users/me/up.md");
  });

  it("returns the path unchanged when there is no base directory", () => {
    // A new-session page has no cwd yet; better to pass the path through than to
    // invent a prefix.
    expect(resolveAgainstBase("src/main.rs", null)).toBe("src/main.rs");
    expect(resolveAgainstBase("src/main.rs", "")).toBe("src/main.rs");
  });

  it("produces one stable form, so the same file cannot open twice", () => {
    // `./a.md` and `a.md` must resolve identically or they become two tabs.
    expect(resolveAgainstBase("./a.md", BASE)).toBe(resolveAgainstBase("a.md", BASE));
  });
});

describe("isAbsolutePath", () => {
  it("recognises the absolute forms", () => {
    expect(isAbsolutePath("/a/b")).toBe(true);
    expect(isAbsolutePath("~/a")).toBe(true);
    expect(isAbsolutePath("C:\\a")).toBe(true);
    expect(isAbsolutePath("\\\\server\\share\\a")).toBe(true);
  });

  it("rejects relative forms", () => {
    expect(isAbsolutePath("a/b")).toBe(false);
    expect(isAbsolutePath("./a")).toBe(false);
    expect(isAbsolutePath("../a")).toBe(false);
  });
});
