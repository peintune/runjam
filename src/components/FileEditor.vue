<script setup lang="ts">
import { ref, watch, onMounted, onBeforeUnmount, nextTick } from "vue";
import { readFileText, writeFile } from "../api/fs";
import { openInFinder } from "../api/app";
import { Loader, ExternalLink } from "lucide-vue-next";

const props = defineProps<{
  filePath: string;
}>();

const content = ref("");
const originalContent = ref("");
const loading = ref(true);
const saving = ref(false);
const error = ref("");
const editorContainer = ref<HTMLElement>();

let monacoEditor: any = null;
let monacoModule: any = null;

// Debounced layout observer. Monaco's `automaticLayout: true` uses an internal
// ResizeObserver that calls a synchronous layout() on the main thread on EVERY
// resize frame. During the sidebar width animation the editor container resizes
// every frame, so layout() runs ~12× in 200ms and freezes the whole UI for
// large files. We disable automaticLayout and re-layout manually, debounced
// (same pattern the terminal uses for xterm.fit()).
let layoutTimer: ReturnType<typeof setTimeout> | null = null;
let layoutObserver: ResizeObserver | null = null;

// Preload Monaco in the background as soon as this module loads (i.e. when the
// workspace panel mounts). Dynamically importing a ~4MB editor on first file
// open is what made opening a file freeze for seconds. Starting the download
// early means it's almost always ready by the time the user clicks a file.
const monacoPromise: Promise<any> = import("monaco-editor").catch((e) => {
  console.error("Failed to preload monaco-editor:", e);
  return null;
});

/** File extensions that are binary / not editable as text */
const BINARY_EXTENSIONS = new Set([
  "jpg", "jpeg", "png", "gif", "bmp", "ico", "webp", "svg",
  "mp3", "mp4", "avi", "mov", "mkv", "wav", "flac", "ogg",
  "zip", "tar", "gz", "bz2", "xz", "7z", "rar",
  "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx",
  "exe", "dll", "so", "dylib", "bin",
  "ttf", "otf", "woff", "woff2",
  "db", "sqlite", "sqlite3",
  "wasm",
]);

function isBinary(ext: string): boolean {
  return BINARY_EXTENSIONS.has(ext.toLowerCase());
}

function getLanguage(ext: string): string {
  const map: Record<string, string> = {
    ts: "typescript",
    tsx: "typescript",
    js: "javascript",
    jsx: "javascript",
    vue: "html",
    rs: "rust",
    py: "python",
    go: "go",
    java: "java",
    c: "c",
    cpp: "cpp",
    cs: "csharp",
    rb: "ruby",
    php: "php",
    swift: "swift",
    kt: "kotlin",
    scala: "scala",
    sh: "shell",
    bash: "shell",
    zsh: "shell",
    json: "json",
    yaml: "yaml",
    yml: "yaml",
    toml: "ini",
    xml: "xml",
    md: "markdown",
    css: "css",
    scss: "scss",
    less: "less",
    html: "html",
    sql: "sql",
  };
  return map[ext] || "plaintext";
}

async function loadFile() {
  if (!props.filePath) return;
  loading.value = true;
  error.value = "";
  try {
    const ext = props.filePath.split(".").pop() || "";
    if (isBinary(ext)) {
      error.value = `Cannot open binary file (*.${ext}) in the text editor.`;
      loading.value = false;
      return;
    }
    const text = await readFileText(props.filePath);
    content.value = text;
    originalContent.value = text;
    if (monacoEditor) {
      monacoEditor.setValue(text);
    }
  } catch (err: any) {
    error.value = String(err);
  } finally {
    loading.value = false;
  }
}

function handleOpenExternally() {
  if (props.filePath) {
    openInFinder(props.filePath).catch((err) => console.error("Failed to open file:", err));
  }
}

