<script setup lang="ts">
import { onMounted, ref } from "vue";
import { t } from "../../i18n";
import { Upload, Trash2, Sparkles, Package, ExternalLink } from "lucide-vue-next";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  listSkills,
  listUserSkills,
  installSkillZip,
  removeUserSkill,
  type SkillInfo,
} from "../../api/sessions";

const builtinSkills = ref<SkillInfo[]>([]);
const userSkills = ref<SkillInfo[]>([]);
const loading = ref(true);
const uploading = ref(false);
const message = ref<{ type: "ok" | "err"; text: string } | null>(null);
const fileInput = ref<HTMLInputElement | null>(null);
const confirmRemove = ref<string | null>(null);
let confirmTimer: ReturnType<typeof setTimeout> | null = null;

async function refresh() {
  loading.value = true;
  try {
    const [all, user] = await Promise.all([listSkills(), listUserSkills()]);
    const userNames = new Set(user.map((s) => s.name));
    // `list_skills` merges user skills over builtins — hide the overridden ones.
    builtinSkills.value = all.filter((s) => !userNames.has(s.name));
    userSkills.value = user;
  } catch (e) {
    console.error("[skills] load failed", e);
  } finally {
    loading.value = false;
  }
}

onMounted(refresh);

function onFileChange(e: Event) {
  const input = e.target as HTMLInputElement;
  const file = input.files?.[0];
  if (file) uploadZip(file);
  input.value = ""; // allow re-selecting the same file later
}

function uploadZip(file: File) {
  uploading.value = true;
  message.value = null;
  const reader = new FileReader();
  reader.onload = async () => {
    try {
      const dataUrl = reader.result as string;
      const base64 = dataUrl.slice(dataUrl.indexOf(",") + 1);
      const installed = await installSkillZip(base64);
      await refresh();
      message.value = {
        type: "ok",
        text:
          t("skills.installSuccess", { count: installed.length }) +
          ": " +
          installed.map((s) => s.name).join(", "),
      };
    } catch (err) {
      message.value = { type: "err", text: String(err) };
    } finally {
      uploading.value = false;
    }
  };
  reader.onerror = () => {
    uploading.value = false;
    message.value = { type: "err", text: t("skills.uploadFailed") };
  };
  reader.readAsDataURL(file);
}

function onRemove(name: string) {
  if (confirmRemove.value === name) {
    confirmRemove.value = null;
    if (confirmTimer) clearTimeout(confirmTimer);
    removeUserSkill(name)
      .then(refresh)
      .catch((e) => (message.value = { type: "err", text: String(e) }));
    return;
  }
  confirmRemove.value = name;
  if (confirmTimer) clearTimeout(confirmTimer);
  confirmTimer = setTimeout(() => {
    if (confirmRemove.value === name) confirmRemove.value = null;
  }, 3000);
}

function openSkillsSite() {
  openUrl("https://skillsmp.com/skills").catch(console.error);
}
</script>

