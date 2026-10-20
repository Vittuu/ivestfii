use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub created_at: String,
}

/// Counts Unicode scalar values of message contents and summary (not JSON bytes).
/// Tool/source records remain in storage and can be fetched separately; they are
/// never silently attached outside this budget.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextBundle {
    pub conversation_id: String,
    pub max_chars: usize,
    pub included_chars: usize,
    pub truncated: bool,
    pub summary: Option<String>,
    pub messages: Vec<Message>,
    pub omitted_message_count: usize,
    pub truncated_message_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    pub id: String,
    pub name: String,
    pub installed: bool,
    pub executable: Option<String>,
    pub transport: String,
}

/// Design-time capabilities, distinct from an end-to-end validated connection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorCapabilities {
    pub streaming: bool,
    pub model_selection: bool,
    pub mcp_tools: bool,
    pub canonical_context_replay: bool,
    pub implementation_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorSpecification {
    pub provider_id: String,
    pub transport: String,
    pub proposed_arguments: Vec<String>,
    pub capabilities: ConnectorCapabilities,
    pub verification_required: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskCheckpoint {
    pub id: String,
    pub conversation_id: String,
    pub kind: String,
    pub status: String,
    pub checkpoint: Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolEvent {
    pub id: String,
    pub conversation_id: String,
    pub task_id: Option<String>,
    pub tool_name: String,
    pub input: Value,
    pub output: Option<Value>,
    pub status: String,
    pub started_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceReference {
    pub id: String,
    pub conversation_id: String,
    pub tool_event_id: Option<String>,
    pub title: String,
    pub url: String,
    pub publisher: Option<String>,
    pub retrieved_at: String,
    pub excerpt: Option<String>,
}
