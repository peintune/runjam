use crate::acp::{AcpEvent, AcpMessage};
use crate::acp_client::{terminate_process_tree, AcpClient};
use crate::models::session::Session;
use crate::rjlog;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use tauri::{AppHandle, Emitter};
use chrono::Local;

enum ClientType {
    Acp(Arc<Mutex<AcpClient>>, u32),
}

pub struct SessionManager {
    active: HashMap<String, ()>,
    clients: Arc<Mutex<HashMap<String, ClientType>>>,
}

impl SessionManager {
    pub fn new() -> Self { 
        Self { 
            active: HashMap::new(),
            clients: Arc::new(Mutex::new(HashMap::new())),
        } 
    }

    pub fn start(
        &mut self, app: &AppHandle, id: String,
        cli: &str, cli_display_name: &str, directory: Option<&str>,
        model: Option<&str>, mode: &str, permission_mode: &str,
    ) -> Result<Session, String> {
        rjlog!("[SESSION DEBUG] start called for session: {}, directory: {:?}, model: {:?}, mode: {}, permission_mode: {}", id, directory, model, mode, permission_mode);

        let agent_type = match cli {
            "claude-code" => "claude",
            "codex-cli" => "codex",
            "gemini-cli" => {
                if let Some(m) = model {
                    if m.starts_with("ollama-") {
                        "ollama"
                    } else {
                        "gemini"
                    }
                } else {
                    "gemini"
                }
            },
            "ollama-cli" => "ollama",
            _ => return Err(format!("Unknown CLI: {}", cli)),
        };

        let acp_client = AcpClient::start(app, &id, agent_type, directory, model, mode, permission_mode)
            .map_err(|e| format!("Failed to start ACP agent: {}", e))?;

        let acp_pid = acp_client.pid();
        let acp_client_arc = Arc::new(Mutex::new(acp_client));
        self.clients.lock().unwrap().insert(id.clone(), ClientType::Acp(acp_client_arc.clone(), acp_pid));
        self.active.insert(id.clone(), ());
        rjlog!("[SESSION DEBUG] session started, clients in map: {}", self.clients.lock().unwrap().len());

        let app_clone = app.clone();
        let id_clone = id.clone();
        let cli_display_name_clone = cli_display_name.to_string();
        // Copied into the thread below (references borrowed from `cli`/`model`
        // would not satisfy `thread::spawn`'s 'static bound).
        let agent_type_owned = agent_type.to_string();
        let model_owned = model.map(|s| s.to_string());
        thread::spawn(move || {
            let mut client = acp_client_arc.lock().unwrap();
            if let Err(e) = client.initialize_session(&app_clone, &id_clone) {
                rjlog!("[ACP ERROR] Initialize session failed: {}", e);
                crate::telemetry::report_error_from_app(
                    &app_clone,
                    "error",
                    "acp_init_failed",
                    &e,
                    None,
                    serde_json::json!({
                        "session_id": &id_clone,
                        "cli": &cli_display_name_clone,
                        "agent_type": &agent_type_owned,
                        "model": &model_owned,
                    }),
                );
                let _ = app_clone.emit(&format!("acp:{}", id_clone), &AcpMessage::new(&id_clone, "init", "init", AcpEvent::Text {
                    content: format!("Error: {}", e),
                }));
                return;
            }
            rjlog!("[ACP DEBUG] Session {} initialized: {} ready", id_clone, cli_display_name_clone);
        });

        Ok(Session {
            id, cli: cli.to_string(), cli_display_name: cli_display_name.to_string(),
            directory: directory.map(|s| s.to_string()), pid: None,
            status: "running".to_string(), created_at: Local::now().to_rfc3339(),
        })
    }

    pub fn send_input(&self, app: &AppHandle, id: &str, text: &str, history: Option<&[String]>) -> Result<(), String> {
        let clients = self.clients.lock().unwrap();
        rjlog!("[SESSION DEBUG] send_input called for session: {}, clients in map: {}", id, clients.len());
        let client = clients.get(id)
            .ok_or_else(|| {
                rjlog!("[SESSION ERROR] No client for session: {}, available: {:?}", id, clients.keys().collect::<Vec<_>>());
                crate::telemetry::report_error_from_app(
                    app,
                    "error",
                    "session_no_client",
                    &format!("No client for session: {}", id),
                    None,
                    serde_json::json!({
                        "session_id": id,
                        "live_sessions": clients.len(),
                    }),
                );
                format!("No client for session: {}", id)
            })?;

        let turn_id = format!("turn_{}", Local::now().timestamp_millis());
        let msg_id = format!("msg_{}", Local::now().timestamp_millis());
        let sid = id.to_string();
        let ev = format!("acp:{}", sid);
        let _ = app.emit(&ev, &AcpMessage::new(&sid, &turn_id, &msg_id, AcpEvent::Start));

        match client {
            ClientType::Acp(acp, _) => {
                let mut acp_client = acp.lock().unwrap();
                let prompt = build_prompt(
                    acp_client.project_context().as_deref(),
                    history,
                    text,
                );
                acp_client.send_prompt(&prompt).map_err(|e| {
                    crate::telemetry::report_error_from_app(
                        app,
                        "error",
                        "session_send_prompt_failed",
                        &e,
                        None,
                        serde_json::json!({ "session_id": id }),
                    );
                    e
                })?;
            }
        }

        Ok(())
    }

