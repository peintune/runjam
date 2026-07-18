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

export function useMarkdown() {
  /**
   * Parse & sanitize Markdown → safe HTML.
   * Mermaid blocks are left as `<pre class="mermaid">` placeholders;
   * call `renderMermaidBlocks()` on the container after nextTick.
   */
  function render(src: string, opts: RenderOptions = {}): string {
    const { sanitize = true } = opts;
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

      await mermaid.default.run({ nodes: mermaidEls });
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
