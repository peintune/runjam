<script setup lang="ts">
import { ref, onMounted, watch } from "vue";
import { AlertTriangle } from "lucide-vue-next";

const props = defineProps<{ code: string }>();

const container = ref<HTMLElement | null>(null);
const renderError = ref(false);
const rendered = ref(false);

async function renderDiagram() {
  if (rendered.value || !container.value) return;

  const el = container.value;
  try {
    const mermaid = await import("mermaid");

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
        edgeLabelBackground: "#ffffff",
        fontSize: "14px",
        fontFamily: "Inter, -apple-system, BlinkMacSystemFont, sans-serif",
      },
    });

    // mermaid uses a unique id; generate one
    const id = `mermaid-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    const { svg } = await mermaid.default.render(id, props.code);
    if (el) {
      el.innerHTML = svg;
      rendered.value = true;
    }
  } catch (err) {
    console.warn("[MermaidBlock] render error:", err);
    renderError.value = true;
  }
}

onMounted(() => renderDiagram());
watch(() => props.code, () => {
  rendered.value = false;
  renderError.value = false;
  renderDiagram();
});
</script>

<template>
  <div class="mermaid-block-wrapper">
    <!-- Header -->
    <div class="mermaid-block-head">
      <span class="mermaid-block-lang">mermaid</span>
    </div>

    <!-- Render target / error fallback -->
    <div v-if="renderError" class="mermaid-block-error">
      <AlertTriangle :size="14" />
      <span>Mermaid render failed</span>
      <pre class="mermaid-block-code">{{ code }}</pre>
    </div>
    <div v-else ref="container" class="mermaid-block-svg" />

    <!-- Loading skeleton -->
    <div
      v-if="!rendered && !renderError"
      class="mermaid-block-loading"
    >
      <div class="mermaid-skeleton-line w-3/4" />
      <div class="mermaid-skeleton-line w-1/2" />
      <div class="mermaid-skeleton-line w-5/6" />
      <div class="mermaid-skeleton-line w-2/3" />
    </div>
  </div>
</template>

<style scoped>
.mermaid-block-wrapper {
  margin: 1.25rem 0;
  border-radius: 12px;
  overflow: hidden;
  background: #ffffff;
  border: 1px solid #e4e7ed;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.04);
}

.mermaid-block-head {
  display: flex;
  align-items: center;
  padding: 7px 14px;
  background: #f8f9fc;
  border-bottom: 1px solid #eef0f4;
}

.mermaid-block-lang {
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: #6366f1;
  font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace;
}

.mermaid-block-svg {
  padding: 20px;
  display: flex;
  justify-content: center;
  overflow-x: auto;
}

.mermaid-block-svg :deep(svg) {
  max-width: 100%;
  height: auto;
}

.mermaid-block-error {
  padding: 16px 20px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  color: #92400e;
  font-size: 13px;
  background: #fffbeb;
}

.mermaid-block-code {
  margin-top: 8px;
  padding: 12px;
  background: #f6f8fa;
  border-radius: 8px;
  font-size: 12px;
  line-height: 1.6;
  color: #586069;
  white-space: pre-wrap;
  word-break: break-all;
  width: 100%;
  max-height: 200px;
  overflow-y: auto;
}

/* Skeleton loading */
.mermaid-block-loading {
  padding: 24px 20px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.mermaid-skeleton-line {
  height: 12px;
  border-radius: 6px;
  background: linear-gradient(90deg, #f0f2f5 25%, #e4e7ed 50%, #f0f2f5 75%);
  background-size: 200% 100%;
  animation: shimmer 1.5s infinite;
}

@keyframes shimmer {
  0% { background-position: 200% 0; }
  100% { background-position: -200% 0; }
}
</style>
