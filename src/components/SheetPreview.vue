<script setup lang="ts">
// Renders a spreadsheet (.xlsx/.xls/.csv/.ods) as a table via SheetJS.
//
// SheetJS parses the workbook in the frontend, so no conversion round trip is
// needed for the most common Office formats. Multiple sheets become tabs; a
// single-sheet file shows just its table.
//
// The values are read as FORMATTED text (`raw: false`) rather than their
// underlying values: a cell holding 0.075 or a date serial should preview the way
// the spreadsheet shows it, not as the raw number a formula consumer would see.
import { ref, watch, computed } from "vue";
import { readFileBytes } from "../api/fs";
import { Loader, FileWarning, ExternalLink } from "lucide-vue-next";
import { openWithSystemApp } from "../api/office";

const props = defineProps<{ filePath: string }>();

const loading = ref(true);
const error = ref("");
const fileName = ref("");
const sheetNames = ref<string[]>([]);
const activeSheet = ref(0);
/** Rows of formatted cell strings, per sheet. */
const sheets = ref<string[][][]>([]);

const maxCols = computed(() => {
  const rows = sheets.value[activeSheet.value] || [];
  return rows.reduce((m, r) => Math.max(m, r.length), 0);
});

type XlsxLike = {
  read(data: Uint8Array, opts: { type: string }): {
    SheetNames: string[];
    Sheets: Record<string, unknown>;
  };
  utils: {
    sheet_to_json(sheet: unknown, opts: { header: 1; raw: false; defval: string }): unknown[][];
  };
};

let xlsxPromise: Promise<XlsxLike | null> | null = null;
function loadXlsx(): Promise<XlsxLike | null> {
  if (!xlsxPromise) {
    xlsxPromise = import("xlsx")
      .then((m) => (m as unknown as XlsxLike))
      .catch((e) => {
        console.error("Failed to load xlsx:", e);
        return null;
      });
  }
  return xlsxPromise;
}

async function load() {
  if (!props.filePath) return;
  loading.value = true;
  error.value = "";
  sheetNames.value = [];
  sheets.value = [];
  activeSheet.value = 0;
  fileName.value = props.filePath.split(/[/\\]/).pop() || "";
  try {
    const bytes = await readFileBytes(props.filePath);
    const xlsx = await loadXlsx();
    if (!xlsx) throw new Error("Failed to load the spreadsheet renderer");
    const wb = xlsx.read(new Uint8Array(bytes), { type: "array" });
    const names = wb.SheetNames || [];
    const parsed = names.map((n) => {
      const rows = xlsx.utils.sheet_to_json(wb.Sheets[n], { header: 1, raw: false, defval: "" });
      // SheetJS may leave holes (undefined) in ragged rows; normalise to strings
      // so the template can render every column without a guard.
      return rows.map((r) => (Array.isArray(r) ? r.map((c) => (c == null ? "" : String(c))) : []));
    });
    sheetNames.value = names;
    sheets.value = parsed;
  } catch (err: any) {
    error.value = String(err?.message || err);
  } finally {
    loading.value = false;
  }
}

function openExternal() {
  openWithSystemApp(props.filePath).catch((e) => console.error("Failed to open file:", e));
}

watch(() => props.filePath, load, { immediate: true });
</script>

<template>
  <div class="h-full flex flex-col bg-white">
    <div class="flex items-center justify-between px-3 py-1.5 border-b border-gray-100 flex-shrink-0 bg-gray-50/50">
      <span class="text-[12px] font-medium text-gray-700 truncate">{{ fileName }}</span>
      <button
        class="flex items-center gap-1 px-2 py-0.5 rounded-md text-[11px] text-gray-500 hover:text-indigo-600 hover:bg-indigo-50 transition-colors cursor-pointer"
        :title="$t('editor.openInSystemApp')"
        @click="openExternal"
      >
        <ExternalLink :size="12" />
        {{ $t("editor.openExternal") }}
      </button>
    </div>

    <!-- Sheet tabs, shown only for a multi-sheet workbook. -->
    <div v-if="sheetNames.length > 1" class="flex items-center gap-1 px-2 py-1 border-b border-gray-100 flex-shrink-0 overflow-x-auto">
      <button
        v-for="(name, i) in sheetNames"
        :key="name"
        class="px-2.5 py-1 rounded-md text-[12px] whitespace-nowrap transition-colors cursor-pointer"
        :class="i === activeSheet
          ? 'bg-indigo-50 text-indigo-600 font-medium'
          : 'text-gray-500 hover:bg-gray-100'"
        @click="activeSheet = i"
      >
        {{ name }}
      </button>
    </div>

    <div class="flex-1 min-h-0 overflow-auto bg-[#f8f9fb] dark:bg-[#101015]">
      <div v-if="loading" class="h-full flex items-center justify-center gap-2 text-gray-400">
        <Loader :size="16" class="animate-spin" />
        <span class="text-[13px]">{{ $t("editor.loading") }}</span>
      </div>
      <div v-else-if="error" class="h-full flex flex-col items-center justify-center gap-2 text-gray-400">
        <FileWarning :size="32" class="text-red-300" />
        <span class="text-[13px] text-red-500">{{ error }}</span>
      </div>
      <table v-else class="border-collapse text-[12px] bg-white">
        <tbody>
          <tr v-for="(row, ri) in sheets[activeSheet]" :key="ri">
            <td
              v-for="ci in maxCols"
              :key="ci"
              class="border border-gray-200 px-2.5 py-1 align-top whitespace-nowrap max-w-[420px] overflow-hidden text-ellipsis"
              :class="ri === 0 ? 'bg-gray-50 font-medium text-gray-700 sticky top-0' : 'text-gray-700'"
            >{{ row[ci - 1] ?? "" }}</td>
          </tr>
        </tbody>
      </table>
      <div v-if="!loading && !error && sheets[activeSheet]?.length === 0" class="p-6 text-[13px] text-gray-400">
        {{ $t("editor.emptySheet") }}
      </div>
    </div>
  </div>
</template>