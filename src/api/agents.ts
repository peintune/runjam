import { invoke } from "@tauri-apps/api/core";

export type AgentStatus = "not_installed" | "connection_failed" | "available";

export interface AgentInfo {
  id: string;
  display_name: string;
  install_path: string | null;
  version: string | null;
  installed: boolean;
  enabled: boolean;
  status: AgentStatus;
  last_tested_at: string | null;
}

export async function detectAgents(): Promise<AgentInfo[]> {
  return invoke<AgentInfo[]>("detect_agents");
}

export async function checkAgent(agentId: string): Promise<AgentInfo> {
  return invoke<AgentInfo>("check_agent", { agentId });
}

export async function installAgent(agentId: string): Promise<{ id: string; installed: boolean; version: string | null }> {
  return invoke("install_agent", { agentId });
}

export async function uninstallAgent(agentId: string): Promise<void> {
  return invoke("uninstall_agent", { agentId });
}

export async function setAgentEnabled(agentId: string, enabled: boolean): Promise<void> {
  return invoke("set_agent_enabled", { agentId, enabled });
}

export async function checkNodejs(): Promise<string> {
  return invoke<string>("check_nodejs");
}

export async function getNodejsInstallGuide(): Promise<string> {
  return invoke<string>("get_nodejs_install_guide");
}

export async function openNodejsDownload(): Promise<void> {
  return invoke<void>("open_nodejs_download");
}

export async function readAgentConfig(agentId: string): Promise<string> {
  return invoke<string>("read_agent_config", { agentId });
}

export async function writeAgentConfig(agentId: string, content: string): Promise<void> {
  return invoke("write_agent_config", { agentId, content });
}

export interface DirEntry {
  name: string;
  item_count: number;
}

export interface FileEntry {
  name: string;
  size_bytes: number;
}

export interface AgentDirInfo {
  path: string;
  exists: boolean;
  total_size_bytes: number;
  config_file: string | null;
  history_file: string | null;
  history_size_bytes: number;
  history_lines: number;
  subdirs: DirEntry[];
  key_files: FileEntry[];
}

export async function getAgentDirInfo(agentId: string): Promise<AgentDirInfo> {
  return invoke<AgentDirInfo>("get_agent_dir_info", { agentId });
}

export async function getAgentStatuses(): Promise<AgentInfo[]> {
  return invoke<AgentInfo[]>("get_agent_statuses");
}

export async function testAgent(agentId: string): Promise<{ success: boolean; message: string }> {
  return invoke("test_agent", { agentId });
}
