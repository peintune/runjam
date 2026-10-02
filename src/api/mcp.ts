import { invoke } from "@tauri-apps/api/core";

/** A name/value pair, matching ACP's env-variable / HTTP-header shape.
 *
 * Deliberately an array of pairs rather than a `Record<string, string>`: that is
 * the form ACP itself uses (`Array<{name,value}>`), and sending an object is
 * silently ignored by the agent. Keeping the UI in the same shape removes a
 * conversion step that could get it wrong. */
export interface NameValue {
  name: string;
  value: string;
}

export type McpTransport = "stdio" | "http" | "sse";

/**
 * One MCP server, as configured in RunJam.
 *
 * RunJam owns this list and injects it into every session, so a server defined
 * once works with every agent — instead of being re-declared in each agent's own
 * config file.
 */
export interface McpServer {
  id: string;
  name: string;
  transport: McpTransport;
  /** stdio: executable path or command name. */
  command: string;
  /** stdio: command-line arguments. */
  args: string[];
  /** stdio: environment variables for the child process. */
  env: NameValue[];
  /** http/sse: endpoint URL. */
  url: string;
  /** http/sse: request headers (often carries an auth token). */
  headers: NameValue[];
  enabled: boolean;
}

/** An empty server record for the "add" form. */
export function emptyMcpServer(): McpServer {
  return {
    id: "",
    name: "",
    transport: "stdio",
    command: "",
    args: [],
    env: [],
    url: "",
    headers: [],
    enabled: true,
  };
}

export async function listMcpServers(): Promise<McpServer[]> {
  return invoke<McpServer[]>("list_mcp_servers");
}

/** Create or update a server. The backend assigns an id when `id` is empty and
 *  returns the stored record (so the caller learns the new id). */
export async function saveMcpServer(server: McpServer): Promise<McpServer> {
  return invoke<McpServer>("save_mcp_server", { server });
}

export async function deleteMcpServer(id: string): Promise<void> {
  return invoke("delete_mcp_server", { id });
}