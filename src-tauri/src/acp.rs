use serde::Serialize;
use serde::Deserialize;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum AcpEvent {
    #[serde(rename = "start")] Start,
    #[serde(rename = "thinking")] Thinking {
        content: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        duration: Option<String>,
    },
    #[serde(rename = "text")] Text { content: String },
    #[serde(rename = "tool_call")] ToolCall {
        tool_name: String,
        input: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        start_time: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    #[serde(rename = "tool_result")] ToolResult {
        tool_name: String,
        output: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    #[serde(rename = "interaction")] Interaction { prompt: String, options: Vec<InteractionOption> },
    #[serde(rename = "permission_request")] PermissionRequest {
        request_id: String,
        prompt: String,
        options: Vec<PermissionOption>,
    },
    #[serde(rename = "finish")] Finish { stop_reason: String },
    #[serde(rename = "error")] Error { message: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct InteractionOption {
    pub key: String,
    pub label: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionOption {
    pub key: String,
    pub label: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AcpMessage {
    pub session_id: String,
    pub msg_id: String,
    pub turn_id: String,
    #[serde(flatten)]
    pub event: AcpEvent,
}

impl AcpMessage {
    pub fn new(sid: &str, tid: &str, mid: &str, event: AcpEvent) -> Self {
        Self { session_id: sid.into(), turn_id: tid.into(), msg_id: mid.into(), event }
    }
}
