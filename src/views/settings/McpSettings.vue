<script setup lang="ts">
/**
 * MCP server settings.
 *
 * RunJam keeps its OWN list of MCP servers and injects it into every session.
 * That is the point of the page: each agent normally reads MCP servers from its
 * own config location (Claude Code from `~/.claude.json`, Codex and Gemini from
 * theirs), so the same server had to be declared once per agent. Defining it here
 * makes it available to all of them.
 */
import { ref, onMounted } from "vue";
import { Plus, Trash2, Boxes, Pencil, Check } from "lucide-vue-next";
import {
  listMcpServers,
  saveMcpServer,
  deleteMcpServer,
  emptyMcpServer,
  type McpServer,
  type McpTransport,
} from "@/api/mcp";
import { useToast } from "@/composables/useToast";

const { showWarning } = useToast();

const servers = ref<McpServer[]>([]);
const loading = ref(true);
const editing = ref<McpServer | null>(null);
const saving = ref(false);

async function load() {
  loading.value = true;
  try {
    servers.value = await listMcpServers();
  } catch (e) {
    console.error("Failed to load MCP servers:", e);
    servers.value = [];
  } finally {
    loading.value = false;
  }
}

onMounted(load);

function startAdd() {
  editing.value = emptyMcpServer();
}

function startEdit(s: McpServer) {
  // Deep copy so cancelling does not mutate the listed row.
  editing.value = JSON.parse(JSON.stringify(s));
}

function cancelEdit() {
  editing.value = null;
}

/** Only the fields the chosen transport actually uses are sent. */
function setTransport(t: McpTransport) {
  if (editing.value) editing.value.transport = t;
}

async function save() {
  const draft = editing.value;
  if (!draft) return;
  if (!draft.name.trim()) {
    showWarning("Name is required");
    return;
  }
  saving.value = true;
  try {
    await saveMcpServer(draft);
    editing.value = null;
    await load();
  } catch (e) {
    showWarning(String(e));
  } finally {
    saving.value = false;
  }
}

async function remove(s: McpServer) {
  try {
    await deleteMcpServer(s.id);
    await load();
  } catch (e) {
    showWarning(String(e));
  }
}

/** Toggle availability without opening the editor. */
async function toggleEnabled(s: McpServer) {
  try {
    await saveMcpServer({ ...s, enabled: !s.enabled });
    await load();
  } catch (e) {
    showWarning(String(e));
  }
}

/** Space-separated text ⇄ string array, for the args field. */
function argsToText(a: string[]): string {
  return a.join(" ");
}
function textToArgs(t: string): string[] {
  return t.split(/\s+/).filter(Boolean);
}

/** "KEY=value" lines ⇄ the ACP name/value array. */
function pairsToText(pairs: { name: string; value: string }[]): string {
  return pairs.map((p) => `${p.name}=${p.value}`).join("\n");
}
function textToPairs(t: string): { name: string; value: string }[] {
  return t
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      const i = line.indexOf("=");
      return i === -1
        ? { name: line, value: "" }
        : { name: line.slice(0, i).trim(), value: line.slice(i + 1) };
    });
}
</script>

