import { getNativeModels, type ModelEntry } from "../api/models";
import { NATIVE_MODEL_ID } from "../api/sessions";
import { t } from "../i18n";
import { ask } from "@tauri-apps/plugin-dialog";
import { withPetWindowDialog } from "./petWindowDialog";

export function chooseContextTransfer(): Promise<boolean> {
  return withPetWindowDialog(() => ask(t("models.switchContext"), {
    title: "RunJam", okLabel: t("models.carryContext"), cancelLabel: t("models.emptyContext"),
  }));
}

export function supportsNativeConfig(agent: string): boolean {
  return ["claude-code", "gemini-cli", "codex-cli"].includes(agent);
}

/** Saved defaults (including first-launch defaults) always beat inheritance. */
export function resolveDefaultPermission(saved: string, native: boolean): string {
  return saved || (native ? "inherit" : "approve_for_me");
}

/** A UI choice, never a persisted provider/model record. */
export async function withNativeChoice(agent: string, models: ModelEntry[]): Promise<ModelEntry[]> {
  const managed = models.filter(m => m.id !== NATIVE_MODEL_ID);
  if (!supportsNativeConfig(agent)) return managed;
  // Metadata is only a label. A read failure must not remove the native
  // launch mode; the backend still validates its config when it is launched.
  const native = await getNativeModels(agent).catch(() => []);
  const name = native[0]?.name || t("models.cliDefault");
  return [{
    id: NATIVE_MODEL_ID, name, alias: t("models.nativeMode"), provider: "native",
    provider_name: {"codex-cli": "Codex", "claude-code": "Claude", "gemini-cli": "Gemini"}[agent] || agent,
    provider_icon: "", api_base: "", api_key: "",
    protocol: "openai_responses", context_window: 0, support_reasoning: true,
    support_tools: true, tags: [], use_proxy: false, force_reasoning_none: false,
  }, ...managed];
}
