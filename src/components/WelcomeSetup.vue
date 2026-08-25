<script setup lang="ts">
import { ref } from "vue";
import { Sun, Moon, Check } from "lucide-vue-next";
import { useThemeStore } from "@/stores/useThemeStore";
import { currentLocale, setLocale, type Locale } from "@/i18n";

const emit = defineEmits<{ (e: "done"): void }>();

const themeStore = useThemeStore();
const theme = ref<"light" | "dark">(themeStore.theme);
const locale = ref<Locale>(currentLocale());

function pickTheme(mode: "light" | "dark") {
  theme.value = mode;
  themeStore.setTheme(mode);
}

function pickLocale(l: Locale) {
  locale.value = l;
  setLocale(l);
}

function finish() {
  try {
    localStorage.setItem("runjam.setupDone", "1");
  } catch {
    // ignore storage errors
  }
  emit("done");
}
</script>

<template>
  <div
    class="fixed inset-0 z-[100000] flex items-center justify-center overflow-y-auto bg-white px-6 py-10 dark:bg-[#101015]"
  >
    <div class="w-full max-w-xl">
      <!-- Logo + heading -->
      <div class="flex flex-col items-center text-center">
        <img src="/runjam-logo.svg" alt="RunJam" class="h-14 w-14" />
        <h1 class="mt-5 text-[26px] font-semibold tracking-tight text-gray-900 dark:text-[#ececf4]">
          {{ $t("setup.title") }}
        </h1>
        <p class="mt-2 text-[14px] text-gray-500 dark:text-[#9a9aa6]">
          {{ $t("setup.subtitle") }}
        </p>
      </div>

      <!-- Theme -->
      <div class="mt-8">
        <p class="text-[13px] font-semibold text-gray-700 dark:text-[#c9c9d4]">
          {{ $t("setup.theme") }}
        </p>
        <div class="mt-3 grid grid-cols-2 gap-3">
          <button
            type="button"
            @click="pickTheme('light')"
            class="flex items-center gap-3 rounded-xl border-2 p-3.5 text-left transition-all"
            :class="theme === 'light'
              ? 'border-blue-500 bg-blue-50/60 dark:border-blue-400 dark:bg-blue-500/10'
              : 'border-gray-200 bg-gray-50 hover:border-gray-300 dark:border-[#2a2a33] dark:bg-[#16161c] dark:hover:border-[#3a3a45]'"
          >
            <Sun :size="18" class="shrink-0 text-amber-500" />
            <span class="min-w-0 flex-1">
              <span class="block text-[14px] font-medium text-gray-900 dark:text-[#ececf4]">{{ $t("setup.light") }}</span>
              <span class="block truncate text-[12px] text-gray-500 dark:text-[#9a9aa6]">{{ $t("setup.lightDesc") }}</span>
            </span>
            <Check v-if="theme === 'light'" :size="16" class="shrink-0 text-blue-600 dark:text-blue-400" />
          </button>
          <button
            type="button"
            @click="pickTheme('dark')"
            class="flex items-center gap-3 rounded-xl border-2 p-3.5 text-left transition-all"
            :class="theme === 'dark'
              ? 'border-blue-500 bg-blue-50/60 dark:border-blue-400 dark:bg-blue-500/10'
              : 'border-gray-200 bg-gray-50 hover:border-gray-300 dark:border-[#2a2a33] dark:bg-[#16161c] dark:hover:border-[#3a3a45]'"
          >
            <Moon :size="18" class="shrink-0 text-indigo-400" />
            <span class="min-w-0 flex-1">
              <span class="block text-[14px] font-medium text-gray-900 dark:text-[#ececf4]">{{ $t("setup.dark") }}</span>
              <span class="block truncate text-[12px] text-gray-500 dark:text-[#9a9aa6]">{{ $t("setup.darkDesc") }}</span>
            </span>
            <Check v-if="theme === 'dark'" :size="16" class="shrink-0 text-blue-600 dark:text-blue-400" />
          </button>
        </div>
      </div>

      <!-- Language -->
      <div class="mt-6">
        <p class="text-[13px] font-semibold text-gray-700 dark:text-[#c9c9d4]">
          {{ $t("setup.language") }}
        </p>
        <div class="mt-3 grid grid-cols-2 gap-3">
          <button
            type="button"
            @click="pickLocale('en-US')"
            class="flex items-center gap-3 rounded-xl border-2 p-3.5 text-left transition-all"
            :class="locale === 'en-US'
              ? 'border-blue-500 bg-blue-50/60 dark:border-blue-400 dark:bg-blue-500/10'
              : 'border-gray-200 bg-gray-50 hover:border-gray-300 dark:border-[#2a2a33] dark:bg-[#16161c] dark:hover:border-[#3a3a45]'"
          >
            <span class="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-blue-100 text-[12px] font-semibold text-blue-700 dark:bg-blue-500/20 dark:text-blue-300">EN</span>
            <span class="block text-[14px] font-medium text-gray-900 dark:text-[#ececf4]">English</span>
            <span class="ml-auto"><Check v-if="locale === 'en-US'" :size="16" class="shrink-0 text-blue-600 dark:text-blue-400" /></span>
          </button>
          <button
            type="button"
            @click="pickLocale('zh-CN')"
            class="flex items-center gap-3 rounded-xl border-2 p-3.5 text-left transition-all"
            :class="locale === 'zh-CN'
              ? 'border-blue-500 bg-blue-50/60 dark:border-blue-400 dark:bg-blue-500/10'
              : 'border-gray-200 bg-gray-50 hover:border-gray-300 dark:border-[#2a2a33] dark:bg-[#16161c] dark:hover:border-[#3a3a45]'"
          >
            <span class="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-red-100 text-[12px] font-semibold text-red-700 dark:bg-red-500/20 dark:text-red-300">中</span>
            <span class="block text-[14px] font-medium text-gray-900 dark:text-[#ececf4]">简体中文</span>
            <span class="ml-auto"><Check v-if="locale === 'zh-CN'" :size="16" class="shrink-0 text-blue-600 dark:text-blue-400" /></span>
          </button>
        </div>
      </div>

      <!-- CTA -->
      <button
        type="button"
        @click="finish"
        class="mt-8 w-full rounded-xl bg-blue-600 py-3 text-[14px] font-semibold text-white transition-colors hover:bg-blue-700"
      >
        {{ $t("setup.start") }}
      </button>

      <p class="mt-4 text-center text-[12px] text-gray-400 dark:text-[#6f6f7d]">
        {{ $t("setup.changable") }}
      </p>
    </div>
  </div>
</template>