async function initEditor() {
  // Reuse the preloaded module (see monacoPromise above) — avoids re-downloading
  // Monaco on every editor mount.
  const monaco = await monacoPromise;
  if (!monaco) return;
  monacoModule = monaco;

  // Configure workers
  (self as any).MonacoEnvironment = {
    getWorker(_: any, label: string) {
      if (label === "json") {
        return new Worker(
          new URL("monaco-editor/esm/vs/language/json/json.worker.js", import.meta.url),
          { type: "module" }
        );
      }
      if (label === "css" || label === "scss" || label === "less") {
        return new Worker(
          new URL("monaco-editor/esm/vs/language/css/css.worker.js", import.meta.url),
          { type: "module" }
        );
      }
      if (label === "html" || label === "handlebars" || label === "razor") {
        return new Worker(
          new URL("monaco-editor/esm/vs/language/html/html.worker.js", import.meta.url),
          { type: "module" }
        );
      }
      if (label === "typescript" || label === "javascript") {
        return new Worker(
          new URL(
            "monaco-editor/esm/vs/language/typescript/ts.worker.js",
            import.meta.url
          ),
          { type: "module" }
        );
      }
      return new Worker(
        new URL("monaco-editor/esm/vs/editor/editor.worker.js", import.meta.url),
        { type: "module" }
      );
    },
  };

  await nextTick();
  if (!editorContainer.value) return;

  const ext = props.filePath.split(".").pop() || "";
  const language = getLanguage(ext);

  monacoEditor = monaco.editor.create(editorContainer.value, {
    value: content.value,
    language,
    theme: "vs",
    fontSize: 13,
    lineNumbers: "on",
    minimap: { enabled: false },
    scrollBeyondLastLine: false,
    wordWrap: "on",
    automaticLayout: false, // we handle re-layout manually, debounced (see below)
    tabSize: 2,
    renderLineHighlight: "all",
    padding: { top: 12, bottom: 12 },
    glyphMargin: false,
    folding: true,
    lineDecorationsWidth: 8,
    lineNumbersMinChars: 3,
    bracketPairColorization: { enabled: true },
    suggest: { showWords: false },
  });

  if (content.value) {
    monacoEditor.setValue(content.value);
  }

  monacoEditor.onDidChangeModelContent(() => {
    if (monacoEditor) {
      content.value = monacoEditor.getValue();
    }
  });

  // Debounced manual layout. Debounce avoids running layout() synchronously on
  // every resize frame (which is what froze the UI during the sidebar width
  // animation); the trailing 80ms fires once after the resize settles.
  layoutObserver = new ResizeObserver(() => {
    if (layoutTimer) clearTimeout(layoutTimer);
    layoutTimer = setTimeout(() => {
      monacoEditor?.layout();
      layoutTimer = null;
    }, 80);
  });
  layoutObserver.observe(editorContainer.value);
}

async function handleSave() {
  if (!props.filePath || !monacoEditor) return;
  saving.value = true;
  try {
    const currentContent = monacoEditor.getValue();
    await writeFile(props.filePath, currentContent);
    originalContent.value = currentContent;
  } catch (err: any) {
    error.value = String(err);
  } finally {
    saving.value = false;
  }
}

// Keyboard shortcut: Cmd/Ctrl+S to save
function handleKeydown(e: KeyboardEvent) {
  if ((e.metaKey || e.ctrlKey) && e.key === "s") {
    e.preventDefault();
    handleSave();
  }
}

watch(() => props.filePath, async (newPath) => {
  if (newPath) {
    await loadFile();
    if (monacoEditor) {
      const ext = newPath.split(".").pop() || "";
      const lang = getLanguage(ext);
      const model = monacoEditor.getModel();
      if (model && monacoModule) {
        monacoModule.editor.setModelLanguage(model, lang);
      }
      monacoEditor.setValue(content.value);
    } else {
      await initEditor();
    }
  }
}, { immediate: true });

onMounted(() => {
  document.addEventListener("keydown", handleKeydown);
});

onBeforeUnmount(() => {
  document.removeEventListener("keydown", handleKeydown);
  if (layoutObserver) {
    layoutObserver.disconnect();
    layoutObserver = null;
  }
  if (layoutTimer) {
    clearTimeout(layoutTimer);
    layoutTimer = null;
  }
  if (monacoEditor) {
    monacoEditor.dispose();
    monacoEditor = null;
  }
});
</script>

<template>
  <div class="h-full flex flex-col bg-white">
    <!-- editor -->
    <div class="flex-1 min-h-0 relative">
      <!-- Loading overlay (above editor, doesn't remove editorContainer from DOM) -->
      <div v-if="loading" class="absolute inset-0 flex items-center justify-center bg-white z-10">
        <div class="flex items-center gap-2 text-gray-400">
          <Loader :size="16" class="animate-spin" />
          <span class="text-[13px]">Loading file...</span>
        </div>
      </div>
      <!-- Error overlay -->
      <div v-else-if="error" class="absolute inset-0 flex items-center justify-center bg-white z-10">
        <div class="text-center max-w-sm">
          <p class="text-[13px] text-red-500 mb-2">{{ error }}</p>
          <p class="text-[12px] text-gray-400 mb-4">This file might be binary or too large to open in the editor.</p>
          <button
            @click="handleOpenExternally"
            class="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-blue-500 text-white text-[12px] font-medium hover:bg-blue-600 transition-colors cursor-pointer"
          >
            <ExternalLink :size="13" />
            Open with system default app
          </button>
        </div>
      </div>
      <!-- Editor container — always in the DOM so Monaco keeps its element
           even when loading/error overlays are shown. Previously v-if/v-else
           removed this element during loading, causing Monaco to lose its
           container and show no content after switching files. -->
      <div ref="editorContainer" class="h-full w-full" />
    </div>
  </div>
</template>
