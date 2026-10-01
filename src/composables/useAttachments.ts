/**
 * Attaching files to a message.
 *
 * Shared by the main window's composer and the desktop-pet popup so both build
 * the exact same payload: the file is parsed to text and appended after the
 * user's own message, capped so a huge attachment cannot blow up the context
 * window. Extracted from `SessionView.vue` because the pet popup needs
 * identical behaviour — keeping two copies in sync would drift.
 */
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { parseFile } from "../api/fs";

/** A file the user attached to the message they are composing. */
export interface AttachedFile {
  path: string;
  name: string;
  size: number;
  ext: string;
  parsedContent: string;
  truncated: boolean;
  error: string;
}

/** Extensions the picker offers (mirrors the backend's `parse_file` support). */
export const ATTACH_ACCEPTED_EXTS = [
  "txt", "md", "json", "csv", "log", "yaml", "yml", "xml",
  "py", "js", "jsx", "ts", "tsx", "java", "rs", "go",
  "html", "css", "scss", "less", "sh", "bash", "toml", "ini", "cfg", "conf",
  "sql", "vue", "rb", "php", "swift", "kt", "scala", "c", "cpp", "h", "hpp",
  "docx", "xlsx", "xls", "pptx", "pdf",
];

/** Hard cap on the total attached text sent to the model, to protect context. */
export const MAX_ATTACH_TOTAL_CHARS = 100_000;

/** Human-readable file size for the attachment list. */
export function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/**
 * Open the native picker and return the newly chosen attachments.
 *
 * Paths already attached are skipped, so re-picking the same file is a no-op.
 * A file whose size cannot be read is still attached (size 0) — the size is
 * only cosmetic, and refusing the file over it would be worse than showing 0 B.
 */
export async function pickAttachedFiles(
  alreadyAttached: ReadonlySet<string>,
): Promise<AttachedFile[]> {
  const selected = await openDialog({
    multiple: true,
    filters: [{ name: "Supported Files", extensions: ATTACH_ACCEPTED_EXTS }],
  });
  if (!selected) return [];
  const list = Array.isArray(selected) ? selected : [selected];

  const added: AttachedFile[] = [];
  for (const file of list) {
    if (alreadyAttached.has(file)) continue;
    const name = file.split("/").pop() || file;
    const ext = name.includes(".") ? name.split(".").pop()!.toLowerCase() : "";
    let size = 0;
    try {
      size = await invoke<number>("get_file_size", { path: file });
    } catch {
      /* size is cosmetic — keep the file */
    }
    added.push({ path: file, name, size, ext, parsedContent: "", truncated: false, error: "" });
  }
  return added;
}

/**
 * Parse every attached file and build the text actually sent to the agent.
 *
 * The user's own text is returned unchanged when nothing is attached (or when
 * every attachment failed to parse). Failures are reported back instead of
 * thrown, so the caller can decide how to surface them — the main window uses a
 * toast, the pet popup writes them into the transcript.
 */
export async function buildAttachmentPayload(
  files: readonly AttachedFile[],
  userText: string,
): Promise<{ text: string; failures: string[] }> {
  const parts: string[] = [];
  const failures: string[] = [];

  for (const f of files) {
    try {
      const parsed = await parseFile(f.path);
      if (parsed.error) {
        f.error = parsed.error;
        failures.push(`${f.name}: ${parsed.error}`);
        continue;
      }
      f.parsedContent = parsed.content;
      f.truncated = parsed.truncated;
      parts.push(`[Attached File: ${f.name}]\n${parsed.content}`);
    } catch (err) {
      f.error = String(err);
      failures.push(`${f.name}: ${err}`);
    }
  }

  let attachText = parts.join("\n\n");
  if (attachText.length > MAX_ATTACH_TOTAL_CHARS) {
    attachText =
      attachText.slice(0, MAX_ATTACH_TOTAL_CHARS) +
      `\n\n[Attachments truncated: content exceeds ${(MAX_ATTACH_TOTAL_CHARS / 1000).toFixed(0)}k characters]`;
  }

  if (!attachText) return { text: userText, failures };
  return { text: userText ? `${userText}\n\n${attachText}` : attachText, failures };
}

/** The list of `📎 name` lines shown in the user's own bubble (never the file
 *  bodies — those are sent to the model but stay out of the transcript). */
export function attachmentDisplaySuffix(files: readonly AttachedFile[]): string {
  if (files.length === 0) return "";
  return `\n\n${files.map((f) => `📎 ${f.name}`).join("\n")}`;
}