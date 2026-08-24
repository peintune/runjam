<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref, nextTick } from "vue";
import {
  FilePlus, FolderPlus, Pencil, Move, ExternalLink, Trash2,
} from "lucide-vue-next";

const props = defineProps<{
  x: number;
  y: number;
  isDir: boolean;
  /** Root mode: only show the "New File / New Folder" create actions. Used for
   *  right-clicking the empty area of the tree (the workspace root isn't a
   *  renatable/movable/deletable entry). */
  rootMode?: boolean;
}>();

const emit = defineEmits<{
  (e: "new-file"): void;
  (e: "new-folder"): void;
  (e: "rename"): void;
  (e: "move-to"): void;
  (e: "open-in-finder"): void;
  (e: "delete"): void;
  (e: "close"): void;
}>();

const menuRef = ref<HTMLDivElement | null>(null);
// Adjust position so the menu stays inside the viewport. Without this, a
// right-click near the bottom-right corner of the screen would push the menu
// off-screen and make it unreachable.
const adjustedX = ref(props.x);
const adjustedY = ref(props.y);

function adjustPosition() {
  nextTick(() => {
    const el = menuRef.value;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    if (rect.right > vw) adjustedX.value = Math.max(4, vw - rect.width - 4);
    if (rect.bottom > vh) adjustedY.value = Math.max(4, vh - rect.height - 4);
  });
}

onMounted(adjustPosition);

// Close on outside click / Escape / scroll. The scroll handler prevents the
// menu from "sticking" in place if the user scrolls the tree while it's open.
function onPointerDown(e: PointerEvent) {
  if (menuRef.value && !menuRef.value.contains(e.target as Node)) {
    emit("close");
  }
}
function onKey(e: KeyboardEvent) {
  if (e.key === "Escape") emit("close");
}
function onScroll() {
  emit("close");
}

onMounted(() => {
  document.addEventListener("pointerdown", onPointerDown);
  document.addEventListener("keydown", onKey);
  // Capture so we hear about scrolls on any ancestor, not just the document.
  window.addEventListener("scroll", onScroll, true);
});
onBeforeUnmount(() => {
  document.removeEventListener("pointerdown", onPointerDown);
  document.removeEventListener("keydown", onKey);
  window.removeEventListener("scroll", onScroll, true);
});

function pick(action: () => void) {
  action();
  emit("close");
}
</script>

<template>
  <Teleport to="body">
    <div
      ref="menuRef"
      class="fixed z-50 min-w-[180px] bg-white rounded-lg shadow-lg border border-gray-200 py-1 text-[12px] text-gray-700 select-none animate-in fade-in zoom-in-95 duration-100"
      :style="{ left: adjustedX + 'px', top: adjustedY + 'px' }"
      @contextmenu.prevent
    >
      <!-- Folder-only: create inside this folder (root mode shows only these) -->
      <template v-if="isDir">
        <button
          class="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-gray-50 text-left"
          @click="pick(() => emit('new-file'))"
        >
          <FilePlus :size="13" class="text-gray-500" />
          <span>{{ $t("fs.newFile") }}</span>
        </button>
        <button
          class="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-gray-50 text-left"
          @click="pick(() => emit('new-folder'))"
        >
          <FolderPlus :size="13" class="text-gray-500" />
          <span>{{ $t("fs.newFolder") }}</span>
        </button>
        <div class="my-1 border-t border-gray-100"></div>
      </template>

      <!-- Non-root: rename / move / open / delete actions -->
      <template v-if="!rootMode">
        <button
          class="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-gray-50 text-left"
          @click="pick(() => emit('rename'))"
        >
          <Pencil :size="13" class="text-gray-500" />
          <span>{{ $t("fs.rename") }}</span>
        </button>

        <button
          class="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-gray-50 text-left"
          @click="pick(() => emit('move-to'))"
        >
          <Move :size="13" class="text-gray-500" />
          <span>{{ $t("fs.moveTo") }}</span>
        </button>

        <!-- Open in Finder / Reveal in Finder: for folders it opens the folder;
             for files it reveals the file in its parent folder. -->
        <button
          class="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-gray-50 text-left"
          @click="pick(() => emit('open-in-finder'))"
        >
          <ExternalLink :size="13" class="text-gray-500" />
          <span>{{ $t("fs.openInFinder") }}</span>
        </button>

        <div class="my-1 border-t border-gray-100"></div>

        <button
          class="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-red-50 text-red-600 text-left"
          @click="pick(() => emit('delete'))"
        >
          <Trash2 :size="13" />
          <span>{{ $t("common.delete") }}</span>
        </button>
      </template>
    </div>
  </Teleport>
</template>
