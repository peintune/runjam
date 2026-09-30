<script setup lang="ts">
/**
 * Desktop pet settings — its own first-level page in the settings sidebar
 * (rather than a row buried inside General) because it is a user-visible
 * surface with its own behaviour, not an app-level preference.
 */
import { ref, onMounted } from "vue";
import { Sparkles, Moon } from "lucide-vue-next";
import { petEnabled, setPetEnabled } from "@/api/app";
import {
  loadPreventSleepPref,
  savePreventSleepPref,
} from "@/composables/usePreventSleep";

const petOn = ref(true);
const preventSleep = ref(loadPreventSleepPref());

onMounted(async () => {
  try {
    petOn.value = await petEnabled();
  } catch {
    // UI running without the pet backend (e.g. older build)
  }
});

async function togglePet() {
  const next = !petOn.value;
  petOn.value = next;
  try {
    await setPetEnabled(next);
  } catch (e) {
    console.error("Failed to update desktop pet setting:", e);
    petOn.value = !next;
  }
}

/**
 * Toggle the sleep guard. Persisting the preference is enough: the live watcher
 * in `usePreventSleep` (owned by App.vue) is notified and applies the assertion,
 * so the settings page never talks to the power backend directly.
 */
function togglePreventSleep() {
  const next = !preventSleep.value;
  preventSleep.value = next;
  savePreventSleepPref(next);
}
</script>

<template>
  <div class="p-6 flex justify-center">
    <div class="max-w-2xl w-full">
      <h2 class="text-[18px] font-semibold text-gray-900 tracking-tight mb-6">
        {{ $t("settings.pet.title") }}
      </h2>

      <div class="bg-white rounded-xl border border-gray-100 divide-y divide-gray-100">
        <div class="flex items-center justify-between px-5 py-4">
          <div class="pr-4 min-w-0">
            <div class="flex items-center gap-2">
              <Sparkles :size="14" class="text-indigo-500 shrink-0" />
              <p class="text-[14px] font-medium text-gray-900">{{ $t("settings.general.pet") }}</p>
            </div>
            <p class="text-[12px] text-gray-400 mt-0.5">
              {{ $t("settings.general.petDesc") }}
            </p>
          </div>
          <button
            role="switch"
            :aria-checked="petOn"
            :title="$t('settings.general.pet')"
            class="relative h-6 w-11 shrink-0 rounded-full transition-colors disabled:opacity-50 cursor-pointer"
            :class="petOn ? 'bg-indigo-600' : 'bg-gray-300'"
            @click="togglePet"
          >
            <span
              class="absolute top-0.5 h-5 w-5 rounded-full bg-white shadow transition-all dark:bg-white"
              :class="petOn ? 'left-[22px]' : 'left-0.5'"
            />
          </button>
        </div>

        <!-- Usage hints: the pet is easy to miss without knowing where it lives
             and that it can be moved. -->
        <div class="px-5 py-4">
          <p class="text-[12px] text-gray-400 leading-relaxed">
            {{ $t("settings.pet.hint") }}
          </p>
        </div>

        <div class="flex items-center justify-between px-5 py-4">
          <div class="pr-4 min-w-0">
            <div class="flex items-center gap-2">
              <Moon :size="14" class="text-indigo-500 shrink-0" />
              <p class="text-[14px] font-medium text-gray-900">{{ $t("settings.pet.preventSleep") }}</p>
            </div>
            <p class="text-[12px] text-gray-400 mt-0.5">
              {{ $t("settings.pet.preventSleepDesc") }}
            </p>
          </div>
          <button
            role="switch"
            :aria-checked="preventSleep"
            :title="$t('settings.pet.preventSleep')"
            class="relative h-6 w-11 shrink-0 rounded-full transition-colors disabled:opacity-50 cursor-pointer"
            :class="preventSleep ? 'bg-indigo-600' : 'bg-gray-300'"
            @click="togglePreventSleep"
          >
            <span
              class="absolute top-0.5 h-5 w-5 rounded-full bg-white shadow transition-all dark:bg-white"
              :class="preventSleep ? 'left-[22px]' : 'left-0.5'"
            />
          </button>
        </div>
      </div>
    </div>
  </div>
</template>