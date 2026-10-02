//! MCP (Model Context Protocol) server management.
//!
//! RunJam owns the list and injects it into every session, so a server defined
//! once here works with every agent. Without this, each agent reads MCP servers
//! from its OWN config location (Claude Code from `~/.claude.json`, Codex and
//! Gemini from theirs) and a user has to configure the same server three times —
//! which is the duplication RunJam exists to remove.
//!
//! The agent receives the list through the ACP `session/new` request. The wire
//! format is strict and partly counter-intuitive, so the translation lives in
//! [`to_acp_server`] as a pure function with tests (see the module's test block):
//!
//!   * `env` and `headers` are ARRAYS of `{name, value}` — NOT objects. An
//!     object here is silently ignored by the agent (verified against
//!     `@agentclientprotocol/claude-agent-acp`, which converts them with
//!     `Object.fromEntries(server.env.map(e => [e.name, e.value]))`).
//!   * a stdio server carries NO `type` field at all: the agent detects it by
//!     the ABSENCE of `type` (`!("type" in server)`). Writing `type: "stdio"`
//!     would make it fall through both branches and be dropped.
//!     http/sse, by contrast, REQUIRE `type`.

use crate::db::connection::Database;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::State;

/// Name/value pair, matching ACP's `EnvVariable` / `HttpHeader` shapes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NameValue {
    pub name: String,
    pub value: String,
}

/// One configured MCP server, as stored and shown in the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServer {
    pub id: String,
    pub name: String,
    /// "stdio" | "http" | "sse"
    pub transport: String,
    /// stdio only: executable path or command name.
    #[serde(default)]
    pub command: String,
    /// stdio only: command-line arguments.
    #[serde(default)]
    pub args: Vec<String>,
    /// stdio only: environment variables for the child process.
    #[serde(default)]
    pub env: Vec<NameValue>,
    /// http/sse only: endpoint URL.
    #[serde(default)]
    pub url: String,
    /// http/sse only: request headers (often carries an auth token).
    #[serde(default)]
    pub headers: Vec<NameValue>,
    pub enabled: bool,
}

/// Translate one stored server into the ACP wire shape.
///
/// See the module docs for the two rules that make this non-obvious. Returns
/// `None` for a server that cannot be expressed (unknown transport, or a
/// missing required field), so a single bad entry cannot break the whole session.
pub fn to_acp_server(server: &McpServer) -> Option<serde_json::Value> {
    let name = server.name.trim();
    if name.is_empty() {
        return None;
    }
    match server.transport.as_str() {
        "stdio" => {
            let command = server.command.trim();
            if command.is_empty() {
                return None;
            }
            // NOTE: deliberately no "type" key — the agent keys off its absence.
            Some(serde_json::json!({
                "name": name,
                "command": command,
                "args": server.args,
                "env": server.env,
            }))
        }
        "http" | "sse" => {
            let url = server.url.trim();
            if url.is_empty() {
                return None;
            }
            Some(serde_json::json!({
                "name": name,
                "type": server.transport,
                "url": url,
                "headers": server.headers,
            }))
        }
        _ => None,
    }
}

/// The ACP `mcpServers` array for a set of stored servers (enabled ones only).
///
/// Used by `AcpClient::start` when it builds `session/new`.
pub fn to_acp_servers(servers: &[McpServer]) -> Vec<serde_json::Value> {
    servers
        .iter()
        .filter(|s| s.enabled)
        .filter_map(to_acp_server)
        .collect()
}

