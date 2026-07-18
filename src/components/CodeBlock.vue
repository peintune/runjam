<script setup lang="ts">
import { computed, ref } from "vue";
import hljs from "highlight.js";
import { Copy, Check } from "lucide-vue-next";

const props = withDefaults(
  defineProps<{
    code: string;
    lang?: string;
    theme?: "light" | "dark";
  }>(),
  {
    lang: "text",
    theme: "light",
  },
);

const copied = ref(false);

const highlighted = computed(() => {
  const l = props.lang.toLowerCase();
  if (l && hljs.getLanguage(l)) {
    try {
      return hljs.highlight(props.code, { language: l, ignoreIllegals: true }).value;
    } catch {
      return hljs.highlightAuto(props.code).value;
    }
  }
  return hljs.highlightAuto(props.code).value;
});

const displayLang = computed(() => props.lang || "text");

async function handleCopy() {
  try {
    await navigator.clipboard.writeText(props.code);
    copied.value = true;
    setTimeout(() => (copied.value = false), 2000);
  } catch {
    // clipboard not available
  }
}
</script>

<template>
  <div
    class="cb-wrap"
    :class="theme === 'dark' ? 'hljs-theme-dark' : 'hljs-theme-light'"
  >
    <div class="cb-head">
      <span class="cb-lang">{{ displayLang }}</span>
      <button class="cb-copy-btn" @click="handleCopy">
        <Check v-if="copied" :size="12" />
        <Copy v-else :size="12" />
        <span>{{ copied ? "Copied" : "Copy" }}</span>
      </button>
    </div>
    <pre><code
      class="hljs"
      :class="lang ? 'language-' + lang.toLowerCase() : ''"
      v-html="highlighted"
    /></pre>
  </div>
</template>
