mod analysis_instructions;
mod antigravity;
mod claude;
mod codex;
mod manager;
mod process;

pub use manager::{ChatRun, RunManager};
pub use process::probe_providers;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct AdapterConfig {
    pub executable: PathBuf,
    pub work_dir: PathBuf,
    pub mcp_executable: PathBuf,
    pub mcp_arguments: Vec<String>,
    pub data_dir: PathBuf,
    pub conversation_id: String,
    pub context: String,
    pub model: Option<String>,
}

#[derive(Clone, Debug)]
pub struct LaunchSpec {
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub environment: Vec<(String, String)>,
}

pub trait AdapterIo {
    fn send(&mut self, value: &Value) -> Result<(), String>;
    fn receive(&mut self) -> Result<Value, String>;
    fn emit(&mut self, event: ProviderEvent) -> Result<(), String>;
    /// Host-managed MCP transport, used when a CLI cannot load a workspace MCP
    /// configuration itself. The adapter still chooses tools, not the UI.
    fn call_tool(&mut self, _name: &str, _arguments: &Value) -> Result<Value, String> {
        Err("O transporte MCP gerenciado não está configurado.".into())
    }
    fn tool_definitions(&mut self) -> Result<Value, String> {
        Err("O catálogo MCP gerenciado não está configurado.".into())
    }
}

#[derive(Clone, Debug)]
pub enum ProviderEvent {
    Status {
        message: String,
    },
    Delta {
        text: String,
    },
    ToolStarted {
        call_id: String,
        name: String,
        input: Value,
    },
    ToolFinished {
        call_id: String,
        name: String,
        output: Value,
        is_error: bool,
    },
}

#[derive(Clone, Debug)]
pub struct AdapterResult {
    pub text: String,
    pub session_id: Option<String>,
}

pub(crate) fn launch(provider: &str, config: &AdapterConfig) -> Result<LaunchSpec, String> {
    match provider {
        "codex" => codex::launch(config),
        "claude" => claude::launch(config),
        "antigravity" => antigravity::launch(config),
        _ => Err("Provedor desconhecido.".into()),
    }
}

pub(crate) fn run_adapter(
    provider: &str,
    config: &AdapterConfig,
    io: &mut dyn AdapterIo,
) -> Result<AdapterResult, String> {
    match provider {
        "codex" => codex::run(config, io),
        "claude" => claude::run(config, io),
        "antigravity" => antigravity::run(config, io),
        _ => Err("Provedor desconhecido.".into()),
    }
}
