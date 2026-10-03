import { marked } from "marked";
import hljs from "highlight.js";
import DOMPurify from "dompurify";

marked.setOptions({ breaks: true, gfm: true });

// ── Shared escape helper ──
function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

// ── Copy button HTML (no inline onclick — events attached via delegation) ──
const COPY_BTN_HTML = `<button class="cb-copy" data-copy>
  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
    <rect x="9" y="9" width="13" height="13" rx="2"/>
    <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>
  </svg>
  <span>Copy</span>
</button>`;

// ── Build custom renderer ──
const renderer = new marked.Renderer();

renderer.code = function (obj: { text: string; lang?: string; escaped?: boolean }) {
  const text = obj.text;
  const lang = (obj.lang || "").toLowerCase();

  // Mermaid code blocks → placeholder for lazy mermaid.run()
  if (lang === "mermaid") {
    return `<div class="mermaid-block" data-mermaid="${escapeHtml(text)}"><pre class="mermaid">${escapeHtml(text)}</pre></div>`;
  }

  // Regular code blocks → hljs highlight
  let highlighted: string;
  if (lang && hljs.getLanguage(lang)) {
    try {
      highlighted = hljs.highlight(text, { language: lang, ignoreIllegals: true }).value;
    } catch {
      highlighted = hljs.highlightAuto(text).value;
    }
  } else {
    highlighted = hljs.highlightAuto(text).value;
  }

  return `<div class="cb-wrap">
    <div class="cb-head">
      <span class="cb-lang">${lang || "text"}</span>
      ${COPY_BTN_HTML}
    </div>
    <pre><code class="hljs${lang ? " language-" + lang : ""}">${highlighted}</code></pre>
  </div>`;
};

// Plain-text runs are linkified (see `linkifyPaths` below), so a file path the
// agent reports is clickable. Assigned BEFORE `marked.use` registers the
// renderer: marked stores what it is handed, and mutating it afterwards is not
// guaranteed to take effect.
renderer.text = function (token: { text: string } | string) {
  const text = typeof token === "string" ? token : token.text;
  return linkifyPaths(text);
};

// Inline `<code>` runs are handled too: the agent overwhelmingly writes a path in
// backticks (`src/components/Foo.vue`), and those were the majority case that
// previously stayed inert. The content is rendered through the same linkifier, so
// a code span holding a path becomes a link while `npm install` stays plain code.
//
// A fenced/code BLOCK never reaches here — marked routes it to `renderer.code` —
// so a path inside a code block keeps its exact text, which is what you want when
// reading source.
renderer.codespan = function (token: { text: string } | string) {
  const text = typeof token === "string" ? token : token.text;
  const linked = linkifyPaths(text);
  // Only replace the code styling when a link was actually produced; otherwise
  // keep looking like code (the whole point of a code span).
  if (linked.includes("data-open-file")) {
    return linked;
  }
  return `<code>${escapeText(text)}</code>`;
};

marked.use({ renderer });

// ── Turn file paths in message text into open-action links ──
//
// A session constantly produces files ("wrote /Users/me/out/report.html") and,
// before this, that path was inert text: the only way to see the file was to find
// it in the tree by hand. Now a path RunJam can open is rendered clickable.
//
// Hooked on `renderer.text` and `renderer.codespan` — marked calls those ONLY for
// inline runs, never for a code block or an existing `[link](…)`. That is what
// keeps the transform safe: real markup is left exactly as the author wrote it.
//
// Detection is deliberately biased toward RECALL: the point is to make a produced
// file reachable, and missing the path is the failure the user actually reported.
// A false positive is bounded — the run must still name a known file extension —
// so bare words like "and" or "the" can never become links.
const OPENABLE_PATH_RE =
  /(?<![\w./~-])((?:~\/|\.\.?\/|\/|[A-Za-z]:\\)?[\w@+-][\w./@+-]*\.[A-Za-z][\w]{0,7})/g;

/** Extensions that make a path clickable in message text. Wider than "has a
 *  viewer": a source file opens in the editor, which is a viewer. */
const OPENABLE_EXTENSIONS = new Set([
  "html", "htm", "md", "markdown", "mdx",
  "png", "jpg", "jpeg", "gif", "svg", "webp", "bmp", "ico",
  "pdf", "docx", "xlsx", "xls", "csv", "pptx", "ppt", "odt", "odp", "ods",
  "json", "jsonl", "txt", "log", "yaml", "yml", "toml", "ini", "conf", "env",
  "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go", "java", "kt",
  "c", "cc", "cpp", "h", "hpp", "cs", "rb", "php", "swift", "scala", "lua",
  "css", "scss", "sass", "less", "vue", "svelte", "astro",
  "sh", "bash", "zsh", "fish", "ps1", "sql", "graphql", "proto",
  "xml", "lock", "gradle", "makefile", "dockerfile",
]);

