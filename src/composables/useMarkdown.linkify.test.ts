import { describe, it, expect } from "vitest";
import { linkifyPaths } from "./useMarkdown";

describe("linkifyPaths", () => {
  it("turns an absolute path with a known extension into an open link", () => {
    const out = linkifyPaths("wrote /Users/me/out/report.html done");
    expect(out).toContain('data-open-file="/Users/me/out/report.html"');
    // The link text is the full path, so the user sees exactly what will open.
    expect(out).toContain(">/Users/me/out/report.html<");
    // The surrounding prose is preserved.
    expect(out).toContain("wrote ");
    expect(out).toContain(" done");
  });

  it("leaves a path with an extension that has no viewer as plain text", () => {
    // .xyz is not openable, so it must NOT become a link — a false positive here
    // turns ordinary prose into something that looks clickable.
    const out = linkifyPaths("saved /Users/me/out/archive.xyz");
    expect(out).not.toContain("data-open-file");
    expect(out).toContain("/Users/me/out/archive.xyz");
  });

  it("does not linkify a directory path (nothing to open it with)", () => {
    const out = linkifyPaths("cd /Users/me/projects");
    expect(out).not.toContain("data-open-file");
  });

  it("links a bare relative path the agent names", () => {
    // The most common form in practice ("I edited src/main.rs"), previously
    // missed because only absolute paths were recognised.
    const out = linkifyPaths("I edited src/main.rs just now");
    expect(out).toContain('data-open-file="src/main.rs"');
  });

  it("leaves a trailing :line reference outside the link", () => {
    const out = linkifyPaths("see src/api/fs.ts:42 for the bug");
    expect(out).toContain('data-open-file="src/api/fs.ts"');
    // The line number stays as visible text.
    expect(out).toContain('</a>:42');
  });

  it("renders a file icon inside the link so it is recognisable", () => {
    const out = linkifyPaths("wrote /a/b.md");
    expect(out).toContain("path-link-icon");
    expect(out).toContain("<svg");
  });

  it("does not linkify bare English words that merely contain a dot", () => {
    const out = linkifyPaths("this is fine. and so is that.");
    expect(out).not.toContain("data-open-file");
  });

  it("does not linkify a known extension when it is not a path-like run", () => {
    // "test.md" appears here as a bare word; with an allowlisted extension it IS
    // linked, which is the accepted trade-off toward recall — assert the current
    // behaviour so a future narrowing shows up as a test change, not a surprise.
    const out = linkifyPaths("named test.md in the report");
    expect(out).toContain('data-open-file="test.md"');
  });

  it("excludes trailing sentence punctuation from the path", () => {
    const out = linkifyPaths("see /Users/me/report.md.");
    expect(out).toContain('data-open-file="/Users/me/report.md"');
    // The period stays outside the link as text.
    expect(out).not.toContain('report.md."');
  });

  it("escapes HTML rather than emitting it", () => {
    // A path is attacker-influenced (the agent writes it), so it must never be
    // interpolated raw — this is the injection boundary.
    const out = linkifyPaths('x <img src=x onerror=alert(1)> /a/b.md');
    expect(out).not.toContain("<img");
    expect(out).toContain("&lt;img");
  });

  it("escapes quotes inside the attribute value", () => {
    // A quote in a path would otherwise break out of the attribute.
    const out = linkifyPaths('/a/we"ird.md');
    expect(out).not.toContain('data-open-file="/a/we"ird.md"');
  });

  it("handles multiple paths in one run", () => {
    const out = linkifyPaths("from /a/one.html and /b/two.pdf");
    expect(out).toContain('data-open-file="/a/one.html"');
    expect(out).toContain('data-open-file="/b/two.pdf"');
  });

  it("handles a tilde path", () => {
    const out = linkifyPaths("see ~/notes/todo.md");
    expect(out).toContain('data-open-file="~/notes/todo.md"');
  });

  it("returns plain escaped text when there is no path", () => {
    const out = linkifyPaths("nothing to see <here>");
    expect(out).toBe("nothing to see &lt;here&gt;");
  });

  it("does not panic on an empty string", () => {
    expect(linkifyPaths("")).toBe("");
  });
});
describe("linkifyPaths — thinking text", () => {
  it("links a path the agent names while planning", () => {
    // Thinking is prose written by the model, often naming the output file.
    const out = linkifyPaths("I'll create /Users/me/out/report.html for you.");
    expect(out).toContain('data-open-file="/Users/me/out/report.html"');
  });

  it("links paths across multiple lines", () => {
    const out = linkifyPaths("Step 1: write /a/one.md\nStep 2: write /b/two.pdf");
    expect(out).toContain('data-open-file="/a/one.md"');
    expect(out).toContain('data-open-file="/b/two.pdf"');
  });

  it("keeps newlines intact (thinking is rendered with pre-wrap)", () => {
    const out = linkifyPaths("first /a/x.md\nsecond");
    expect(out).toContain("\n");
  });

  it("does not link a path with no extension", () => {
    const out = linkifyPaths("writing into /Users/me/out/inspect");
    expect(out).not.toContain("data-open-file");
  });
});
