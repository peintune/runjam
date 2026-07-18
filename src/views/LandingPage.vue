<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount } from "vue";
import {
  Zap, Terminal, Shield, Cpu, Layers, Github,
  Download, ChevronDown, ArrowRight, CheckCircle2,
  Monitor, Code2, Box, Sparkles, Workflow, Database,
  MessageSquare, Globe,
} from "lucide-vue-next";
import { useI18n } from "../i18n";

const { t, lang, toggleLang } = useI18n();

// ═══ Intersection Observer for scroll-reveal animations ═══
const reveals = ref<Map<string, boolean>>(new Map());
let observer: IntersectionObserver | null = null;

onMounted(() => {
  observer = new IntersectionObserver(
    (entries) => {
      entries.forEach((entry) => {
        if (entry.isIntersecting) {
          const id = (entry.target as HTMLElement).dataset.revealId;
          if (id) reveals.value.set(id, true);
          observer?.unobserve(entry.target);
        }
      });
    },
    { threshold: 0.15, rootMargin: "0px 0px -40px 0px" },
  );
  document.querySelectorAll("[data-reveal-id]").forEach((el) => {
    observer?.observe(el);
  });
});

onBeforeUnmount(() => observer?.disconnect());

// ═══ Recompute intersection observer after language switch ═══
// The DOM changes when language switches (v-if/v-else patterns etc), so
// we re-run the observer on next tick after any lang change.
// We handle this by watching the lang and re-observing.
// Since the component template uses v-for with computed keys, Vue handles diffing.
// Intersection observer targets are id-based, which don't change. So no extra work needed.

// ═══ FAQ state ═══
const faqOpen = ref<Set<number>>(new Set());
function toggleFaq(i: number) {
  if (faqOpen.value.has(i)) faqOpen.value.delete(i);
  else faqOpen.value.add(i);
}

// ═══ Reactive i18n data ═══
const features = computed(() => [
  { icon: Zap, title: t("features.cards.0.title"), desc: t("features.cards.0.desc") },
  { icon: Layers, title: t("features.cards.1.title"), desc: t("features.cards.1.desc") },
  { icon: Terminal, title: t("features.cards.2.title"), desc: t("features.cards.2.desc") },
  { icon: Shield, title: t("features.cards.3.title"), desc: t("features.cards.3.desc") },
  { icon: Cpu, title: t("features.cards.4.title"), desc: t("features.cards.4.desc") },
  { icon: Workflow, title: t("features.cards.5.title"), desc: t("features.cards.5.desc") },
]);

const agents = [
  { name: "Claude Code", color: "#f59e0b", descKey: "Anthropic", cmd: "claude -p" },
  { name: "Codex CLI", color: "#6366f1", descKey: "OpenAI", cmd: "codex exec" },
  { name: "Gemini CLI", color: "#10b981", descKey: "Google", cmd: "gemini chat" },
];

// We access it via the raw locale data.
import { locales, currentLang } from "../i18n";
const agentStepsList = computed(() => locales[currentLang.value].agents.steps);

const archCards = computed(() => [
  { icon: Cpu, title: t("architecture.cards.0.title"), desc: t("architecture.cards.0.desc"), color: "#f59e0b" },
  { icon: Monitor, title: t("architecture.cards.1.title"), desc: t("architecture.cards.1.desc"), color: "#10b981" },
  { icon: Box, title: t("architecture.cards.2.title"), desc: t("architecture.cards.2.desc"), color: "#6366f1" },
]);

