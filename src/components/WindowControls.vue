<script setup lang="ts">
/**
 * Window control buttons (minimize / maximize / close).
 *
 * Only rendered on platforms without a native title bar — macOS keeps the
 * system traffic lights (titleBarStyle: Overlay), so these are hidden there.
 * On Windows/Linux (decorations: false) they replace the missing native
 * buttons. Requires capabilities core:window:allow-minimize /
 * allow-toggle-maximize / allow-close.
 */
import { Minus, Square, X } from "lucide-vue-next";
import { getCurrentWindow } from "@tauri-apps/api/window";

const isMac =
  typeof navigator !== "undefined" && /mac/i.test(navigator.platform || navigator.userAgent);

const appWindow = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window
  ? getCurrentWindow()
  : null;

function minimize() {
  appWindow?.minimize();
}
function toggleMaximize() {
  appWindow?.toggleMaximize();
}
function close() {
  appWindow?.close();
}
</script>

<template>
  <div v-if="!isMac" class="flex items-center h-8 -my-[5px] mr-[-16px]" style="-webkit-app-region: no-drag">
    <button
      class="w-11 h-8 flex items-center justify-center text-gray-400 hover:text-gray-700 hover:bg-gray-200/70 transition-colors duration-100 cursor-pointer"
      @click="minimize"
      title="Minimize"
    >
      <Minus :size="14" />
    </button>
    <button
      class="w-11 h-8 flex items-center justify-center text-gray-400 hover:text-gray-700 hover:bg-gray-200/70 transition-colors duration-100 cursor-pointer"
      @click="toggleMaximize"
      title="Maximize"
    >
      <Square :size="11" />
    </button>
    <button
      class="w-11 h-8 flex items-center justify-center text-gray-400 hover:text-red-500 hover:bg-red-500/10 transition-colors duration-100 cursor-pointer"
      @click="close"
      title="Close"
    >
      <X :size="16" />
    </button>
  </div>
</template>
