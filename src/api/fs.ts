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