/// Load every enabled MCP server. Used on the session-start path, so it must
/// never fail the session: a broken row is skipped rather than propagated.
pub fn enabled_servers(db: &Database) -> Vec<McpServer> {
    let Ok(conn) = db.conn.lock() else { return Vec::new() };
    let Ok(mut stmt) = conn.prepare(
        "SELECT id, name, transport, command, args, env, url, headers, enabled
         FROM mcp_servers WHERE enabled = 1 ORDER BY created_at",
    ) else {
        return Vec::new();
    };
    let rows = stmt.query_map([], |row| {
        Ok(McpServer {
            id: row.get(0)?,
            name: row.get(1)?,
            transport: row.get(2)?,
            command: row.get(3)?,
            args: parse_string_array(row.get::<_, String>(4)?.as_str()),
            env: parse_name_values(row.get::<_, String>(5)?.as_str()),
            url: row.get(6)?,
            headers: parse_name_values(row.get::<_, String>(7)?.as_str()),
            enabled: row.get::<_, i32>(8)? != 0,
        })
    });
    match rows {
        Ok(iter) => iter.filter_map(Result::ok).collect(),
        Err(_) => Vec::new(),
    }
}

fn parse_string_array(raw: &str) -> Vec<String> {
    serde_json::from_str(raw).unwrap_or_default()
}

fn parse_name_values(raw: &str) -> Vec<NameValue> {
    serde_json::from_str(raw).unwrap_or_default()
}

/// The ACP `mcpServers` array, read straight from the app's database.
///
/// Wraps the `State` → lock → convert chain so the caller (`AcpClient::start`)
/// does not have to juggle Tauri's `State` lifetimes. Never fails: any error
/// yields an empty list, which means "no MCP servers" — the behaviour before
/// this feature existed, so a broken database can't block session startup.
pub fn acp_servers_from_app(app: &tauri::AppHandle) -> Vec<serde_json::Value> {
    use tauri::Manager;
    let state = app.state::<std::sync::Mutex<Database>>();
    let Ok(guard) = state.lock() else {
        return Vec::new();
    };
    let configured = enabled_servers(&guard);
    to_acp_servers(&configured)
}

#[tauri::command]
pub fn list_mcp_servers(db: State<'_, Mutex<Database>>) -> Vec<McpServer> {
    let db_guard = match db.lock() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };
    let conn = match db_guard.conn.lock() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut stmt = match conn.prepare(
        "SELECT id, name, transport, command, args, env, url, headers, enabled
         FROM mcp_servers ORDER BY created_at",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map([], |row| {
        Ok(McpServer {
            id: row.get(0)?,
            name: row.get(1)?,
            transport: row.get(2)?,
            command: row.get(3)?,
            args: parse_string_array(row.get::<_, String>(4)?.as_str()),
            env: parse_name_values(row.get::<_, String>(5)?.as_str()),
            url: row.get(6)?,
            headers: parse_name_values(row.get::<_, String>(7)?.as_str()),
            enabled: row.get::<_, i32>(8)? != 0,
        })
    });
    match rows {
        Ok(iter) => iter.filter_map(Result::ok).collect(),
        Err(_) => Vec::new(),
    }
}

/// Create or replace an MCP server.
///
/// The id is generated here when absent so the UI never has to invent one.
/// Validation is deliberately minimal (name required; the field the chosen
/// transport needs must be present) — rejecting a server the agent could
/// actually run would be worse than passing it through.
#[tauri::command]
pub fn save_mcp_server(
    server: McpServer,
    db: State<'_, Mutex<Database>>,
) -> Result<McpServer, String> {
    let name = server.name.trim();
    if name.is_empty() {
        return Err("MCP server name is required".into());
    }
    match server.transport.as_str() {
        "stdio" if server.command.trim().is_empty() => {
            return Err("A stdio MCP server needs a command".into());
        }
        "http" | "sse" if server.url.trim().is_empty() => {
            return Err("An http/sse MCP server needs a URL".into());
        }
        "stdio" | "http" | "sse" => {}
        other => return Err(format!("Unsupported MCP transport: {other}")),
    }

    let id = if server.id.trim().is_empty() {
        format!("mcp-{}", chrono::Utc::now().timestamp_millis())
    } else {
        server.id.clone()
    };
    let args = serde_json::to_string(&server.args).map_err(|e| e.to_string())?;
    let env = serde_json::to_string(&server.env).map_err(|e| e.to_string())?;
    let headers = serde_json::to_string(&server.headers).map_err(|e| e.to_string())?;

    let db_guard = db.lock().map_err(|e| e.to_string())?;
    let conn = db_guard.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT OR REPLACE INTO mcp_servers
         (id, name, transport, command, args, env, url, headers, enabled)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            id,
            name,
            server.transport,
            server.command,
            args,
            env,
            server.url,
            headers,
            if server.enabled { 1 } else { 0 },
        ],
    )
    .map_err(|e| e.to_string())?;

    Ok(McpServer { id, name: name.to_string(), ..server })
}

