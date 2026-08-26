<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useRouter } from "vue-router";
import { listen } from "@tauri-apps/api/event";
import { useAgentStore } from "./stores/useAgentStore";
import { useWorkspaceStore } from "./stores/useWorkspaceStore";
import { useAppTabsStore } from "./stores/useAppTabsStore";
import { faviconFor, type AppItem } from "./stores/useAppsStore";
import { getAgentStatuses } from "./api/agents";
import { getModels } from "./api/models";
import {
  getTelemetryStatus,
  setTelemetryEnabled,
  checkUpdateUi,
  getAnnouncements,
  markAnnouncementRead,
} from "./api/telemetry";
import type { Announcement, UpdateCheckResult } from "./api/telemetry";
import { useToast } from "./composables/useToast";
import Toast from "./components/Toast.vue";
import UpdatePrompt from "./components/UpdatePrompt.vue";
import AnnouncementBanner from "./components/AnnouncementBanner.vue";
import AnnouncementModal from "./components/AnnouncementModal.vue";
import WelcomeSetup from "./components/WelcomeSetup.vue";

const agentStore = useAgentStore();
const workspaceStore = useWorkspaceStore();
const appTabs = useAppTabsStore();
const router = useRouter();
const { toasts, removeToast } = useToast();

// First-launch setup (theme + language).
const showSetup = ref(false);

// First-launch telemetry consent dialog.
const showConsent = ref(false);
const deciding = ref(false);

onMounted(async () => {
  // Ask for theme/language preferences on the very first launch.
  try {
    if (!localStorage.getItem("runjam.setupDone")) {
      showSetup.value = true;
    }
  } catch {
    // ignore storage errors
  }
  try {
    const status = await getTelemetryStatus();
    // Only ask on first launch; once answered we never show it again.
    if (!status.consentShown) {
      showConsent.value = true;
    }
  } catch {
    // Backend without telemetry support (older builds) — skip silently.
  }

  // Kick off update check + announcement fetch in the background.
  checkUpdates();
  loadAnnouncements();

  // A website inside an app tab asked for a new window/tab (`window.open`,
  // `target=_blank`) — the Rust side forwards the URL here so we can open it
  // as a fresh RunJam tab, browser-style.
  listen<string>("app-tab-new-window", (event) => {
    const raw = event.payload;
    let parsed: URL;
    try {
      parsed = new URL(raw);
    } catch {
      return;
    }
    const item: AppItem = {
      id: "",
      name: parsed.hostname.replace(/^www\./, ""),
      url: parsed.toString(),
      icon: faviconFor(parsed.toString()),
      builtin: false,
    };
    appTabs.openApp(item).catch(console.error);
    if (router.currentRoute.value.path !== "/") {
      router.push("/");
    }
  }).catch(() => {});
});

async function decideConsent(enabled: boolean) {
  if (deciding.value) return;
  deciding.value = true;
  try {
    await setTelemetryEnabled(enabled);
  } catch {
    // Non-fatal: consent dialog just closes.
  } finally {
    deciding.value = false;
    showConsent.value = false;
  }
}

// Update prompt + announcements state.
const updateResult = ref<UpdateCheckResult | null>(null);
const announcements = ref<Announcement[]>([]);
const activeImportant = ref<Announcement | null>(null);

function currentVersion(): string {
  // Read from a global injected at build time; fall back to "0.1.0".
  // Tauri injects __TAURI_INTERNALS__; version comes from the Rust command
  // on the backend side, so here we just pass a placeholder that the Rust
  // command ignores for announcements and uses for the update check.
  return "0.1.0";
}

async function checkUpdates() {
  try {
    const res = await checkUpdateUi(currentVersion());
    if (res.updateAvailable) {
      updateResult.value = res;
    }
  } catch {
    // Non-fatal: skip update prompt on failure.
  }
}

async function loadAnnouncements() {
  try {
    const items = await getAnnouncements();
    announcements.value = items;
    // Show the first important one as a modal; rest as banners.
    const important = items.find((a) => a.level === "important");
    if (important) {
      activeImportant.value = important;
    }
  } catch {
    // Non-fatal: skip announcements on failure.
  }
}

function closeImportant() {
  if (activeImportant.value) {
    markAnnouncementRead(activeImportant.value.id).catch(() => {});
    activeImportant.value = null;
  }
}

function dismissBanner(id: string) {
  markAnnouncementRead(id).catch(() => {});
  announcements.value = announcements.value.filter((a) => a.id !== id);
}

Promise.all([
  (async () => {
    try { agentStore.agents = await getAgentStatuses(); } catch {}
  })(),
  (async () => {
    try { agentStore.models = await getModels(); } catch {}
  })(),
  (async () => {
    try { await workspaceStore.loadSessions(); } catch {}
  })(),
]);
</script>

<template>
  <router-view v-slot="{ Component }">
    <keep-alive include="WorkspaceLayout">
      <component :is="Component" />
    </keep-alive>
  </router-view>

  <!-- First-launch setup (theme + language) -->
  <WelcomeSetup v-if="showSetup" @done="showSetup = false" />

  <!-- First-launch telemetry consent -->
  <div
    v-if="showConsent"
    class="fixed inset-0 z-[99999] flex items-center justify-center bg-black/40 px-6"
  >
    <div class="w-full max-w-md rounded-2xl bg-white p-6 shadow-2xl">
      <h2 class="text-[18px] font-semibold text-gray-900 tracking-tight">{{ $t("app.consent.title") }}</h2>
      <p class="mt-3 text-[13px] leading-relaxed text-gray-500">
        {{ $t("app.consent.body") }}<span class="font-medium text-gray-700"> {{ $t("app.consent.noCode") }}</span>
        {{ $t("app.consent.disableAt") }}
      </p>
      <div class="mt-6 flex items-center justify-end gap-3">
        <button
          class="rounded-md px-4 py-2 text-[13px] font-medium text-gray-500 hover:text-gray-700 transition-colors"
          :disabled="deciding"
          @click="decideConsent(false)"
        >
          {{ $t("app.consent.decline") }}
        </button>
        <button
          class="rounded-md bg-blue-600 px-4 py-2 text-[13px] font-medium text-white hover:bg-blue-700 transition-colors disabled:opacity-50"
          :disabled="deciding"
          @click="decideConsent(true)"
        >
          {{ $t("app.consent.accept") }}
        </button>
      </div>
    </div>
  </div>

  <!-- Update prompt -->
  <UpdatePrompt
    v-if="updateResult"
    :result="updateResult"
    @close="updateResult = null"
  />

  <!-- Important announcement modal -->
  <AnnouncementModal
    v-if="activeImportant"
    :announcement="activeImportant"
    @close="closeImportant"
  />

  <!-- Info announcement banners (above the toast stack) -->
  <div class="fixed top-4 right-4 z-[9998] flex flex-col gap-2 w-80">
    <AnnouncementBanner
      v-for="a in announcements.filter((x) => x.id !== activeImportant?.id)"
      :key="a.id"
      :announcement="a"
      @close="dismissBanner(a.id)"
    />
  </div>

  <div class="fixed top-4 right-4 z-[9999] flex flex-col gap-2 w-80">
    <Toast
      v-for="toast in toasts"
      :key="toast.id"
      :toast="toast"
      @remove="removeToast"
    />
  </div>
</template>
