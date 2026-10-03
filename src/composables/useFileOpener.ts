import { openWithSystemApp } from "../api/office";
import { useSessionLayout } from "./useSessionLayout";

/**
 * How a file should be presented when opened inside a session.
 *
 * - `editor`   — Monaco text editor (code, config, plain text).
 * - `html`     — sandboxed iframe rendering the page.
 * - `image`    — `<img>`.
 * - `sheet`    — spreadsheet table (xlsx / csv).
 * - `docx`     — Word document rendered to HTML.
 * - `presentation` — pptx, converted to PDF first.
 * - `unsupported`  — nothing fits; offer "open with the system app".
 *
 * A single enum keeps the decision in one place: the session message links and
 * the workspace panel both route through `resolveViewer`, so they can never
 * disagree about what a `.docx` opens as.
 *
 * The file tree keeps its own, older rule (image/PDF preview, everything else in
 * the editor). That is intentional: the two agree for every source/text file,
 * which is the common case, and a document that only gets a rich viewer on the
 * session path is still shown correctly — it is just not reachable from the tree.
 */
export type ViewerKind =
  | "editor"
  | "html"
  | "image"
  | "sheet"
  | "docx"
  | "presentation"
  | "unsupported";

const IMAGE_EXT = new Set(["png", "jpg", "jpeg", "gif", "svg", "webp", "bmp", "ico", "avif"]);

/** Spreadsheets the frontend parses directly into a table. */
const SHEET_EXT = new Set(["xlsx", "xls", "csv", "ods"]);

/** Binary formats with no in-app viewer at all.
 *
 *  Without this list `resolveViewer` would fall through to `editor` for a `.zip`
 *  or a `.mp4`, and Monaco would render the raw bytes as text. Anything here that
 *  gains a real viewer later moves to its own branch in `resolveViewer`. */
const BINARY_EXT = new Set([
  ...IMAGE_EXT,
  "pdf", "doc", "odt", "ods",
  "mp3", "mp4", "avi", "mov", "mkv", "wav", "flac", "ogg",
  "zip", "tar", "gz", "bz2", "xz", "7z", "rar",
  "exe", "dll", "so", "dylib", "bin", "wasm",
  "ttf", "otf", "woff", "woff2",
  "db", "sqlite", "sqlite3",
]);

/** The lowercased extension of a path, or "" when there is none. */
export function extensionOf(path: string): string {
  const name = path.split(/[/\\]/).pop() || "";
  const dot = name.lastIndexOf(".");
  // A leading dot (".gitignore") is a name, not an extension.
  if (dot <= 0) return "";
  return name.slice(dot + 1).toLowerCase();
}

/**
 * Which viewer a path should open in.
 *
 * Text-like extensions are NOT enumerated — anything not recognised as a
 * special binary/document type falls through to `editor`, which is the right
 * default for source files, configs and logs and means a new file type does not
 * silently lose its "open" affordance.
 */
export function resolveViewer(path: string): ViewerKind {
  const ext = extensionOf(path);
  if (!ext) return "editor";

  // Only the "rich document" formats get a dedicated viewer. Everything else —
  // source code, Markdown, config, logs — opens in the text editor, which is
  // what the file tree already does. Keeping the rule this narrow means the two
  // entry points agree by construction: a file opened from a session behaves the
  // same as the same file opened from the tree, except that a document RunJam can
  // render properly gets rendered.
  if (ext === "pptx" || ext === "ppt" || ext === "odp") return "presentation";
  if (ext === "docx") return "docx";
  if (SHEET_EXT.has(ext)) return "sheet";
  if (ext === "html" || ext === "htm") return "html";
  // Images are handled by the editor's binary guard today, but the dedicated
  // preview is the better presentation for a picture, so keep it explicit.
  if (IMAGE_EXT.has(ext)) return "image";

  // No in-app viewer: never hand raw bytes to the text editor, and the UI offers
  // "open with the system app" instead.
  //
  // PDF lives here on purpose. A browser cannot render one in an `<img>` (which is
  // why the old preview came up blank), and an `<iframe>`/`<embed>` is not
  // dependable either — Tauri uses WKWebView on macOS, which does not render PDFs
  // inline. Handing the file to the system viewer is the honest option.
  if (BINARY_EXT.has(ext)) return "unsupported";

  return "editor";
}

/** Whether RunJam can show this file in-app at all (no system app needed). */
export function canPreviewInApp(path: string): boolean {
  return resolveViewer(path) !== "unsupported";
}


/**
 * Cache directory for derived previews (converted PDFs).
 *
 * Exported so the conversion call and the previewer agree on the location. It
 * lives under the app's own data directory — these are RunJam's scratch files,
 * not the user's documents.
 */
export function previewCacheDir(dataDir: string): string {
  return `${dataDir.replace(/[/\\]+$/, "")}/office-preview`;
}