<template>
  <div class="p-6 flex justify-center">
    <div class="max-w-4xl w-full">
      <!-- Header -->
      <div class="flex items-center justify-between mb-6">
        <div>
          <h2 class="text-[18px] font-semibold text-gray-900 tracking-tight">{{ t("skills.title") }}</h2>
          <p class="text-[13px] text-gray-500 mt-0.5">{{ t("skills.subtitle") }}</p>
        </div>
      </div>

      <!-- Upload + download hint -->
      <div class="grid grid-cols-1 gap-4 mb-8">
        <div class="bg-white rounded-xl border border-gray-100 p-5">
          <div class="flex items-center gap-2 mb-3">
            <Upload :size="16" class="text-blue-500" />
            <h3 class="text-[14px] font-medium text-gray-700">{{ t("skills.upload") }}</h3>
          </div>
          <p class="text-[12px] text-gray-400 mb-4">{{ t("skills.uploadHint") }}</p>
          <div class="flex items-center gap-3">
            <input
              ref="fileInput"
              type="file"
              accept=".zip,application/zip"
              class="hidden"
              @change="onFileChange"
            />
            <button
              @click="fileInput?.click()"
              :disabled="uploading"
              class="inline-flex items-center gap-2 px-4 py-2 rounded-lg bg-gray-900 text-white text-[13px] font-medium hover:bg-gray-800 transition-colors disabled:opacity-60 disabled:cursor-not-allowed cursor-pointer dark:bg-zinc-700 dark:hover:bg-zinc-800 dark:text-white"
            >
              <Upload :size="14" />
              {{ uploading ? t("skills.uploading") : t("skills.chooseZip") }}
            </button>
            <button
              @click="openSkillsSite"
              class="inline-flex items-center gap-2 px-4 py-2 rounded-lg border border-gray-200 text-gray-600 text-[13px] font-medium hover:bg-gray-50 hover:text-gray-800 transition-colors cursor-pointer dark:border-zinc-700 dark:text-zinc-300 dark:hover:bg-zinc-800 dark:hover:text-zinc-100"
            >
              <ExternalLink :size="13" />
              {{ t("skills.getMore") }} · skillsmp.com
            </button>
          </div>
          <p v-if="message" class="mt-3 text-[12px]" :class="message.type === 'ok' ? 'text-emerald-600' : 'text-red-500'">
            {{ message.text }}
          </p>
        </div>
      </div>

      <!-- Installed (user) skills -->
      <div class="mb-8">
        <div class="flex items-center gap-2 mb-3">
          <Package :size="15" class="text-violet-500" />
          <h3 class="text-[14px] font-medium text-gray-700">{{ t("skills.installed") }}</h3>
          <span class="text-[12px] text-gray-400 ml-1">{{ t("skills.installedHint") }}</span>
        </div>
        <div v-if="loading" class="p-8 text-center text-[13px] text-gray-400">{{ t("skills.loading") }}</div>
        <div v-else-if="userSkills.length === 0" class="bg-white rounded-xl border border-dashed border-gray-200 p-8 text-center">
          <Sparkles :size="28" class="mx-auto mb-2 text-gray-300" />
          <p class="text-[13px] text-gray-400">{{ t("skills.empty") }}</p>
        </div>
        <div v-else class="grid grid-cols-1 sm:grid-cols-2 gap-3">
          <div
            v-for="skill in userSkills"
            :key="skill.name"
            class="bg-white rounded-xl border border-gray-100 p-4 group hover:border-violet-200 hover:shadow-sm transition-all"
          >
            <div class="flex items-start justify-between gap-2 mb-1">
              <div class="text-[14px] font-medium text-gray-900 truncate">{{ skill.name }}</div>
              <button
                @click="onRemove(skill.name)"
                :class="[
                  'flex-shrink-0 inline-flex items-center gap-1 px-2 py-1 rounded-md text-[11px] transition-colors cursor-pointer',
                  confirmRemove === skill.name
                    ? 'bg-red-500 text-white hover:bg-red-600'
                    : 'text-gray-300 hover:text-red-500 hover:bg-red-50',
                ]"
              >
                <Trash2 :size="11" />
                {{ confirmRemove === skill.name ? t("skills.confirmRemove") : t("skills.remove") }}
              </button>
            </div>
            <p class="text-[12px] text-gray-500 leading-snug line-clamp-3">{{ skill.description || t("skills.noDesc") }}</p>
          </div>
        </div>
      </div>

      <!-- Built-in skills -->
      <div>
        <div class="flex items-center gap-2 mb-3">
          <Sparkles :size="15" class="text-indigo-500" />
          <h3 class="text-[14px] font-medium text-gray-700">{{ t("skills.builtin") }}</h3>
          <span class="text-[12px] text-gray-400 ml-1">{{ t("skills.builtinHint") }}</span>
        </div>
        <div v-if="loading" class="p-8 text-center text-[13px] text-gray-400">{{ t("skills.loading") }}</div>
        <div v-else class="grid grid-cols-1 sm:grid-cols-2 gap-3">
          <div
            v-for="skill in builtinSkills"
            :key="skill.name"
            class="bg-white rounded-xl border border-gray-100 p-4 hover:border-indigo-200 hover:shadow-sm transition-all"
          >
            <div class="text-[14px] font-medium text-gray-900 mb-1">{{ skill.name }}</div>
            <p class="text-[12px] text-gray-500 leading-snug line-clamp-3">{{ skill.description || t("skills.noDesc") }}</p>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