/** HTML-escape, since renderer output is inserted verbatim. */
function escapeText(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

/**
 * Whether a path run should become a link.
 *
 * Requires a real file extension from the allowlist. That single constraint is
 * what keeps the wider pattern honest: it excludes prose and bare identifiers,
 * and a path to a format RunJam cannot open anywhere (a `.dylib`, say) — there is
 * nothing useful to do with those, so they stay text.
 */
function looksOpenablePath(path: string): boolean {
  const name = path.split(/[/\\]/).pop() || "";
  // A trailing line/column reference (file.ts:42) is stripped before the check.
  const bareName = name.replace(/:\d+(-\d+)?$/, "");
  const dot = bareName.lastIndexOf(".");
  if (dot <= 0) return false;
  return OPENABLE_EXTENSIONS.has(bareName.slice(dot + 1).toLowerCase());
}

/**
 * A small inline icon marking the run as a file, chosen by extension so the kind
 * of file is visible at a glance (code vs. document vs. spreadsheet), the same way
 * the file tree does it.
 *
 * These are inline SVG paths rather than the `lucide-vue-next` components the
 * file tree uses: this module produces HTML strings for `v-html`, so it cannot
 * mount Vue components. The shapes are the Lucide outlines for the matching
 * icons, and `PATH_ICON_TONE` mirrors the tree's colour coding.
 */
const ICON_SVG = {
  code: '<path d="m16 18 6-6-6-6"/><path d="m8 6-6 6 6 6"/>',
  text: '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M10 9H8"/><path d="M16 13H8"/><path d="M16 17H8"/>',
  json: '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M10 12a1 1 0 0 0-1 1v1a1 1 0 0 1-1 1 1 1 0 0 1 1 1v1a1 1 0 0 0 1 1"/><path d="M14 18a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1 1 1 0 0 1-1-1v-1a1 1 0 0 0-1-1"/>',
  sheet: '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M8 13h2"/><path d="M14 13h2"/><path d="M8 17h2"/><path d="M14 17h2"/>',
  doc: '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M16 13H8"/><path d="M16 17H8"/><path d="M10 9H8"/>',
  deck: '<path d="M2 3h20"/><path d="M21 3v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V3"/><path d="m7 21 5-5 5 5"/>',
  image: '<rect width="18" height="18" x="3" y="3" rx="2" ry="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"/>',
  pdf: '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M9 15v-3h1.5a1.5 1.5 0 0 1 0 3H9"/><path d="M14 15v-3h2"/>',
  archive: '<rect width="20" height="5" x="2" y="3" rx="1"/><path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8"/><path d="M10 12h4"/>',
  file: '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/>',
} as const;

type IconKind = keyof typeof ICON_SVG;

/** Extension → icon kind, mirroring the file tree's `FILE_ICON_MAP`. */
function iconKindFor(ext: string): IconKind {
  const e = ext.toLowerCase();
  if (["json", "jsonl"].includes(e)) return "json";
  if (["xlsx", "xls", "ods", "csv"].includes(e)) return "sheet";
  if (["docx", "doc", "odt", "rtf"].includes(e)) return "doc";
  if (["pptx", "ppt", "odp"].includes(e)) return "deck";
  if (["png", "jpg", "jpeg", "gif", "svg", "webp", "bmp", "ico", "avif"].includes(e)) return "image";
  if (e === "pdf") return "pdf";
  if (["zip", "tar", "gz", "bz2", "xz", "7z", "rar"].includes(e)) return "archive";
  if (["md", "markdown", "mdx", "txt", "log", "rtf", "ini", "conf", "env"].includes(e)) return "text";
  // Everything else in the allowlist is source code / config.
  return "code";
}

/** Per-kind tint, matching the colours the file tree gives the same types. */
const ICON_TONE: Record<IconKind, string> = {
  code: "#3b82f6",
  text: "#6b7280",
  json: "#eab308",
  sheet: "#16a34a",
  doc: "#2563eb",
  deck: "#ea580c",
  image: "#a855f7",
  pdf: "#dc2626",
  archive: "#a16207",
  file: "#6b7280",
};

/**
 * Build the icon markup for a path.
 *
 * The kind is derived from the full path (a trailing `:42` is already stripped by
 * the caller). Falls back to a generic file glyph, so a link is always visually
 * marked as a file even for an extension not in the maps above.
 */
function fileIconSvg(path: string): string {
  const name = path.split(/[/\\]/).pop() || "";
  const dot = name.lastIndexOf(".");
  const ext = dot > 0 ? name.slice(dot + 1) : "";
  const kind = ext ? iconKindFor(ext) : "file";
  return (
    `<svg class="path-link-icon" width="11" height="11" viewBox="0 0 24 24" fill="none" ` +
    `stroke="${ICON_TONE[kind]}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" ` +
    `aria-hidden="true">${ICON_SVG[kind]}</svg>`
  );
}

/**
 * Wrap openable paths in `<a class="path-link" data-open-file>`, escaping
 * everything else.
 *
 * The attribute (not an inline handler) is deliberate: DOMPurify strips
 * `onclick`, and `data-open-file` is whitelisted, so the click is handled by
 * delegation on the message container — the same pattern the copy button uses.
 * The icon makes the link identifiable at a glance rather than relying on the
 * reader noticing a colour change.
 */
export function linkifyPaths(text: string): string {
  let out = "";
  let last = 0;
  OPENABLE_PATH_RE.lastIndex = 0;
  for (const m of text.matchAll(OPENABLE_PATH_RE)) {
    const raw = m[1];
    const start = m.index ?? 0;
    // Trailing sentence punctuation is not part of the filename ("see /a/b.md."
    // must link /a/b.md). A `:42` line reference is already excluded by the
    // pattern itself, so it is left outside the link as text.
    const path = raw.replace(/[.,;:)\]]+$/, "");
    if (!path || !looksOpenablePath(path)) continue;
    out += escapeText(text.slice(last, start));
    out +=
      `<a class="path-link" data-open-file="${escapeText(path)}" title="${escapeText(path)}">` +
      fileIconSvg(path) +
      `<span class="path-link-text">${escapeText(path)}</span></a>`;
    last = start + path.length;
  }
  out += escapeText(text.slice(last));
  return out;
}