/**
 * Resolve `path` against `baseDir` when it is relative.
 *
 * The agent writes paths relative to its working directory (`src/main.rs`), but
 * the backend opens them relative to the RunJam PROCESS's cwd — a different
 * directory — so a relative path must be anchored to the session's cwd before it
 * is handed over. Without this, every relative path in a message was unopenable.
 *
 * An absolute path (POSIX `/…`, Windows `C:\`, or UNC `\\…`) and a `~` path are
 * returned unchanged: they are already unambiguous (`~` is expanded by the
 * backend's file APIs, which run against the user's home).
 *
 * `.` / `..` segments are collapsed so the tab label and the recorded viewer key
 * are stable, and so two spellings of the same file (`./a.md` and `a.md`) resolve
 * to one tab instead of two.
 */
export function resolveAgainstBase(path: string, baseDir: string | null | undefined): string {
  if (!path) return path;
  if (isAbsolutePath(path)) return normalizeSegments(path);
  if (!baseDir) return path;

  const sep = baseDir.includes("\\") && !baseDir.includes("/") ? "\\" : "/";
  const joined = `${baseDir.replace(/[/\\]+$/, "")}${sep}${path}`;
  return normalizeSegments(joined);
}

/** Whether a path is already absolute (POSIX, Windows drive, or UNC). */
export function isAbsolutePath(path: string): boolean {
  return path.startsWith("/") || path.startsWith("~") || /^[A-Za-z]:[/\\]/.test(path) || path.startsWith("\\\\");
}

/**
 * Collapse `.` and `..` segments in a path without touching the filesystem.
 *
 * A hand-rolled version (rather than `node:path`) because this runs in the
 * browser bundle, where `node:path` is unavailable — and because resolving
 * against the real filesystem would make the result depend on whether the file
 * exists yet.
 */
function normalizeSegments(path: string): string {
  const isWin = /^[A-Za-z]:/.test(path) || path.startsWith("\\\\");
  const sep = isWin ? "\\" : "/";
  const leading = path.match(/^(\/|~\/|[A-Za-z]:[/\\]|\\\\[^/\\]+[/\\][^/\\]+[/\\])/)?.[0] ?? "";
  const rest = path.slice(leading.length);
  const parts: string[] = [];
  for (const seg of rest.split(/[/\\]+/)) {
    if (!seg || seg === ".") continue;
    if (seg === "..") {
      // Never pop past the root/prefix; a stray `..` at the top stays literal.
      if (parts.length > 0 && parts[parts.length - 1] !== "..") parts.pop();
      else if (!leading) parts.push("..");
      continue;
    }
    parts.push(seg);
  }
  // `leading` already ends with the separator (it is the root/prefix), so the
  // join must NOT add another — that doubled it into "//Users/...".
  const head = leading;
  const tail = parts.join(sep);
  if (!tail) return head || ".";
  return head ? `${head}${tail}` : tail;
}

/**
 * Open a file in the right-hand workspace panel, choosing the viewer by type.
 *
 * This is what a file path in a chat message calls. Writing to the shared
 * session layout is enough to make the panel show it — the panel reads that same
 * reactive state, so no event bus or prop drilling is needed.
 *
 * `unsupported` types (a zip, a video) are handed to the system application
 * directly rather than opened into a tab that can only say "cannot preview": the
 * user asked to open the file, and that is the only thing that can satisfy it.
 */
export function openFileInWorkspace(path: string, baseDir?: string | null): void {
  if (!path) return;

  // Anchor a relative path to the session's working directory before anything
  // else looks at it: the viewer decision and the backend both need a real path,
  // and the backend would otherwise resolve against RunJam's own cwd.
  const resolved = resolveAgainstBase(path, baseDir);
  if (!resolved) return;

  if (!canPreviewInApp(resolved)) {
    openWithSystemApp(resolved).catch((e) => console.error("Failed to open file:", e));
    return;
  }

  const { layout, saveLayout, signalFileOpened } = useSessionLayout();
  // Remember which viewer this path needs, so the panel does not have to guess
  // (and so a restored session still opens it correctly).
  layout.fileViewers = { ...layout.fileViewers, [resolved]: resolveViewer(resolved) };
  const idx = layout.openFiles.indexOf(resolved);
  if (idx >= 0) {
    layout.activeFileIndex = idx;
  } else {
    layout.openFiles = [...layout.openFiles, resolved];
    layout.activeFileIndex = layout.openFiles.length - 1;
  }
  saveLayout();
  // Announce the ACTION, not just the resulting state: re-opening an already-open
  // file changes nothing observable, yet the panel still has to be shown.
  signalFileOpened();
}

/** Forget the viewer recorded for a path (called when its tab is closed). */
export function forgetFileViewer(path: string): void {
  const { layout, saveLayout } = useSessionLayout();
  if (!(path in layout.fileViewers)) return;
  const next = { ...layout.fileViewers };
  delete next[path];
  layout.fileViewers = next;
  saveLayout();
}