<template>
  <div class="p-6 flex justify-center">
    <div class="max-w-3xl w-full">
      <div class="flex items-start justify-between mb-2">
        <div>
          <h2 class="text-[18px] font-semibold text-gray-900 tracking-tight">
            {{ $t("settings.mcp.title") }}
          </h2>
          <p class="text-[13px] text-gray-500 mt-1 max-w-xl">
            {{ $t("settings.mcp.subtitle") }}
          </p>
        </div>
        <button
          v-if="!editing"
          @click="startAdd"
          class="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-semibold bg-indigo-600 text-white hover:bg-indigo-700 transition-all cursor-pointer shrink-0"
        >
          <Plus :size="13" /> {{ $t("settings.mcp.add") }}
        </button>
      </div>

      <!-- Editor -->
      <div v-if="editing" class="mt-5 bg-white rounded-xl border border-gray-200 p-5">
        <div class="space-y-4">
          <div>
            <label class="block text-[12px] font-medium text-gray-700 mb-1">{{ $t("settings.mcp.name") }}</label>
            <input
              v-model="editing.name"
              :placeholder="$t('settings.mcp.namePlaceholder')"
              class="w-full px-3 py-2 rounded-lg border border-gray-200 text-[13px] outline-none focus:border-indigo-400"
            />
          </div>

          <div>
            <label class="block text-[12px] font-medium text-gray-700 mb-1">{{ $t("settings.mcp.transport") }}</label>
            <div class="flex gap-1.5">
              <button
                v-for="t in (['stdio', 'http', 'sse'] as McpTransport[])"
                :key="t"
                @click="setTransport(t)"
                class="px-3 py-1.5 rounded-lg text-[12px] font-medium border transition-all cursor-pointer"
                :class="editing.transport === t
                  ? 'bg-indigo-50 border-indigo-300 text-indigo-700'
                  : 'bg-white border-gray-200 text-gray-600 hover:bg-gray-50'"
              >{{ t }}</button>
            </div>
          </div>

          <!-- stdio -->
          <template v-if="editing.transport === 'stdio'">
            <div>
              <label class="block text-[12px] font-medium text-gray-700 mb-1">{{ $t("settings.mcp.command") }}</label>
              <input
                v-model="editing.command"
                placeholder="npx"
                class="w-full px-3 py-2 rounded-lg border border-gray-200 text-[13px] font-mono outline-none focus:border-indigo-400"
              />
            </div>
            <div>
              <label class="block text-[12px] font-medium text-gray-700 mb-1">{{ $t("settings.mcp.args") }}</label>
              <input
                :value="argsToText(editing.args)"
                @input="editing.args = textToArgs(($event.target as HTMLInputElement).value)"
                placeholder="-y @modelcontextprotocol/server-filesystem /tmp"
                class="w-full px-3 py-2 rounded-lg border border-gray-200 text-[13px] font-mono outline-none focus:border-indigo-400"
              />
            </div>
            <div>
              <label class="block text-[12px] font-medium text-gray-700 mb-1">{{ $t("settings.mcp.env") }}</label>
              <textarea
                :value="pairsToText(editing.env)"
                @input="editing.env = textToPairs(($event.target as HTMLTextAreaElement).value)"
                rows="3"
                placeholder="API_KEY=abc123"
                class="w-full px-3 py-2 rounded-lg border border-gray-200 text-[13px] font-mono outline-none focus:border-indigo-400 resize-y"
              />
              <p class="text-[11px] text-gray-400 mt-1">{{ $t("settings.mcp.envHint") }}</p>
            </div>
          </template>

          <!-- http / sse -->
          <template v-else>
            <div>
              <label class="block text-[12px] font-medium text-gray-700 mb-1">{{ $t("settings.mcp.url") }}</label>
              <input
                v-model="editing.url"
                placeholder="https://example.com/mcp"
                class="w-full px-3 py-2 rounded-lg border border-gray-200 text-[13px] font-mono outline-none focus:border-indigo-400"
              />
            </div>
            <div>
              <label class="block text-[12px] font-medium text-gray-700 mb-1">{{ $t("settings.mcp.headers") }}</label>
              <textarea
                :value="pairsToText(editing.headers)"
                @input="editing.headers = textToPairs(($event.target as HTMLTextAreaElement).value)"
                rows="3"
                placeholder="Authorization=Bearer TOKEN"
                class="w-full px-3 py-2 rounded-lg border border-gray-200 text-[13px] font-mono outline-none focus:border-indigo-400 resize-y"
              />
            </div>
          </template>
        </div>

        <div class="flex justify-end gap-2 mt-5">
          <button
            @click="cancelEdit"
            class="px-3 py-1.5 rounded-lg text-[12px] font-medium bg-gray-50 border border-gray-200 text-gray-600 hover:bg-gray-100 transition-all cursor-pointer"
          >{{ $t("settings.mcp.cancel") }}</button>
          <button
            @click="save"
            :disabled="saving"
            class="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-semibold bg-indigo-600 text-white hover:bg-indigo-700 disabled:opacity-50 transition-all cursor-pointer"
          >
            <Check :size="13" /> {{ $t("settings.mcp.save") }}
          </button>
        </div>
      </div>

      <!-- List -->
      <div v-if="loading" class="mt-5 text-[13px] text-gray-400">{{ $t("settings.mcp.loading") }}</div>

      <div v-else-if="servers.length === 0 && !editing" class="mt-8 text-center">
        <Boxes :size="28" class="mx-auto text-gray-300 mb-3" />
        <p class="text-[13px] text-gray-400">{{ $t("settings.mcp.empty") }}</p>
      </div>

      <div v-else class="mt-5 space-y-2">
        <div
          v-for="s in servers"
          :key="s.id"
          class="flex items-center gap-3 bg-white rounded-xl border border-gray-100 px-4 py-3"
        >
          <span class="w-2 h-2 rounded-full shrink-0" :class="s.enabled ? 'bg-emerald-500' : 'bg-gray-300'" />
          <div class="flex-1 min-w-0">
            <div class="flex items-center gap-2">
              <span class="text-[13px] font-medium text-gray-900 truncate">{{ s.name }}</span>
              <span class="text-[10px] px-1.5 py-0.5 rounded bg-gray-100 text-gray-500 font-mono">{{ s.transport }}</span>
            </div>
            <div class="text-[11px] text-gray-400 truncate font-mono mt-0.5">
              {{ s.transport === 'stdio' ? `${s.command} ${s.args.join(' ')}` : s.url }}
            </div>
          </div>
          <button
            @click="toggleEnabled(s)"
            class="px-2 py-1 rounded-lg text-[11px] font-medium transition-all cursor-pointer shrink-0"
            :class="s.enabled ? 'text-gray-500 hover:bg-gray-100' : 'text-emerald-600 hover:bg-emerald-50'"
          >{{ s.enabled ? $t("settings.mcp.disable") : $t("settings.mcp.enable") }}</button>
          <button @click="startEdit(s)" class="p-1.5 rounded-lg text-gray-400 hover:text-gray-700 hover:bg-gray-100 transition-all cursor-pointer shrink-0">
            <Pencil :size="14" />
          </button>
          <button @click="remove(s)" class="p-1.5 rounded-lg text-gray-400 hover:text-red-500 hover:bg-red-50 transition-all cursor-pointer shrink-0">
            <Trash2 :size="14" />
          </button>
        </div>
      </div>
    </div>
  </div>
</template>