const comparisons = computed(() => [
  { feature: t("compare.rows.0.feature"), runjam: t("compare.rows.0.runjam"), aionui: t("compare.rows.0.aionui"), cursor: t("compare.rows.0.cursor") },
  { feature: t("compare.rows.1.feature"), runjam: t("compare.rows.1.runjam"), aionui: t("compare.rows.1.aionui"), cursor: t("compare.rows.1.cursor") },
  { feature: t("compare.rows.2.feature"), runjam: t("compare.rows.2.runjam"), aionui: t("compare.rows.2.aionui"), cursor: t("compare.rows.2.cursor") },
  { feature: t("compare.rows.3.feature"), runjam: t("compare.rows.3.runjam"), aionui: t("compare.rows.3.aionui"), cursor: t("compare.rows.3.cursor") },
  { feature: t("compare.rows.4.feature"), runjam: t("compare.rows.4.runjam"), aionui: t("compare.rows.4.aionui"), cursor: t("compare.rows.4.cursor") },
  { feature: t("compare.rows.5.feature"), runjam: t("compare.rows.5.runjam"), aionui: t("compare.rows.5.aionui"), cursor: t("compare.rows.5.cursor") },
  { feature: t("compare.rows.6.feature"), runjam: t("compare.rows.6.runjam"), aionui: t("compare.rows.6.aionui"), cursor: t("compare.rows.6.cursor") },
]);

const faqs = computed(() => [
  { q: t("faq.items.0.q"), a: t("faq.items.0.a") },
  { q: t("faq.items.1.q"), a: t("faq.items.1.a") },
  { q: t("faq.items.2.q"), a: t("faq.items.2.a") },
  { q: t("faq.items.3.q"), a: t("faq.items.3.a") },
  { q: t("faq.items.4.q"), a: t("faq.items.4.a") },
  { q: t("faq.items.5.q"), a: t("faq.items.5.a") },
]);

const highlightChatBullets = computed(() => [
  t("highlights.chat.bullets.0"),
  t("highlights.chat.bullets.1"),
  t("highlights.chat.bullets.2"),
  t("highlights.chat.bullets.3"),
]);

const modelProviders = computed(() => [
  ...locales[currentLang.value].highlights.model.providers,
  locales[currentLang.value].highlights.model.custom,
]);

// ═══ Scroll to section ═══
function scrollToSection(id: string) {
  document.getElementById(id)?.scrollIntoView({ behavior: "smooth" });
}
</script>

