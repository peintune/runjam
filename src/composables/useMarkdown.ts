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

marked.use({ renderer });

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
    "xmlns", "data-copy", "data-mermaid", "data-lang",
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
function renderMarkdown(src: string, sanitize: boolean): string {
  try {
    let html = marked.parse(src) as string;
    if (sanitize && typeof DOMPurify?.sanitize === "function") {
      html = DOMPurify.sanitize(html, PURIFY_CONFIG as any) as unknown as string;
    }
    return html;
  } catch {
    return src;
  }
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
  if (!cache) {
    // Streaming content: use a separate cache so we don't evict the shared
    // history cache. The same streaming slice is requested multiple times per
    // render cycle because the full message list re-renders on every tick.
    let html = streamingMdCache.get(src);
    if (html !== undefined) return html;
    const t0 = performance.now();
    // Skip DOMPurify for streaming content — it will be re-parsed within 16ms
    // anyway, and the final (complete) version always goes through full
    // sanitization. This saves ~50% of parse time.
    html = renderMarkdown(src, false);
    onMiss?.(performance.now() - t0);
    streamingMdCache.set(src, html);
    if (streamingMdCache.size > STREAMING_MD_CACHE_MAX) {
      const oldest = streamingMdCache.keys().next().value;
      if (oldest !== undefined) streamingMdCache.delete(oldest);
    }
    return html;
  }
  let html = sharedMdCache.get(src);
  if (html === undefined) {
    const t0 = performance.now();
    html = renderMarkdown(src, opts.sanitize ?? true);
    onMiss?.(performance.now() - t0);
    sharedMdCache.set(src, html);
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
    return renderMarkdown(src, opts.sanitize ?? true);
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
  async function renderMermaidBlocks(container: HTMLElement): Promise<void> {
    const mermaidEls = container.querySelectorAll<HTMLElement>("pre.mermaid");
    if (mermaidEls.length === 0) return;

    try {
      const mermaid = await import("mermaid");

      // Initialize once (mermaid remembers state across calls)
      mermaid.default.initialize({
        startOnLoad: false,
        theme: "base",
        themeVariables: {
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