    /// Whether a backend client (agent process) currently exists AND is still
    /// running for a session. The frontend uses this after a webview reload to
    /// avoid restarting a still-alive agent — restarting would wipe the
    /// process's in-memory session context (conversation history).
    pub fn has_client(&self, id: &str) -> bool {
        let clients = self.clients.lock().unwrap();
        match clients.get(id) {
            Some(ClientType::Acp(acp, _)) => acp.lock().unwrap().is_alive(),
            None => false,
        }
    }

    /// Live-update the permission mode of a running session's ACP client
    /// (e.g. switching out of Plan Mode mid-conversation).
    pub fn set_permission_mode(&self, id: &str, mode: &str) -> Result<(), String> {
        let clients = self.clients.lock().unwrap();
        let client = clients.get(id)
            .ok_or_else(|| format!("No client for session: {}", id))?;
        match client {
            ClientType::Acp(acp, _) => {
                let mut acp_client = acp.lock().unwrap();
                acp_client.set_permission_mode(mode);
            }
        }
        Ok(())
    }

    pub fn respond(&self, id: &str, response: &str) -> Result<(), String> {
        let clients = self.clients.lock().unwrap();
        let client = clients.get(id)
            .ok_or_else(|| format!("No client for session: {}", id))?;

        match client {
            ClientType::Acp(acp, _) => {
                let mut acp_client = acp.lock().unwrap();
                acp_client.send_prompt(response)?;
            }
        }

        Ok(())
    }

    pub fn respond_permission(&self, id: &str, request_id: &str, response: &str) -> Result<(), String> {
        let clients = self.clients.lock().unwrap();
        let client = clients.get(id)
            .ok_or_else(|| format!("No client for session: {}", id))?;

        match client {
            ClientType::Acp(acp, _) => {
                let mut acp_client = acp.lock().unwrap();
                acp_client.respond_permission(request_id, response)?;
            }
        }

        Ok(())
    }

    pub fn stop(&mut self, id: &str) -> Result<(), String> {
        rjlog!("[SESSION DEBUG] stop called for session: {}", id);
        // Terminate the agent process tree BEFORE dropping the client — dropping
        // the Arc alone leaves the process running and the stdout reader thread
        // emitting events (the "Stop button doesn't stop" bug).
        let removed = self.clients.lock().unwrap().remove(id);
        if let Some(ClientType::Acp(acp, pid)) = removed {
            // Kill by pid WITHOUT taking the acp lock: the init thread may hold
            // it for up to 30s during a hung handshake, and stop() must return
            // immediately regardless of that.
            terminate_process_tree(pid);
            if let Ok(mut client) = acp.try_lock() {
                // Reap and (re)kill via the client — a no-op if already dead.
                client.stop();
            }
            // If the lock is busy, the process is already terminated above; the
            // init thread will unwind and AcpClient::Drop reaps it.
        }
        self.active.remove(id);
        rjlog!("[SESSION DEBUG] session stopped, clients in map: {}", self.clients.lock().unwrap().len());
        Ok(())
    }
}

/// Assemble the text actually sent to the agent for one turn.
///
/// Up to three parts, in order: the project scope hint, the previous turns (a
/// restarted process has no memory), then the user's own message. `---` plus the
/// `New message:` label marks where the user's words begin, so the agent does not
/// mistake the surrounding context for the request itself.
///
/// When there is nothing to prepend, the user's text is forwarded verbatim —
/// this is the common case for a fresh session with no project selected, and
/// wrapping it would only add noise.
pub(crate) fn build_prompt(
    project_context: Option<&str>,
    history: Option<&[String]>,
    text: &str,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(ctx) = project_context {
        parts.push(ctx.to_string());
    }
    if let Some(h) = history {
        if !h.is_empty() {
            parts.push(format!("Previous conversation:\n{}", h.join("\n")));
        }
    }
    if parts.is_empty() {
        text.to_string()
    } else {
        format!("{}\n---\nNew message: {}", parts.join("\n"), text)
    }
}

#[cfg(test)]
mod build_prompt_tests {
    use super::build_prompt;

    #[test]
    fn forwards_the_message_unchanged_when_there_is_no_context() {
        assert_eq!(build_prompt(None, None, "why is it broken?"), "why is it broken?");
    }

    #[test]
    fn puts_the_project_scope_before_the_user_message() {
        let prompt = build_prompt(Some("[Context] project: /repo"), None, "why broken?");
        let (scope, msg) = prompt.split_once("---\nNew message: ").expect("separator present");
        // The separator is "\n---\n", so the scope segment ends with that newline.
        assert_eq!(scope.trim_end(), "[Context] project: /repo");
        // The user's own words come last, unmodified.
        assert_eq!(msg, "why broken?");
    }

    #[test]
    fn combines_scope_and_history_in_order() {
        let history = vec!["user: hi".to_string(), "agent: hello".to_string()];
        let prompt = build_prompt(Some("[Context] /repo"), Some(&history), "and now?");
        let scope_at = prompt.find("project").or_else(|| prompt.find("/repo")).unwrap();
        let hist_at = prompt.find("Previous conversation:").unwrap();
        let msg_at = prompt.find("and now?").unwrap();
        // Order is what makes the message unambiguous: scope, then history, then
        // the actual request.
        assert!(scope_at < hist_at && hist_at < msg_at, "got: {prompt}");
        assert!(prompt.contains("user: hi") && prompt.contains("agent: hello"));
    }

    #[test]
    fn ignores_an_empty_history() {
        // An empty slice must not produce a dangling "Previous conversation:"
        // header that would look like the context exists when it does not.
        let prompt = build_prompt(None, Some(&[]), "question");
        assert_eq!(prompt, "question");
    }
}
