<script setup lang="ts">
/**
 * Desktop-pet launcher icon — the small circular badge that floats in the
 * bottom-right corner of the screen, above every other application.
 *
 * This window is intentionally *tiny* (56x56) and mostly transparent, so it
 * mounts its own minimal component instead of the full app shell (see
 * `main.ts` for the window-label routing). Clicking it toggles the Q&A popup;
 * dragging it repositions the window via the OS.
 */
import { onMounted, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Loader2 } from "lucide-vue-next";

const busy = ref(false);

/**
 * Distance (px) the pointer must travel before a press counts as a drag rather
 * than a click. Tauri's `data-tauri-drag-region` deliberately ignores presses
 * on `<button>` elements (see `tauri-2.11.5/src/window/scripts/drag.js`), so the
 * button drives the drag itself — this threshold separates a drag from a click.
 */
const DRAG_THRESHOLD = 4;

/** Some platforms still deliver a `click` after the OS drag loop ends. Ignore
 *  any click landing this close to a completed drag. */
const POST_DRAG_CLICK_MS = 300;

let pressed = false;
let draggedAt = 0;
let startX = 0;
let startY = 0;

function onMouseDown(e: MouseEvent) {
  if (e.button !== 0) return;
  pressed = true;
  startX = e.clientX;
  startY = e.clientY;
}

function onMouseMove(e: MouseEvent) {
  if (!pressed) return;
  if (Math.hypot(e.clientX - startX, e.clientY - startY) < DRAG_THRESHOLD) return;
  // Hand the press over to the OS drag loop, which runs until release.
  pressed = false;
  draggedAt = Date.now();
  getCurrentWindow().startDragging().catch((err) => {
    console.error("[pet] failed to start drag:", err);
  });
}

function onMouseUp() {
  pressed = false;
}

onMounted(() => {
  window.addEventListener("mousemove", onMouseMove);
  window.addEventListener("mouseup", onMouseUp);
});

onUnmounted(() => {
  window.removeEventListener("mousemove", onMouseMove);
  window.removeEventListener("mouseup", onMouseUp);
});

/** Toggle the chat popup: a second click on the badge closes it again. */
async function toggleChat() {
  if (busy.value) return;
  if (Date.now() - draggedAt < POST_DRAG_CLICK_MS) return; // tail of a drag
  busy.value = true;
  try {
    await invoke("toggle_pet_chat");
  } catch (err) {
    console.error("[pet] failed to toggle chat:", err);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <!--
    The whole window is transparent; only the logo is painted. The wrapper keeps
    `data-tauri-drag-region` so the thin transparent rim around the logo stays
    draggable too.

    The logo is a rounded SQUARE, so the button is square too — clipping it to a
    circle would show the transparent window corners through as gaps.
  -->
  <div
    class="w-screen h-screen flex items-center justify-center select-none"
    data-tauri-drag-region
  >
    <button
      type="button"
      class="w-9 h-9 rounded-xl overflow-hidden flex items-center justify-center cursor-pointer
             shadow-lg transition-transform duration-150 hover:scale-110 active:scale-95
             focus:outline-none"
      title="RunJam"
      @mousedown="onMouseDown"
      @click="toggleChat"
    >
      <Loader2 v-if="busy" :size="16" class="animate-spin text-indigo-500" />
      <img v-else src="/runjam-logo.svg" alt="RunJam" class="w-9 h-9 block" draggable="false" />
    </button>
  </div>
</template>

<style scoped>
/* Transparent window background — the rounded button is the only visible part,
   so macOS can render it as a floating circle without a rectangular card. */
:global(html),
:global(body),
:global(#app) {
  background: transparent !important;
}
</style>