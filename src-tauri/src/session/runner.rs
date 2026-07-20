use crate::acp::{AcpEvent, AcpMessage};
use crate::acp_client::AcpClient;
use crate::models::session::Session;
use crate::rjlog;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use tauri::{AppHandle, Emitter};

enum ClientType {
    Acp(Arc<Mutex<AcpClient>>),
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

        let acp_client_arc = Arc::new(Mutex::new(acp_client));
        self.clients.lock().unwrap().insert(id.clone(), ClientType::Acp(acp_client_arc.clone()));
        self.active.insert(id.clone(), ());
        rjlog!("[SESSION DEBUG] session started, clients in map: {}", self.clients.lock().unwrap().len());

        let app_clone = app.clone();
        let id_clone = id.clone();
        let cli_display_name_clone = cli_display_name.to_string();
        thread::spawn(move || {
            let mut client = acp_client_arc.lock().unwrap();
            if let Err(e) = client.initialize_session() {
                rjlog!("[ACP ERROR] Initialize session failed: {}", e);
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
            status: "running".to_string(), created_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    pub fn send_input(&self, app: &AppHandle, id: &str, text: &str, _history: Option<&[String]>) -> Result<(), String> {
        let clients = self.clients.lock().unwrap();
        rjlog!("[SESSION DEBUG] send_input called for session: {}, clients in map: {}", id, clients.len());
        let client = clients.get(id)
            .ok_or_else(|| {
                rjlog!("[SESSION ERROR] No client for session: {}, available: {:?}", id, clients.keys().collect::<Vec<_>>());
                format!("No client for session: {}", id)
            })?;

        let turn_id = format!("turn_{}", chrono::Utc::now().timestamp_millis());
        let msg_id = format!("msg_{}", chrono::Utc::now().timestamp_millis());
        let sid = id.to_string();
        let ev = format!("acp:{}", sid);
        let _ = app.emit(&ev, &AcpMessage::new(&sid, &turn_id, &msg_id, AcpEvent::Start));

        match client {
            ClientType::Acp(acp) => {
                let mut acp_client = acp.lock().unwrap();
                let prompt = match _history {
                    Some(h) if !h.is_empty() => format!("Previous conversation:\n{}\n---\nNew message: {}", h.join("\n"), text),
                    _ => text.to_string(),
                };
                acp_client.send_prompt(&prompt)?;
            }
        }

        Ok(())
    }

    pub fn respond(&self, id: &str, response: &str) -> Result<(), String> {
        let clients = self.clients.lock().unwrap();
        let client = clients.get(id)
            .ok_or_else(|| format!("No client for session: {}", id))?;

        match client {
            ClientType::Acp(acp) => {
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
            ClientType::Acp(acp) => {
                let mut acp_client = acp.lock().unwrap();
                acp_client.respond_permission(request_id, response)?;
            }
        }

        Ok(())
    }

    pub fn stop(&mut self, id: &str) -> Result<(), String> {
        rjlog!("[SESSION DEBUG] stop called for session: {}", id);
        self.clients.lock().unwrap().remove(id);
        self.active.remove(id);
        rjlog!("[SESSION DEBUG] session stopped, clients in map: {}", self.clients.lock().unwrap().len());
        Ok(())
    }
}