#[tauri::command]
pub fn delete_mcp_server(id: String, db: State<'_, Mutex<Database>>) -> Result<(), String> {
    let db_guard = db.lock().map_err(|e| e.to_string())?;
    let conn = db_guard.conn.lock().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM mcp_servers WHERE id = ?1", [&id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(transport: &str) -> McpServer {
        McpServer {
            id: "1".into(),
            name: "test".into(),
            transport: transport.into(),
            command: String::new(),
            args: vec![],
            env: vec![],
            url: String::new(),
            headers: vec![],
            enabled: true,
        }
    }

    #[test]
    fn stdio_server_has_no_type_field() {
        // Critical: the agent detects stdio by the ABSENCE of `type`
        // (`!("type" in server)`). Adding one would make it be dropped silently.
        let mut s = server("stdio");
        s.command = "/usr/bin/mcp-server".into();
        let v = to_acp_server(&s).unwrap();
        assert!(v.get("type").is_none(), "stdio must not carry a type: {v}");
        assert_eq!(v["name"], "test");
        assert_eq!(v["command"], "/usr/bin/mcp-server");
    }

    #[test]
    fn stdio_env_is_an_array_of_name_value_pairs() {
        // ACP models env as Array<{name,value}>. Emitting an object (the shape
        // most MCP configs use) is silently ignored by the agent at runtime.
        let mut s = server("stdio");
        s.command = "node".into();
        s.env = vec![NameValue { name: "API_KEY".into(), value: "abc".into() }];
        let v = to_acp_server(&s).unwrap();
        assert_eq!(v["env"], serde_json::json!([{ "name": "API_KEY", "value": "abc" }]));
    }

    #[test]
    fn http_server_requires_the_type_field() {
        let mut s = server("http");
        s.url = "https://example.com/mcp".into();
        let v = to_acp_server(&s).unwrap();
        assert_eq!(v["type"], "http");
        assert_eq!(v["url"], "https://example.com/mcp");
    }

    #[test]
    fn sse_headers_are_name_value_pairs() {
        let mut s = server("sse");
        s.url = "https://example.com/sse".into();
        s.headers = vec![NameValue { name: "Authorization".into(), value: "Bearer t".into() }];
        let v = to_acp_server(&s).unwrap();
        assert_eq!(v["type"], "sse");
        assert_eq!(
            v["headers"],
            serde_json::json!([{ "name": "Authorization", "value": "Bearer t" }])
        );
    }

    #[test]
    fn args_pass_through_as_an_array() {
        let mut s = server("stdio");
        s.command = "npx".into();
        s.args = vec!["-y".into(), "@modelcontextprotocol/server-filesystem".into(), "/tmp".into()];
        let v = to_acp_server(&s).unwrap();
        assert_eq!(v["args"], serde_json::json!(["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]));
    }

    #[test]
    fn incomplete_servers_are_skipped_rather_than_breaking_the_session() {
        // A row missing its required field must not take down session creation.
        let mut broken = server("stdio"); // no command
        broken.name = "broken".into();
        assert!(to_acp_server(&broken).is_none());

        let mut no_url = server("http"); // no url
        no_url.name = "no-url".into();
        assert!(to_acp_server(&no_url).is_none());

        let mut unknown = server("carrier-pigeon");
        unknown.name = "weird".into();
        assert!(to_acp_server(&unknown).is_none());

        let mut unnamed = server("stdio");
        unnamed.name = "   ".into();
        unnamed.command = "x".into();
        assert!(to_acp_server(&unnamed).is_none());
    }

    #[test]
    fn disabled_servers_are_excluded() {
        let mut on = server("stdio");
        on.name = "on".into();
        on.command = "a".into();
        let mut off = server("stdio");
        off.name = "off".into();
        off.command = "b".into();
        off.enabled = false;
        let list = to_acp_servers(&[on, off]);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["name"], "on");
    }

    #[test]
    fn an_empty_list_produces_an_empty_array() {
        // The previous behaviour (always empty) must remain valid.
        assert!(to_acp_servers(&[]).is_empty());
    }
}
#[cfg(test)]
mod roundtrip_tests {
    use super::*;
    use rusqlite::Connection;

    /// An in-memory database with the real schema, so this exercises the actual
    /// SQL and the JSON column encoding — not just the pure converter.
    fn memory_db() -> Database {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::run_migrations(&conn);
        Database { conn: std::sync::Mutex::new(conn) }
    }

    fn insert(db: &Database, server: &McpServer) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO mcp_servers (id, name, transport, command, args, env, url, headers, enabled)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                server.id,
                server.name,
                server.transport,
                server.command,
                serde_json::to_string(&server.args).unwrap(),
                serde_json::to_string(&server.env).unwrap(),
                server.url,
                serde_json::to_string(&server.headers).unwrap(),
                if server.enabled { 1 } else { 0 },
            ],
        )
        .unwrap();
    }

    #[test]
    fn a_stored_server_round_trips_into_an_acp_payload() {
        // The whole path: DB row → struct → ACP wire shape.
        let db = memory_db();
        insert(&db, &McpServer {
            id: "m1".into(),
            name: "filesystem".into(),
            transport: "stdio".into(),
            command: "npx".into(),
            args: vec!["-y".into(), "@modelcontextprotocol/server-filesystem".into(), "/tmp".into()],
            env: vec![NameValue { name: "TOKEN".into(), value: "t0k".into() }],
            url: String::new(),
            headers: vec![],
            enabled: true,
        });

        let configured = enabled_servers(&db);
        assert_eq!(configured.len(), 1);
        // The JSON columns must decode back into structured values, not strings.
        assert_eq!(configured[0].args.len(), 3);
        assert_eq!(configured[0].env[0].name, "TOKEN");

        let acp = to_acp_servers(&configured);
        assert_eq!(acp.len(), 1);
        // And the wire shape keeps the two counter-intuitive rules.
        assert!(acp[0].get("type").is_none(), "stdio must not carry type: {}", acp[0]);
        assert_eq!(acp[0]["env"], serde_json::json!([{ "name": "TOKEN", "value": "t0k" }]));
    }

    #[test]
    fn disabled_rows_never_reach_the_agent() {
        let db = memory_db();
        let mut s = McpServer {
            id: "off".into(),
            name: "off".into(),
            transport: "http".into(),
            command: String::new(),
            args: vec![],
            env: vec![],
            url: "https://x/mcp".into(),
            headers: vec![],
            enabled: false,
        };
        insert(&db, &s);
        s.id = "on".into();
        s.name = "on".into();
        s.enabled = true;
        insert(&db, &s);

        // The SQL filters on enabled, so the agent is only offered the live one.
        let acp = to_acp_servers(&enabled_servers(&db));
        assert_eq!(acp.len(), 1);
        assert_eq!(acp[0]["name"], "on");
    }

    #[test]
    fn malformed_json_columns_do_not_panic() {
        // A hand-edited or truncated row must degrade to "no values", not crash
        // session startup.
        let db = memory_db();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO mcp_servers (id, name, transport, command, args, env, url, headers, enabled)
                 VALUES ('bad','bad','stdio','cmd','not-json','not-json','','not-json',1)",
                [],
            ).unwrap();
        }
        let configured = enabled_servers(&db);
        assert_eq!(configured.len(), 1);
        assert!(configured[0].args.is_empty());
        assert!(configured[0].env.is_empty());
        // Still usable: the command survived, so the server is passed on.
        assert!(to_acp_server(&configured[0]).is_some());
    }
}