<template>
  <div class="landing-page min-h-screen bg-[#0a0a0f] text-white font-sans overflow-x-hidden">
    <!-- ═══════════ Navigation ═══════════ -->
    <nav class="fixed top-0 inset-x-0 z-50 bg-[#0a0a0f]/80 backdrop-blur-xl border-b border-white/[0.06]">
      <div class="max-w-6xl mx-auto px-6 h-14 flex items-center justify-between">
        <div class="flex items-center gap-2.5">
          <img src="/runjam-logo.svg" alt="RunJam" class="w-7 h-7 rounded-lg" />
          <span class="font-bold text-[17px] tracking-tight">Run<span style="color: #10b981">Jam</span></span>
        </div>
        <div class="flex items-center gap-5 text-[13px] text-gray-400 font-medium">
          <button @click="scrollToSection('features')" class="hover:text-white transition-colors cursor-pointer">{{ t("nav.features") }}</button>
          <button @click="scrollToSection('architecture')" class="hover:text-white transition-colors cursor-pointer">{{ t("nav.architecture") }}</button>
          <button @click="scrollToSection('compare')" class="hover:text-white transition-colors cursor-pointer">{{ t("nav.compare") }}</button>
          <button @click="scrollToSection('faq')" class="hover:text-white transition-colors cursor-pointer">{{ t("nav.faq") }}</button>
          <!-- Language switcher -->
          <button
            @click="toggleLang()"
            class="flex items-center gap-1 px-2.5 py-1.5 rounded-lg border border-white/[0.08] hover:border-white/25 transition-colors cursor-pointer text-gray-400 hover:text-white"
            :title="t('nav.language')"
          >
            <Globe :size="13" />
            <span>{{ t("nav.language") }}</span>
          </button>
          <a
            href="https://github.com"
            target="_blank"
            class="flex items-center gap-1.5 px-3.5 py-1.5 rounded-lg border border-white/[0.1] hover:border-white/30 transition-colors cursor-pointer"
          >
            <Github :size="14" />
            <span>{{ t("nav.github") }}</span>
          </a>
        </div>
      </div>
    </nav>

    <!-- ═══════════ Hero ═══════════ -->
    <section id="hero" class="relative pt-32 pb-20 md:pt-40 md:pb-28 overflow-hidden">
      <div class="absolute top-0 left-1/2 -translate-x-1/2 w-[800px] h-[600px] bg-[radial-gradient(ellipse_at_center,rgba(16,185,129,0.15)_0%,transparent_70%)] pointer-events-none" />
      <div class="absolute top-20 left-1/4 w-[400px] h-[400px] bg-[radial-gradient(ellipse_at_center,rgba(99,102,241,0.1)_0%,transparent_70%)] pointer-events-none" />

      <div class="max-w-5xl mx-auto px-6 text-center relative z-10">
        <!-- Badge -->
        <div
          data-reveal-id="hero-badge"
          :class="['inline-flex items-center gap-2 px-3.5 py-1.5 rounded-full bg-white/[0.05] border border-white/[0.08] text-[12px] text-gray-400 mb-8', reveals.get('hero-badge') ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-4']"
          style="transition: all 0.6s ease 0.1s;"
        >
          <Sparkles :size="13" class="text-emerald-400" />
          {{ t("hero.badge") }}
        </div>

        <!-- Headline -->
        <h1
          data-reveal-id="hero-title"
          :class="['text-4xl sm:text-5xl md:text-6xl lg:text-7xl font-extrabold tracking-tight leading-[1.1] mb-6', reveals.get('hero-title') ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-4']"
          style="transition: all 0.6s ease 0.2s;"
        >
          {{ t("hero.title1") }}<br>
          <span class="bg-gradient-to-r from-emerald-400 via-emerald-300 to-indigo-400 bg-clip-text text-transparent">
            {{ t("hero.title2") }}
          </span><br>
          {{ t("hero.title3") }}
        </h1>

        <!-- Subtitle -->
        <p
          data-reveal-id="hero-sub"
          :class="['max-w-2xl mx-auto text-[15px] sm:text-base text-gray-400 leading-relaxed mb-10', reveals.get('hero-sub') ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-4']"
          style="transition: all 0.6s ease 0.3s;"
          v-html="t('hero.subtitle')"
        />

        <!-- CTA Buttons -->
        <div
          data-reveal-id="hero-cta"
          :class="['flex items-center justify-center gap-4 mb-16', reveals.get('hero-cta') ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-4']"
          style="transition: all 0.6s ease 0.4s;"
        >
          <button
            class="px-6 py-3 rounded-xl bg-gradient-to-r from-emerald-500 to-emerald-600 text-white text-[14px] font-semibold hover:from-emerald-400 hover:to-emerald-500 transition-all shadow-lg shadow-emerald-500/25 flex items-center gap-2 cursor-pointer"
          >
            <Download :size="16" />
            {{ t("hero.download") }}
          </button>
          <a
            href="https://github.com"
            target="_blank"
            class="px-6 py-3 rounded-xl border border-white/[0.1] bg-white/[0.03] text-white text-[14px] font-semibold hover:bg-white/[0.06] hover:border-white/20 transition-all flex items-center gap-2 cursor-pointer"
          >
            <Github :size="16" />
            {{ t("hero.viewSource") }}
          </a>
        </div>

        <!-- Terminal Demo -->
        <div
          data-reveal-id="hero-term"
          :class="['max-w-3xl mx-auto rounded-2xl border border-white/[0.08] bg-[#0d0d14] overflow-hidden shadow-2xl shadow-emerald-500/[0.05]', reveals.get('hero-term') ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-8']"
          style="transition: all 0.7s ease 0.5s;"
        >
          <div class="flex items-center gap-2 px-4 py-3 bg-white/[0.02] border-b border-white/[0.05]">
            <div class="w-2.5 h-2.5 rounded-full bg-red-500/70" />
            <div class="w-2.5 h-2.5 rounded-full bg-yellow-500/70" />
            <div class="w-2.5 h-2.5 rounded-full bg-green-500/70" />
            <span class="ml-3 text-[11px] text-gray-500 font-mono">~/projects/my-app</span>
          </div>
          <div class="p-5 text-left font-mono text-[13px] leading-relaxed">
            <div class="flex items-start gap-2 text-gray-400 mb-1">
              <span class="text-emerald-400 shrink-0">→</span>
              <span>{{ t("terminal.prompt") }}</span>
            </div>
            <div class="flex items-start gap-2 text-gray-500 mb-0.5">
              <span class="shrink-0">⚡</span>
              <span>{{ t("terminal.running") }}</span>
            </div>
            <div class="flex items-start gap-2 text-gray-500 mb-0.5">
              <span class="shrink-0">⏳</span>
              <span>{{ t("terminal.thinking") }}</span>
            </div>
            <div class="flex items-start gap-2 text-gray-400 mb-0.5">
              <span class="shrink-0">🔧</span>
              <span>{{ t("terminal.tool") }}</span>
            </div>
            <div class="flex items-start gap-2 text-green-400/80">
              <span class="shrink-0">✓</span>
              <span>{{ t("terminal.done") }}</span>
            </div>
          </div>
        </div>
      </div>

      <div class="absolute bottom-8 left-1/2 -translate-x-1/2 text-gray-600 animate-bounce cursor-pointer" @click="scrollToSection('features')">
        <ChevronDown :size="20" />
      </div>
    </section>

    <!-- ═══════════ Features ═══════════ -->
    <section id="features" class="py-24 md:py-32 relative">
      <div class="max-w-6xl mx-auto px-6">
        <div class="text-center mb-16">
          <h2
            data-reveal-id="feat-title"
            :class="['text-3xl sm:text-4xl font-bold tracking-tight mb-4', reveals.get('feat-title') ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-4']"
            style="transition: all 0.6s ease;"
          >
            {{ t("features.title") }}
          </h2>
          <p
            data-reveal-id="feat-sub"
            :class="['text-[15px] text-gray-400 max-w-xl mx-auto', reveals.get('feat-sub') ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-4']"
            style="transition: all 0.6s ease 0.15s;"
          >
            {{ t("features.subtitle") }}
          </p>
        </div>

        <div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-5">
          <div
            v-for="(f, i) in features"
            :key="i"
            :data-reveal-id="'feat-' + i"
            :class="['group relative rounded-2xl border border-white/[0.06] bg-white/[0.02] p-6 hover:bg-white/[0.04] hover:border-white/[0.1] transition-all duration-300', reveals.get('feat-' + i) ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-6']"
            :style="{ transition: `all 0.5s ease ${0.1 + i * 0.08}s` }"
          >
            <div class="w-9 h-9 rounded-lg bg-emerald-500/10 flex items-center justify-center mb-4 group-hover:bg-emerald-500/20 transition-colors">
              <component :is="f.icon" :size="18" class="text-emerald-400" />
            </div>
            <h3 class="text-[15px] font-semibold text-white mb-2">{{ f.title }}</h3>
            <p class="text-[13px] text-gray-400 leading-relaxed">{{ f.desc }}</p>
          </div>
        </div>
      </div>
    </section>

    <!-- ═══════════ Agent Cards ═══════════ -->
    <section id="agents" class="py-24 md:py-32 relative border-t border-white/[0.04]">
      <div class="max-w-6xl mx-auto px-6">
        <div class="text-center mb-16">
          <h2
            data-reveal-id="agents-title"
            :class="['text-3xl sm:text-4xl font-bold tracking-tight mb-4', reveals.get('agents-title') ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-4']"
            style="transition: all 0.6s ease;"
          >
            {{ t("agents.title") }}
          </h2>
          <p
            data-reveal-id="agents-sub"
            :class="['text-[15px] text-gray-400 max-w-xl mx-auto', reveals.get('agents-sub') ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-4']"
            style="transition: all 0.6s ease 0.15s;"
          >
            {{ t("agents.subtitle") }}
          </p>
        </div>

        <div class="grid grid-cols-1 sm:grid-cols-3 gap-5">
          <div
            v-for="(a, i) in agents"
            :key="a.name"
            :data-reveal-id="'agent-' + i"
            :class="['rounded-2xl border border-white/[0.06] bg-white/[0.02] p-6 hover:bg-white/[0.04] transition-all duration-300', reveals.get('agent-' + i) ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-6']"
            :style="{ transition: `all 0.5s ease ${0.15 + i * 0.1}s` }"
          >
            <div class="flex items-center gap-3 mb-3">
              <div class="w-8 h-8 rounded-lg flex items-center justify-center text-xs font-black text-white" :style="{ background: a.color }">
                {{ a.name[0] }}
              </div>
              <div>
                <div class="text-[14px] font-semibold text-white">{{ a.name }}</div>
                <div class="text-[11px] text-gray-500">{{ a.descKey }} {{ lang === 'zh' ? '官方 CLI' : 'Official CLI' }}</div>
              </div>
            </div>
            <div class="flex items-center gap-2 mt-4 px-3 py-2 rounded-lg bg-black/30 border border-white/[0.04]">
              <Terminal :size="12" class="text-gray-500" />
              <code class="text-[12px] text-gray-400 font-mono">{{ a.cmd }} "..."</code>
            </div>
          </div>
        </div>

        <!-- Agent detection flow -->
        <div
          data-reveal-id="detect-flow"
          :class="['mt-12 rounded-2xl border border-white/[0.06] bg-white/[0.02] p-6 md:p-8 flex flex-col md:flex-row items-center gap-4 md:gap-0 justify-between text-[13px] text-gray-400', reveals.get('detect-flow') ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-6']"
          style="transition: all 0.6s ease 0.3s;"
        >
          <template v-for="(step, i) in agentStepsList" :key="i">
            <div class="flex items-center gap-2">
              <div class="w-6 h-6 rounded bg-emerald-500/20 flex items-center justify-center text-[11px] text-emerald-400">{{ i + 1 }}</div>
              <span :class="i === agentStepsList.length - 1 ? 'text-emerald-300' : ''">{{ step }}</span>
            </div>
            <ArrowRight v-if="i < agentStepsList.length - 1" :size="14" class="hidden md:block text-gray-600" />
          </template>
        </div>
      </div>
    </section>

    <!-- ═══════════ Architecture ═══════════ -->
    <section id="architecture" class="py-24 md:py-32 relative border-t border-white/[0.04]">
      <div class="max-w-6xl mx-auto px-6">
        <div class="text-center mb-16">
          <h2
            data-reveal-id="arch-title"
            :class="['text-3xl sm:text-4xl font-bold tracking-tight mb-4', reveals.get('arch-title') ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-4']"
            style="transition: all 0.6s ease;"
          >
            {{ t("architecture.title") }}
          </h2>
          <p
            data-reveal-id="arch-sub"
            :class="['text-[15px] text-gray-400 max-w-xl mx-auto', reveals.get('arch-sub') ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-4']"
            style="transition: all 0.6s ease 0.15s;"
          >
            {{ t("architecture.subtitle") }}
          </p>
        </div>

        <div class="grid grid-cols-1 md:grid-cols-3 gap-5 mb-12">
          <div
            v-for="(tech, i) in archCards"
            :key="tech.title"
            :data-reveal-id="'tech-' + i"
            :class="['rounded-2xl border border-white/[0.06] bg-white/[0.02] p-6 hover:bg-white/[0.04] transition-all duration-300', reveals.get('tech-' + i) ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-6']"
            :style="{ transition: `all 0.5s ease ${0.1 + i * 0.1}s` }"
          >
            <div class="w-9 h-9 rounded-lg flex items-center justify-center mb-4" :style="{ background: tech.color + '18' }">
              <component :is="tech.icon" :size="18" :style="{ color: tech.color }" />
            </div>
            <h3 class="text-[15px] font-semibold text-white mb-2">{{ tech.title }}</h3>
            <p class="text-[13px] text-gray-400 leading-relaxed">{{ tech.desc }}</p>
          </div>
        </div>

        <!-- Architecture diagram -->
        <div
          data-reveal-id="arch-diag"
          :class="['rounded-2xl border border-white/[0.06] bg-white/[0.02] p-6 md:p-8', reveals.get('arch-diag') ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-8']"
          style="transition: all 0.7s ease 0.3s;"
        >
          <div class="flex flex-col md:flex-row items-center gap-4 md:gap-8 justify-center">
            <div class="flex-1 text-center p-4 rounded-xl bg-emerald-500/[0.06] border border-emerald-500/10 min-w-[140px]">
              <div class="text-[11px] text-emerald-400/70 mb-1 font-mono">{{ t("architecture.diagram.frontend") }}</div>
              <div class="text-white text-sm font-semibold">{{ t("architecture.diagram.frontendTech") }}</div>
              <div class="text-gray-500 text-[11px] mt-0.5">{{ t("architecture.diagram.frontendComp") }}</div>
            </div>
            <ArrowRight :size="16" class="hidden md:block text-gray-600 rotate-90 md:rotate-0" />
            <div class="flex-1 text-center p-4 rounded-xl bg-amber-500/[0.06] border border-amber-500/10 min-w-[140px]">
              <div class="text-[11px] text-amber-400/70 mb-1 font-mono">{{ t("architecture.diagram.core") }}</div>
              <div class="text-white text-sm font-semibold">{{ t("architecture.diagram.coreTech") }}</div>
              <div class="text-gray-500 text-[11px] mt-0.5">{{ t("architecture.diagram.coreComp") }}</div>
            </div>
            <ArrowRight :size="16" class="hidden md:block text-gray-600 rotate-90 md:rotate-0" />
            <div class="flex-1 text-center p-4 rounded-xl bg-indigo-500/[0.06] border border-indigo-500/10 min-w-[140px]">
              <div class="text-[11px] text-indigo-400/70 mb-1 font-mono">{{ t("architecture.diagram.agent") }}</div>
              <div class="text-white text-sm font-semibold">{{ t("architecture.diagram.agentTech") }}</div>
              <div class="text-gray-500 text-[11px] mt-0.5">{{ t("architecture.diagram.agentComp") }}</div>
            </div>
          </div>
          <div class="mt-4 text-center text-[11px] text-gray-600 font-mono">
            {{ t("architecture.diagram.ipc") }}
          </div>
        </div>
      </div>
    </section>

    <!-- ═══════════ Features Highlights ═══════════ -->
    <section class="py-24 md:py-32 relative border-t border-white/[0.04]">
      <div class="max-w-6xl mx-auto px-6">
        <div class="grid grid-cols-1 md:grid-cols-2 gap-16">
          <!-- Chat UI highlight -->
          <div
            data-reveal-id="hl-chat"
            :class="['space-y-5', reveals.get('hl-chat') ? 'opacity-100 translate-x-0' : 'opacity-0 -translate-x-6']"
            style="transition: all 0.6s ease 0.2s;"
          >
            <div class="flex items-center gap-2 text-emerald-400 text-[13px] font-semibold">
              <MessageSquare :size="14" /> {{ t("highlights.chat.label") }}
            </div>
            <h3 class="text-2xl font-bold text-white" v-html="t('highlights.chat.title')" />
            <p class="text-[14px] text-gray-400 leading-relaxed">
              {{ t("highlights.chat.desc") }}
            </p>
            <ul class="space-y-2.5 text-[13px] text-gray-400">
              <li v-for="(b, i) in highlightChatBullets" :key="i" class="flex items-center gap-2">
                <CheckCircle2 :size="13" class="text-emerald-400" /> {{ b }}
              </li>
            </ul>
          </div>

          <!-- Model hub highlight -->
          <div
            data-reveal-id="hl-model"
            :class="['space-y-5', reveals.get('hl-model') ? 'opacity-100 translate-x-0' : 'opacity-0 translate-x-6']"
            style="transition: all 0.6s ease 0.3s;"
          >
            <div class="flex items-center gap-2 text-indigo-400 text-[13px] font-semibold">
              <Database :size="14" /> {{ t("highlights.model.label") }}
            </div>
            <h3 class="text-2xl font-bold text-white" v-html="t('highlights.model.title')" />
            <p class="text-[14px] text-gray-400 leading-relaxed">
              {{ t("highlights.model.desc") }}
            </p>
            <div class="flex flex-wrap gap-2 pt-2">
              <span
                v-for="p in modelProviders"
                :key="p"
                :class="['px-2.5 py-1 rounded-md text-[12px]', p.startsWith('+') || p.startsWith('+ ') ? 'bg-emerald-500/[0.1] border border-emerald-500/20 text-emerald-400' : 'bg-white/[0.04] border border-white/[0.06] text-gray-300']"
              >
                {{ p }}
              </span>
            </div>
          </div>
        </div>
      </div>
    </section>

    <!-- ═══════════ Comparison ═══════════ -->
    <section id="compare" class="py-24 md:py-32 relative border-t border-white/[0.04]">
      <div class="max-w-6xl mx-auto px-6">
        <div class="text-center mb-16">
          <h2
            data-reveal-id="cmp-title"
            :class="['text-3xl sm:text-4xl font-bold tracking-tight mb-4', reveals.get('cmp-title') ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-4']"
            style="transition: all 0.6s ease;"
          >
            {{ t("compare.title") }}
          </h2>
          <p
            data-reveal-id="cmp-sub"
            :class="['text-[15px] text-gray-400 max-w-xl mx-auto', reveals.get('cmp-sub') ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-4']"
            style="transition: all 0.6s ease 0.15s;"
          >
            {{ t("compare.subtitle") }}
          </p>
        </div>

        <div
          data-reveal-id="cmp-table"
          :class="['rounded-2xl border border-white/[0.06] overflow-hidden', reveals.get('cmp-table') ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-6']"
          style="transition: all 0.6s ease 0.2s;"
        >
          <div class="overflow-x-auto">
            <table class="w-full text-[13px]">
              <thead>
                <tr class="border-b border-white/[0.06] bg-white/[0.02]">
                  <th class="text-left px-5 py-3 text-gray-400 font-medium">{{ t("compare.tableHead.feature") }}</th>
                  <th class="text-left px-5 py-3 text-emerald-400 font-semibold">
                    <span class="flex items-center gap-1.5">RunJam <Code2 :size="12" /></span>
                  </th>
                  <th class="text-left px-5 py-3 text-gray-400 font-medium">AionUI</th>
                  <th class="text-left px-5 py-3 text-gray-400 font-medium">Cursor</th>
                </tr>
              </thead>
              <tbody>
                <tr
                  v-for="row in comparisons"
                  :key="row.feature"
                  class="border-b border-white/[0.03] hover:bg-white/[0.02] transition-colors"
                >
                  <td class="px-5 py-3 text-gray-300">{{ row.feature }}</td>
                  <td class="px-5 py-3 text-white">{{ row.runjam }}</td>
                  <td class="px-5 py-3 text-gray-400">{{ row.aionui }}</td>
                  <td class="px-5 py-3 text-gray-400">{{ row.cursor }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>

        <div
          data-reveal-id="cmp-note"
          :class="['mt-8 rounded-2xl border border-emerald-500/10 bg-emerald-500/[0.03] p-6 text-center', reveals.get('cmp-note') ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-6']"
          style="transition: all 0.6s ease 0.4s;"
        >
          <p class="text-[14px] text-emerald-300/80" v-html="t('compare.note')" />
        </div>
      </div>
    </section>

    <!-- ═══════════ FAQ ═══════════ -->
    <section id="faq" class="py-24 md:py-32 relative border-t border-white/[0.04]">
      <div class="max-w-3xl mx-auto px-6">
        <div class="text-center mb-16">
          <h2
            data-reveal-id="faq-title"
            :class="['text-3xl sm:text-4xl font-bold tracking-tight mb-4', reveals.get('faq-title') ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-4']"
            style="transition: all 0.6s ease;"
          >
            {{ t("faq.title") }}
          </h2>
        </div>

        <div class="space-y-3">
          <div
            v-for="(f, i) in faqs"
            :key="i"
            :data-reveal-id="'faq-' + i"
            :class="['rounded-xl border transition-colors duration-300 overflow-hidden', reveals.get('faq-' + i) ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-4']"
            :style="{ transition: `all 0.5s ease ${0.1 + i * 0.07}s` }"
          >
            <button
              @click="toggleFaq(i)"
              class="w-full flex items-center justify-between px-5 py-4 text-left cursor-pointer"
              :class="faqOpen.has(i) ? 'bg-white/[0.04] border-white/[0.08] text-white' : 'bg-transparent border-white/[0.04] text-gray-300'"
            >
              <span class="text-[14px] font-medium pr-4">{{ f.q }}</span>
              <ChevronDown
                :size="14"
                class="text-gray-500 shrink-0 transition-transform duration-300"
                :class="{ 'rotate-180': faqOpen.has(i) }"
              />
            </button>
            <div
              class="overflow-hidden transition-all duration-300"
              :style="{ maxHeight: faqOpen.has(i) ? '300px' : '0px' }"
            >
              <div class="px-5 pb-5 pt-1 text-[13px] text-gray-400 leading-relaxed">
                {{ f.a }}
              </div>
            </div>
          </div>
        </div>
      </div>
    </section>

    <!-- ═══════════ CTA Footer ═══════════ -->
    <section class="py-24 md:py-32 relative border-t border-white/[0.04]">
      <div class="max-w-3xl mx-auto px-6 text-center">
        <h2
          data-reveal-id="cta-title"
          :class="['text-3xl sm:text-4xl font-bold tracking-tight mb-4', reveals.get('cta-title') ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-4']"
          style="transition: all 0.6s ease;"
        >
          {{ t("footer.title1") }}<br>
          <span class="bg-gradient-to-r from-emerald-400 to-indigo-400 bg-clip-text text-transparent">
            {{ t("footer.title2") }}
          </span>
        </h2>
        <p
          data-reveal-id="cta-sub"
          :class="['text-[15px] text-gray-400 mb-10', reveals.get('cta-sub') ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-4']"
          style="transition: all 0.6s ease 0.15s;"
        >
          {{ t("footer.subtitle") }}
        </p>
        <div
          data-reveal-id="cta-btn"
          :class="['flex items-center justify-center gap-4 flex-wrap', reveals.get('cta-btn') ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-4']"
          style="transition: all 0.6s ease 0.25s;"
        >
          <button
            class="px-8 py-3.5 rounded-xl bg-gradient-to-r from-emerald-500 to-emerald-600 text-white text-[15px] font-semibold hover:from-emerald-400 hover:to-emerald-500 transition-all shadow-xl shadow-emerald-500/25 flex items-center gap-2 cursor-pointer"
          >
            <Download :size="17" />
            {{ t("footer.download") }}
          </button>
          <a
            href="https://github.com"
            target="_blank"
            class="px-8 py-3.5 rounded-xl border border-white/[0.1] bg-white/[0.03] text-white text-[15px] font-semibold hover:bg-white/[0.06] transition-all flex items-center gap-2 cursor-pointer"
          >
            <Github :size="17" />
            {{ t("footer.star") }}
          </a>
        </div>
      </div>

      <div class="max-w-6xl mx-auto px-6 mt-24 pt-8 border-t border-white/[0.04]">
        <div class="flex flex-col md:flex-row items-center justify-between gap-4 text-[12px] text-gray-500">
          <div class="flex items-center gap-2">
            <img src="/runjam-logo.svg" alt="RunJam" class="w-5 h-5 rounded" />
            <span>{{ t("footer.tagline") }}</span>
          </div>
          <div class="flex items-center gap-6">
            <a href="https://github.com" target="_blank" class="hover:text-gray-300 transition-colors">{{ t("nav.github") }}</a>
            <span>{{ t("footer.license") }}</span>
            <span>{{ t("footer.copyright") }}</span>
          </div>
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
html {
  scroll-behavior: smooth;
}
.landing-page ::selection {
  background: rgba(16, 185, 129, 0.25);
  color: white;
}
@keyframes shimmer {
  0%, 100% { background-position: 0% 50%; }
  50% { background-position: 100% 50%; }
}
.landing-page {
  overflow-x: hidden;
}
</style>