// ── DOMPurify config ──
const PURIFY_CONFIG: Record<string, unknown> = {
  ALLOWED_TAGS: [
    "a", "abbr", "article", "b", "blockquote", "br", "caption", "code", "dd",
    "del", "details", "div", "dl", "dt", "em", "figcaption", "figure", "h1",
    "h2", "h3", "h4", "h5", "h6", "hr", "i", "img", "ins", "kbd", "li",
    "mark", "ol", "p", "pre", "q", "rp", "rt", "ruby", "s", "samp", "small",
    "span", "strike", "strong", "sub", "summary", "sup", "table", "tbody",
    "td", "tfoot", "th", "thead", "tr", "u", "ul", "var",
    // Extra for code-block / mermaid UI
    "button", "svg", "path", "rect", "polyline", "section", "nav", "header", "footer",
  ],
  ALLOWED_ATTR: [
    "href", "target", "rel", "title", "alt", "src", "class", "id", "style",
    "width", "height", "viewBox", "fill", "stroke", "stroke-width",
    "stroke-linecap", "stroke-linejoin", "d", "rx", "ry", "x", "y",
    "xmlns", "data-copy", "data-mermaid", "data-lang", "data-open-file", "aria-hidden",
  ],
};

// ── Public API ──

export interface RenderOptions {
  /** Code-block highlight theme, defaults to 'light' */
  theme?: "light" | "dark";
  /** Sanitize HTML via DOMPurify, defaults to true */
  sanitize?: boolean;
}

// ── Module-level render + shared cache ──
function renderMarkdown(src: string, sanitize: boolean, theme: "light" | "dark" = "light"): string {
  try {
    let html = marked.parse(src) as string;
    if (theme === "dark") {
      // Tag code blocks so the dark hljs theme (and dark surface) applies.
      html = html.replace(/class="cb-wrap"/g, 'class="cb-wrap hljs-theme-dark"');
    }
    if (sanitize && typeof DOMPurify?.sanitize === "function") {
      html = DOMPurify.sanitize(html, PURIFY_CONFIG as any) as unknown as string;
    }
    return html;
  } catch {
    return src;
  }
}

/** Cache keys are theme-scoped so switching themes never serves stale HTML. */
function cacheKey(src: string, theme: "light" | "dark"): string {
  return theme === "dark" ? "d\u0000" + src : "l\u0000" + src;
}

/**
 * Parse & sanitize Markdown with a MODULE-level cache keyed by source string.
 * Markdown parsing (marked + DOMPurify + hljs) is the most expensive step in the
 * streaming hot path. Keeping the cache at module scope lets it survive component
 * re-mounts — switching back to a conversation with history renders instantly
 * instead of re-parsing every message (the old per-component cache was cleared on
 * every session switch, which is part of why switching sessions stuttered).
 */
