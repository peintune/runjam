import { invoke } from "@tauri-apps/api/core";

export interface FileEntry {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
  modified: string;
  extension: string;
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
