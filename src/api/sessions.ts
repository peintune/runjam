import { invoke } from "@tauri-apps/api/core";

export interface SessionInfo {
  id: string;
  cli: string;
  cli_display_name: string;
  directory: string | null;
  pid: number | null;
  status: string;
  created_at: string;
}

export interface SkillInfo {
  name: string;
  description: string;
}

export async function listSkills(): Promise<SkillInfo[]> {
  return invoke<SkillInfo[]>("list_skills");
}

/** List skills the user installed from .zip packages (~/.runjam/skills/). */
export async function listUserSkills(): Promise<SkillInfo[]> {
  return invoke<SkillInfo[]>("list_user_skills");
}

/** Install skills from a base64-encoded .zip package. Returns installed skills. */
export async function installSkillZip(zipBase64: string): Promise<SkillInfo[]> {
  return invoke<SkillInfo[]>("install_skill_zip", { zipBase64 });
}

/** Remove a user-installed skill. */
export async function removeUserSkill(skillName: string): Promise<void> {
  return invoke("remove_user_skill", { skillName });
}

/** List the skill names already deployed in a session's per-agent skills directory. */
export async function listSessionSkills(cwd: string, cli: string): Promise<string[]> {
  return invoke<string[]>("list_session_skills", { cwd, cli });
}

/** Deploy a single builtin skill to a session's per-agent skills directory. */
export async function deploySessionSkill(cwd: string, cli: string, skillName: string): Promise<string> {
  return invoke<string>("deploy_session_skill", { cwd, cli, skillName });
}

/** Remove a single skill from a session's per-agent skills directory. */
export async function removeSessionSkill(cwd: string, cli: string, skillName: string): Promise<void> {
  return invoke("remove_session_skill", { cwd, cli, skillName });
}

export async function startSession(
  cli: string,
  cliDisplayName: string,
  directory?: string,
  sessionId?: string,
  model?: string,
  mode?: string,
  permissionMode?: string,
  skills?: string[],
): Promise<SessionInfo> {
  return invoke<SessionInfo>("start_session", {
    cli,
    cliDisplayName,
    directory: directory ?? null,
    sessionId: sessionId ?? null,
    model: model ?? null,
    mode: mode ?? "assistant",
    permissionMode: permissionMode ?? null,
    skills: skills ?? null,
  });
}

export async function stopSession(id: string): Promise<void> {
  return invoke("stop_session", { id });
}

/** Whether a backend agent process still exists for the session. Used after a
 *  webview reload to avoid restarting a still-alive agent (which would wipe
 *  its in-process conversation context). */
export async function sessionAlive(id: string): Promise<boolean> {
  return invoke<boolean>("session_alive", { id });
}

export async function listSessions(): Promise<SessionInfo[]> {
  return invoke<SessionInfo[]>("list_sessions");
}

export async function getSessionLogs(id: string): Promise<string[]> {
  return invoke<string[]>("get_session_logs", { id });
}

export async function sendInput(id: string, text: string, history?: string[]): Promise<void> {
  return invoke("send_input", { id, text, history: history ?? null });
}

export async function respondInteraction(id: string, response: string): Promise<void> {
  return invoke("respond_interaction", { id, response });
}

export async function respondPermission(id: string, requestId: string, response: string): Promise<void> {
  return invoke("respond_permission", { id, requestId, response });
}