const sharedMdCache = new Map<string, string>();
const SHARED_MD_CACHE_MAX = 1000;

/**
 * Separate cache for streaming content. Each typewriter tick produces a unique
 * source string, but the same string will be requested many times within the same
 * render cycle (Vue re-renders the entire message list, calling renderContent for
 * every message). This cache absorbs those duplicate requests without touching the
 * shared history cache.
 *
 * Cleared when a message completes (see clearStreamingCache).
 */
const streamingMdCache = new Map<string, string>();
const STREAMING_MD_CACHE_MAX = 200;
export function clearStreamingCache(): void {
  streamingMdCache.clear();
}

/** Drop every module-level render cache (shared + streaming + mermaid SVG).
 *  Called when the app theme switches so stale cached HTML/SVG is discarded. */
export function clearMarkdownCaches(): void {
  sharedMdCache.clear();
  streamingMdCache.clear();
  mermaidSvgCache.clear();
}

/** Returns true if `src` contains a fenced code block (``` or ~~~).
 * 供智能打字机判定复用：含代码块的内容跳过逐字揭示，直接完整显示，
 * 避免流式阶段对代码围栏做上千次不完整的 markdown 解析。 */
export function containsCodeFence(src: string): boolean {
  return /```|~~~/.test(src);
}

// ── Mermaid SVG 渲染缓存 ──
// mermaid.run() 单图 100ms+（布局+排版），同一张图（同一段源码）在会话
// 重挂载/重激活时会反复渲染。缓存 图源码 → SVG outerHTML，命中时直接注入，
// 跳过 mermaid.run。只缓存成功渲染的结果（失败走 code-block fallback）。
const mermaidSvgCache = new Map<string, string>();
const MERMAID_SVG_CACHE_MAX = 50;

export function renderCached(
  src: string,
  opts: RenderOptions = {},
  onMiss?: (renderMs: number) => void,
  cache = true,
): string {
  const theme = opts.theme ?? "light";
  if (!cache) {
    // Streaming content: use a separate cache so we don't evict the shared
    // history cache. The same streaming slice is requested multiple times per
    // render cycle because the full message list re-renders on every tick.
    const key = cacheKey(src, theme);
    let html = streamingMdCache.get(key);
    if (html !== undefined) return html;
    const t0 = performance.now();
    // Skip DOMPurify for streaming content — it will be re-parsed within 16ms
    // anyway, and the final (complete) version always goes through full
    // sanitization. This saves ~50% of parse time.
    html = renderMarkdown(src, false, theme);
    onMiss?.(performance.now() - t0);
    streamingMdCache.set(key, html);
    if (streamingMdCache.size > STREAMING_MD_CACHE_MAX) {
      const oldest = streamingMdCache.keys().next().value;
      if (oldest !== undefined) streamingMdCache.delete(oldest);
    }
    return html;
  }
  const key = cacheKey(src, theme);
  let html = sharedMdCache.get(key);
  if (html === undefined) {
    const t0 = performance.now();
    html = renderMarkdown(src, opts.sanitize ?? true, theme);
    onMiss?.(performance.now() - t0);
    sharedMdCache.set(key, html);
    if (sharedMdCache.size > SHARED_MD_CACHE_MAX) {
      const oldest = sharedMdCache.keys().next().value;
      if (oldest !== undefined) sharedMdCache.delete(oldest);
    }
  }
  return html;
}

export function useMarkdown() {
  /**
   * Parse & sanitize Markdown → safe HTML.
   * Mermaid blocks are left as `<pre class="mermaid">` placeholders;
   * call `renderMermaidBlocks()` on the container after nextTick.
   */
  function render(src: string, opts: RenderOptions = {}): string {
    return renderMarkdown(src, opts.sanitize ?? true, opts.theme);
  }

  /** Returns true if `src` contains at least one mermaid code fence */
  function hasMermaid(src: string): boolean {
    return /```mermaid/i.test(src);
  }

  /**
   * Safe streaming slice: truncates `src` to a parse-safe boundary,
   * avoiding half-open code fences / HTML tags / partial tables.
   */
  function safeSliceForStreaming(src: string): string {
    // 1. Ensure code fences are balanced
    const fenceMatches = [...src.matchAll(/```/g)];
    if (fenceMatches.length % 2 !== 0) {
      const last = fenceMatches[fenceMatches.length - 1];
      if (last.index !== undefined) return src.substring(0, last.index);
    }

    // 2. Avoid cutting inside an HTML tag
    const lastOpen = src.lastIndexOf("<");
    const lastClose = src.lastIndexOf(">");
    if (lastOpen > lastClose) return src.substring(0, lastOpen);

    // 3. Prefer cutting at a double-newline paragraph boundary
    //    if the tail is short (avoids losing meaningful content)
    const tailLen = 300;
    const searchStart = Math.max(0, src.length - tailLen);
    const lastBreak = src.lastIndexOf("\n\n", src.length - 1);
    if (lastBreak > searchStart) return src.substring(0, lastBreak);

    // 4. Fallback: cut at last single newline if within tail
    const lastNewline = src.lastIndexOf("\n", src.length - 1);
    if (lastNewline > searchStart) return src.substring(0, lastNewline);

    return src;
  }

  /** Return a CSS class name for the hljs theme container */
  function highlightThemeClass(theme: "light" | "dark"): string {
    return theme === "dark" ? "hljs-theme-dark" : "hljs-theme-light";
  }

  /**
   * Render all `<pre class="mermaid">` elements inside `container`.
   * Lazily imports mermaid on first call. Safe to call multiple times.
   */
  async function renderMermaidBlocks(container: HTMLElement, theme: "light" | "dark" = "light"): Promise<void> {
    const mermaidEls = container.querySelectorAll<HTMLElement>("pre.mermaid");
    if (mermaidEls.length === 0) return;

    try {
      const mermaid = await import("mermaid");

      // Initialize once (mermaid remembers state across calls)
      const isDark = theme === "dark";
      mermaid.default.initialize({
        startOnLoad: false,
        theme: "base",
        themeVariables: isDark
          ? {
              primaryColor: "#1e1b4b",
              primaryBorderColor: "#6366f1",
              primaryTextColor: "#e2e8f0",
              lineColor: "#818cf8",
              secondaryColor: "#312e81",
              tertiaryColor: "#0f172a",
              edgeLabelBackground: "#1e1e36",
              fontSize: "14px",
              fontFamily: "Inter, -apple-system, BlinkMacSystemFont, sans-serif",
            }
          : {
              primaryColor: "#f0f2ff",
              primaryBorderColor: "#6366f1",
              primaryTextColor: "#1e1e2e",
              lineColor: "#6366f1",
              secondaryColor: "#fef3c7",
              tertiaryColor: "#ecfdf5",
              // Edge label background
              edgeLabelBackground: "#ffffff",
              // Node text
              fontSize: "14px",
              fontFamily: "Inter, -apple-system, BlinkMacSystemFont, sans-serif",
            },
      });

      // 命中缓存：直接注入 SVG，跳过 mermaid.run（主要成本）
      const toRender: HTMLElement[] = [];
      for (const el of mermaidEls) {
        const raw = el.textContent || "";
        const cached = mermaidSvgCache.get(raw);
        const wrapper = el.closest(".mermaid-block");
        if (cached && wrapper) {
          wrapper.innerHTML = cached;
        } else {
          toRender.push(el);
        }
      }

      if (toRender.length > 0) {
        await mermaid.default.run({ nodes: toRender });
        // 收集刚渲染的 SVG 入缓存（key = 图源码）
        for (const el of toRender) {
          const raw = el.textContent || "";
          const wrapper = el.closest(".mermaid-block");
          const svgEl = wrapper?.querySelector("svg");
          if (svgEl) {
            mermaidSvgCache.set(raw, svgEl.outerHTML);
            if (mermaidSvgCache.size > MERMAID_SVG_CACHE_MAX) {
              const oldest = mermaidSvgCache.keys().next().value;
              if (oldest !== undefined) mermaidSvgCache.delete(oldest);
            }
          }
        }
      }
    } catch (err) {
      console.warn("[useMarkdown] Mermaid render failed, falling back to code block:", err);
      // Replace each mermaid element with a code-block fallback
      mermaidEls.forEach((el) => {
        const wrapper = el.closest(".mermaid-block");
        if (wrapper) {
          const raw = wrapper.getAttribute("data-mermaid") || el.textContent || "";
          wrapper.outerHTML = `<div class="cb-wrap mermaid-fallback">
            <div class="cb-head"><span class="cb-lang">mermaid</span><span class="cb-lang-error">render error</span></div>
            <pre><code>${escapeHtml(raw)}</code></pre>
          </div>`;
        }
      });
    }
  }

  return {
    render,
    hasMermaid,
    safeSliceForStreaming,
    highlightThemeClass,
    renderMermaidBlocks,
  };
}
