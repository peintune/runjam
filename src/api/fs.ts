import { invoke } from "@tauri-apps/api/core";

export interface FileEntry {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
  modified: string;
  extension: string;
}

export interface FileSearchResult {
  path: string;
  name: string;
  relative_path: string;
  match_type: string; // "filename" | "content"
  line_number: number | null;
  line_content: string | null;
}

export async function listDir(path: string): Promise<FileEntry[]> {
  return invoke<FileEntry[]>("list_dir", { path });
}

export async function readFileText(path: string): Promise<string> {
  return invoke<string>("read_file_text", { path });
}

export async function writeFile(path: string, content: string): Promise<void> {
  return invoke("write_file", { path, content });
}

export async function readFileBytes(path: string): Promise<number[]> {
  return invoke<number[]>("read_file_bytes", { path });
}

export async function searchFiles(rootPath: string, query: string, limit?: number): Promise<FileSearchResult[]> {
  return invoke<FileSearchResult[]>("search_files", { rootPath, query, limit });
}

// ── File tree mutations ──────────────────────────────────────────
//
// The file tree's right-click menu and "+" toolbar use these to create and
// rename files/folders. `renamePath` doubles as cross-directory move since
// Tauri hands it straight to fs::rename.

export async function createDir(path: string, root: string): Promise<void> {
  return invoke("create_dir", { path, root });
}

export async function createFile(path: string, root: string): Promise<void> {
  return invoke("create_file", { path, root });
}

export async function renamePath(oldPath: string, newPath: string, root: string): Promise<void> {
  return invoke("rename_path", { oldPath, newPath, root });
}

export async function deletePath(path: string, root: string): Promise<void> {
  return invoke("delete_path", { path, root });
}

export interface MentionEntries {
  recent: FileEntry[];
  root: FileEntry[];
}

export async function listMentionEntries(rootPath: string, recentLimit?: number): Promise<MentionEntries> {
  return invoke<MentionEntries>("list_mention_entries", { rootPath, recentLimit });
}

export async function searchMentionFiles(rootPath: string, query: string, limit?: number): Promise<FileEntry[]> {
  return invoke<FileEntry[]>("search_mention_files", { rootPath, query, limit });
}

// ── File Attachment Parser ──────────────────────────────────────────

export interface ParsedFile {
  name: string;
  path: string;
  content: string;
  size: number;
  truncated: boolean;
  error: string | null;
}

export async function parseFile(path: string): Promise<ParsedFile> {
  return invoke<ParsedFile>("parse_file", { path });
}
