import { invoke } from "@tauri-apps/api/core";

export interface CostSummary {
  today_tokens: number;
  week_tokens: number;
  month_tokens: number;
  total_tokens: number;
  today_cost: number;
  week_cost: number;
  month_cost: number;
  total_cost: number;
}

export interface AgentCost {
  agent_id: string;
  agent_name: string;
  total_tokens: number;
  input_tokens: number;
  output_tokens: number;
  cost: number;
  sessions: number;
}

export interface DailyCost {
  date: string;
  total_tokens: number;
  input_tokens: number;
  output_tokens: number;
  cost: number;
}

export interface SessionCost {
  session_id: string;
  agent_name: string;
  directory: string;
  total_tokens: number;
  input_tokens: number;
  output_tokens: number;
  cost: number;
  created_at: string;
  message_count: number;
}

export interface DirectoryCost {
  directory: string;
  total_tokens: number;
  input_tokens: number;
  output_tokens: number;
  cost: number;
  sessions: number;
}

export async function getCostSummary(): Promise<CostSummary> {
  return invoke("get_cost_summary");
}

export async function getCostByAgent(): Promise<AgentCost[]> {
  return invoke("get_cost_by_agent");
}

export async function getCostByDay(days: number): Promise<DailyCost[]> {
  return invoke("get_cost_by_day", { days });
}

export async function getCostBySession(limit: number): Promise<SessionCost[]> {
  return invoke("get_cost_by_session", { limit });
}

export async function getCostByDirectory(): Promise<DirectoryCost[]> {
  return invoke("get_cost_by_directory");
}
