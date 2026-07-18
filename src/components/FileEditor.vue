<script setup lang="ts">
import { ref, watch, onMounted, onBeforeUnmount, nextTick } from "vue";
import { readFileText, writeFile } from "../api/fs";
import { Loader } from "lucide-vue-next";

const props = defineProps<{
  filePath: string;
}>();

const emit = defineEmits<{
  (e: "close"): void;
}>();

const content = ref("");
const originalContent = ref("");
const loading = ref(true);
const saving = ref(false);
const error = ref("");
const editorContainer = ref<HTMLElement>();

let monacoEditor: any = null;
let monacoModule: any = null;

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

async function initEditor() {
  // Dynamic import to avoid issues with SSR/build
  const monaco = await import("monaco-editor");
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
    automaticLayout: true,
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

function isDirty() {
  return content.value !== originalContent.value;
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
  if (monacoEditor) {
    monacoEditor.dispose();
    monacoEditor = null;
  }
});
</script>

<template>
  <div class="h-full flex flex-col bg-white">
    <!-- editor -->
    <div class="flex-1 min-h-0">
      <div v-if="loading" class="flex items-center justify-center h-full">
        <div class="flex items-center gap-2 text-gray-400">
          <Loader :size="16" class="animate-spin" />
          <span class="text-[13px]">Loading file...</span>
        </div>
      </div>
      <div v-else-if="error" class="flex items-center justify-center h-full">
        <div class="text-center">
          <p class="text-[13px] text-red-500 mb-2">{{ error }}</p>
          <p class="text-[12px] text-gray-400">This file might be binary or too large to open in the editor.</p>
        </div>
      </div>
      <div v-else ref="editorContainer" class="h-full w-full" />
    </div>
  </div>
</template>
