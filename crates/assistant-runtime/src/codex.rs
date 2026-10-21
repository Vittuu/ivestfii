//! Codex app-server adapter (protocol verified against CLI 0.147.0 schemas).
//!
//! The application owns conversation history. Every invocation creates an
//! ephemeral thread and replays that history instead of resuming opaque CLI
//! state. Authentication is left entirely to the installed official CLI.

use crate::{AdapterConfig, AdapterIo, AdapterResult, LaunchSpec, ProviderEvent};
use serde_json::{json, Value};
use std::collections::HashSet;

const SERVER: &str = "ivestfii";
const TOOLS: [&str; 5] = [
    "carteira_snapshot",
    "projetar_renda",
    "recuperar_contexto",
    "preparar_analise_carteira",
    "simular_aporte",
];

/// Generate a TOML basic string without shell interpolation. JSON strings use
/// escapes accepted by TOML, including doubled Windows path separators.
fn toml_string(value: &str) -> String {
    serde_json::to_string(value).expect("serializing a string is infallible")
}

pub fn launch(config: &AdapterConfig) -> Result<LaunchSpec, String> {
    let command = config
        .mcp_executable
        .to_str()
        .ok_or("O caminho do servidor MCP não é UTF-8 válido.")?;
    let tools = TOOLS.map(toml_string).join(",");
    let mcp_arguments = config
        .mcp_arguments
        .iter()
        .map(|arg| toml_string(arg))
        .collect::<Vec<_>>()
        .join(",");
    let mcp_config = format!(
        "mcp_servers={{ivestfii={{command={},args=[{}],enabled=true,required=true,enabled_tools=[{}],startup_timeout_sec=30,tool_timeout_sec=90}},node_repl={{command=\"disabled\",enabled=false}}}}",
        toml_string(command),
        mcp_arguments,
        tools,
    );
    let mut args = vec!["app-server".into(), "--stdio".into()];
    // Supply a complete map and explicitly disable the host's node_repl.
    // Some app-server builds inject this server from their host environment.
    // Verify the effective inventory again before submitting any user data.
    // No personal configuration files are read or edited here.
    for config_override in [
        mcp_config,
        "plugins={}".into(),
        "web_search=\"disabled\"".into(),
        "sandbox_mode=\"read-only\"".into(),
        "approval_policy=\"never\"".into(),
    ] {
        args.extend(["-c".into(), config_override]);
    }
    // Read-only sandboxing alone still permits shell reads. Remove those
    // tools, plugins and UI automation from this finance chat altogether.
    for feature in [
        "shell_tool",
        "unified_exec",
        "plugins",
        "apps",
        "hooks",
        "computer_use",
        "browser_use",
        "browser_use_external",
        "in_app_browser",
        "image_generation",
        "view_image",
        "multi_agent",
        "workspace_dependencies",
        "skill_search",
        "skill_mcp_dependency_install",
        "code_mode",
        "code_mode_only",
        "memories",
    ] {
        args.extend(["--disable".into(), feature.into()]);
    }
    // CLI 0.147.0 requires its internal tool host to expose MCP calls to the
    // model, even when code_mode itself is disabled. Disabling this host leaves
    // mcpServerStatus/list populated but makes every model-side MCP unavailable.
    // This does not enable node_repl: its MCP remains explicitly disabled above.
    args.extend(["--enable".into(), "code_mode_host".into()]);
    Ok(LaunchSpec {
        executable: config.executable.clone(),
        args,
        environment: Vec::new(),
    })
}

#[derive(Default)]
struct State {
    thread_id: Option<String>,
    turn_id: Option<String>,
    completed_messages: Vec<CompletedMessage>,
    started_tools: HashSet<String>,
    completed_tools: HashSet<String>,
}

struct CompletedMessage {
    id: String,
    text: String,
    phase: Option<String>,
}

fn error_message(value: &Value) -> String {
    value
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("erro de protocolo sem detalhes")
        .to_owned()
}

/// Every server request gets a response. Unsupported interactive requests
/// must never leave an invisible approval dialog blocking the subprocess.
fn refuse_request(io: &mut dyn AdapterIo, frame: &Value) -> Result<(), String> {
    let id = frame.get("id").cloned().unwrap_or(Value::Null);
    let method = frame.get("method").and_then(Value::as_str).unwrap_or("");
    let result = match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            Some(json!({"decision":"decline"}))
        }
        "execCommandApproval" | "applyPatchApproval" => Some(json!({"decision":"abort"})),
        "item/permissions/requestApproval" => Some(json!({"permissions":{},"scope":"turn"})),
        "mcpServer/elicitation/request" => {
            Some(json!({"action":"decline","content":null,"_meta":null}))
        }
        _ => None,
    };
    let response = match result {
        Some(result) => json!({"id":id,"result":result}),
        None => json!({"id":id,"error":{"code":-32601,"message":
            "Interação indisponível: o chat permite somente as ferramentas de leitura do MCP ivestfii. Faça login pela CLI oficial se necessário."}}),
    };
    io.send(&response)?;
    io.emit(ProviderEvent::Status {
        message: format!("Codex: solicitação interativa recusada ({method})."),
    })?;
    Ok(())
}

impl State {
    fn belongs_to_thread(&self, params: &Value) -> bool {
        match (
            self.thread_id.as_deref(),
            params.get("threadId").and_then(Value::as_str),
        ) {
            (Some(expected), Some(actual)) => expected == actual,
            _ => true,
        }
    }

    fn belongs_to_turn(&self, params: &Value) -> bool {
        self.belongs_to_thread(params)
            && match (
                self.turn_id.as_deref(),
                params.get("turnId").and_then(Value::as_str),
            ) {
                (Some(expected), Some(actual)) => expected == actual,
                _ => true,
            }
    }

    fn record_item(
        &mut self,
        io: &mut dyn AdapterIo,
        item: &Value,
        completed: bool,
    ) -> Result<(), String> {
        match item.get("type").and_then(Value::as_str) {
            Some("agentMessage") if completed => {
                let id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or("Codex enviou mensagem sem identificador.")?;
                let text = item
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or("Codex enviou mensagem sem texto.")?;
                let message = CompletedMessage {
                    id: id.to_owned(),
                    text: text.to_owned(),
                    phase: item.get("phase").and_then(Value::as_str).map(str::to_owned),
                };
                if let Some(existing) = self.completed_messages.iter_mut().find(|m| m.id == id) {
                    *existing = message;
                } else {
                    self.completed_messages.push(message);
                }
            }
            Some("mcpToolCall") => {
                let server = item.get("server").and_then(Value::as_str).unwrap_or("");
                let name = item.get("tool").and_then(Value::as_str).unwrap_or("");
                if server != SERVER || !TOOLS.contains(&name) {
                    return Err("Codex tentou usar uma ferramenta fora do MCP ivestfii.".into());
                }
                let id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or("Codex enviou chamada MCP sem identificador.")?;
                if self.started_tools.insert(id.to_owned()) {
                    io.emit(ProviderEvent::ToolStarted {
                        call_id: id.to_owned(),
                        name: name.to_owned(),
                        input: item.get("arguments").cloned().unwrap_or_else(|| json!({})),
                    })?;
                }
                if completed && self.completed_tools.insert(id.to_owned()) {
                    let error = item.get("error").filter(|value| !value.is_null());
                    let result = item.get("result").filter(|value| !value.is_null());
                    let is_error = item.get("status").and_then(Value::as_str) != Some("completed")
                        || error.is_some()
                        || result
                            .and_then(|r| r.get("isError"))
                            .and_then(Value::as_bool)
                            == Some(true);
                    io.emit(ProviderEvent::ToolFinished {
                        call_id: id.to_owned(),
                        name: name.to_owned(),
                        output: error.or(result).cloned().unwrap_or(Value::Null),
                        is_error,
                    })?;
                }
            }
            Some("commandExecution" | "fileChange" | "dynamicToolCall" | "browserToolCall") => {
                return Err(
                    "Codex tentou executar uma ação externa às ferramentas MCP permitidas.".into(),
                );
            }
            _ => {}
        }
        Ok(())
    }

    fn notification(&mut self, io: &mut dyn AdapterIo, frame: &Value) -> Result<(), String> {
        let params = &frame["params"];
        if !self.belongs_to_turn(params) {
            return Ok(());
        }
        match frame.get("method").and_then(Value::as_str) {
            Some("mcpServer/startupStatus/updated") => {
                if params.get("name").and_then(Value::as_str) == Some(SERVER) {
                    match params.get("status").and_then(Value::as_str) {
                        Some("failed" | "cancelled") => {
                            let detail = params
                                .get("error")
                                .and_then(Value::as_str)
                                .unwrap_or("inicialização interrompida");
                            return Err(format!(
                                "Não foi possível conectar o MCP ivestfii: {detail}"
                            ));
                        }
                        Some("ready") => io.emit(ProviderEvent::Status {
                            message: "MCP local conectado ao Codex.".into(),
                        })?,
                        _ => {}
                    }
                }
            }
            Some("error") => {
                let detail = error_message(&params["error"]);
                if params.get("willRetry").and_then(Value::as_bool) == Some(true) {
                    io.emit(ProviderEvent::Status {
                        message: format!("Codex vai tentar novamente: {detail}"),
                    })?;
                } else {
                    return Err(format!("Codex: {detail}"));
                }
            }
            Some("item/agentMessage/delta") => {
                let text = params
                    .get("delta")
                    .and_then(Value::as_str)
                    .ok_or("Codex enviou fragmento de texto inválido.")?;
                io.emit(ProviderEvent::Delta {
                    text: text.to_owned(),
                })?;
            }
            Some("item/started") => self.record_item(io, &params["item"], false)?,
            Some("item/completed") => self.record_item(io, &params["item"], true)?,
            Some("item/mcpToolCall/progress") => {
                if let Some(message) = params.get("message").and_then(Value::as_str) {
                    io.emit(ProviderEvent::Status {
                        message: message.to_owned(),
                    })?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn final_text(&self) -> Result<String, String> {
        let final_messages: Vec<&str> = self
            .completed_messages
            .iter()
            .filter(|message| message.phase.as_deref() == Some("final_answer"))
            .map(|message| message.text.as_str())
            .filter(|text| !text.trim().is_empty())
            .collect();
        if !final_messages.is_empty() {
            return Ok(final_messages.join("\n\n"));
        }
        // Older models omit phase. A completed message is authoritative;
        // deltas alone never prove a successful final response.
        self.completed_messages
            .iter()
            .rev()
            .find(|message| message.phase.is_none() && !message.text.trim().is_empty())
            .map(|message| message.text.clone())
            .ok_or_else(|| "Codex encerrou sem uma resposta final confirmada.".into())
    }
}

fn await_response(io: &mut dyn AdapterIo, state: &mut State, id: u64) -> Result<Value, String> {
    loop {
        let frame = io.receive()?;
        if frame.get("method").is_some() && frame.get("id").is_some() {
            refuse_request(io, &frame)?;
        } else if frame.get("id").and_then(Value::as_u64) == Some(id) {
            if let Some(error) = frame.get("error") {
                return Err(format!("Codex: {}", error_message(error)));
            }
            return frame
                .get("result")
                .cloned()
                .ok_or_else(|| "Resposta inválida do Codex.".into());
        } else {
            state.notification(io, &frame)?;
        }
    }
}

fn verify_tools(result: &Value) -> Result<(), String> {
    let servers = result
        .get("data")
        .and_then(Value::as_array)
        .ok_or("Codex não confirmou o inventário de ferramentas MCP.")?;
    if result
        .get("nextCursor")
        .is_some_and(|cursor| !cursor.is_null())
    {
        return Err("O Codex precisa iniciar com apenas o MCP ivestfii habilitado.".into());
    }
    // Codex includes the explicitly disabled host node_repl in this response.
    // Exempt only that known server with no tools/resources; an unrelated MCP
    // exposing resources alone must not bypass the inventory check.
    let active: Vec<&Value> = servers
        .iter()
        .filter(|server| {
            !(server.get("name").and_then(Value::as_str) == Some("node_repl")
                && server
                    .get("tools")
                    .and_then(Value::as_object)
                    .is_some_and(|tools| tools.is_empty())
                && ["resources", "resourceTemplates"].iter().all(|field| {
                    server
                        .get(field)
                        .is_none_or(|value| value.as_array().is_some_and(|items| items.is_empty()))
                }))
        })
        .collect();
    if active.len() != 1 || active[0].get("name").and_then(Value::as_str) != Some(SERVER) {
        return Err("O Codex precisa iniciar com apenas o MCP ivestfii habilitado.".into());
    }
    let tools = active[0]
        .get("tools")
        .and_then(Value::as_object)
        .ok_or("O MCP ivestfii não disponibilizou ferramentas no Codex.")?;
    let available: HashSet<&str> = tools
        .values()
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .collect();
    if available.len() != TOOLS.len() || TOOLS.iter().any(|name| !available.contains(name)) {
        return Err(
            "O MCP ivestfii não confirmou as cinco ferramentas de leitura esperadas.".into(),
        );
    }
    Ok(())
}

fn select_model(
    config: &AdapterConfig,
    io: &mut dyn AdapterIo,
    state: &mut State,
) -> Result<(String, Option<String>), String> {
    let mut models = Vec::new();
    let mut cursor = Value::Null;
    let mut seen_cursors = HashSet::new();
    loop {
        io.send(&json!({"id":2,"method":"model/list","params":{
            "includeHidden":true,"limit":100,"cursor":cursor
        }}))?;
        let page = await_response(io, state, 2)?;
        let entries = page
            .get("data")
            .and_then(Value::as_array)
            .ok_or("Codex não retornou uma lista de modelos válida.")?;
        models.extend(entries.iter().cloned());
        cursor = page.get("nextCursor").cloned().unwrap_or(Value::Null);
        if cursor.is_null() {
            break;
        }
        if models.len() > 1_000 || !seen_cursors.insert(cursor.to_string()) {
            return Err("Codex retornou paginação inválida ao listar modelos.".into());
        }
    }
    let selected = match config.model.as_deref() {
        Some(requested) => models.iter().find(|entry| {
            entry.get("model").and_then(Value::as_str) == Some(requested)
                || entry.get("id").and_then(Value::as_str) == Some(requested)
        }),
        None => models
            .iter()
            .find(|entry| entry.get("isDefault").and_then(Value::as_bool) == Some(true)),
    };
    let Some(selected) = selected else {
        let available = models
            .iter()
            .filter_map(|entry| entry.get("model").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(", ");
        let reason = match config.model.as_deref() {
            Some(requested) => {
                format!("O modelo '{requested}' não foi anunciado pela CLI Codex instalada.")
            }
            None => "A CLI Codex não anunciou um modelo padrão.".into(),
        };
        return Err(format!("{reason} Modelos disponíveis: {available}."));
    };
    let model = selected
        .get("model")
        .and_then(Value::as_str)
        .filter(|model| !model.is_empty())
        .ok_or("Codex anunciou um identificador de modelo inválido.")?
        .to_owned();
    let effort = selected
        .get("defaultReasoningEffort")
        .and_then(Value::as_str)
        .map(str::to_owned);
    io.emit(ProviderEvent::Status {
        message: format!("Codex: modelo {model}. Histórico e ferramentas serão conectados."),
    })?;
    Ok((model, effort))
}

pub fn run(config: &AdapterConfig, io: &mut dyn AdapterIo) -> Result<AdapterResult, String> {
    let mut state = State::default();
    io.emit(ProviderEvent::Status {
        message: "Conectando ao Codex app-server…".into(),
    })?;
    io.send(&json!({"id":1,"method":"initialize","params":{
        "clientInfo":{"name":"ivestfii","title":"IvestFII","version":env!("CARGO_PKG_VERSION")},
        "capabilities":{"experimentalApi":false}
    }}))?;
    await_response(io, &mut state, 1)?;
    io.send(&json!({"method":"initialized","params":{}}))?;
    let (model, effort) = select_model(config, io, &mut state)?;
    let params = json!({
        "model":model,
        "cwd":config.work_dir,
        "approvalPolicy":"never",
        "sandbox":"read-only",
        "ephemeral":true,
        "serviceName":"ivestfii",
        "baseInstructions":format!("Você é o assistente da carteira IvestFII. Responda em português. Use exclusivamente as cinco ferramentas MCP do servidor ivestfii para consultar dados e executar simulações. O histórico canônico é fornecido pelo aplicativo no contexto da mensagem. Não execute comandos, não leia arquivos diretamente, não acesse ferramentas de outros serviços e não modifique dados. Quando uma fonte estiver indisponível, informe a limitação sem inventar valores. Se precisar de esclarecimento, escreva uma pergunta na resposta final. {}", crate::analysis_instructions::POLICY)
    });
    io.send(&json!({"id":3,"method":"thread/start","params":params}))?;
    let started = await_response(io, &mut state, 3)?;
    let thread_id = started
        .pointer("/thread/id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or("Codex não retornou o identificador da conversa.")?
        .to_owned();
    state.thread_id = Some(thread_id.clone());
    io.send(&json!({"id":4,"method":"mcpServerStatus/list","params":{
        "threadId":thread_id,"detail":"toolsAndAuthOnly","limit":100
    }}))?;
    let tools = await_response(io, &mut state, 4)?;
    verify_tools(&tools)?;
    io.emit(ProviderEvent::Status {
        message: "Codex conectado com histórico e ferramentas locais.".into(),
    })?;
    let mut turn_params = json!({
        "threadId":thread_id,
        "input":[{"type":"text","text":config.context,"text_elements":[]}]
    });
    if let Some(effort) = effort {
        turn_params["effort"] = Value::String(effort);
    }
    io.send(&json!({"id":5,"method":"turn/start","params":turn_params}))?;
    loop {
        let frame = io.receive()?;
        if frame.get("method").is_some() && frame.get("id").is_some() {
            refuse_request(io, &frame)?;
            continue;
        }
        if frame.get("id").and_then(Value::as_u64) == Some(5) {
            if let Some(error) = frame.get("error") {
                return Err(format!("Codex: {}", error_message(error)));
            }
            if let Some(id) = frame.pointer("/result/turn/id").and_then(Value::as_str) {
                state.turn_id = Some(id.to_owned());
            }
            continue;
        }
        let params = &frame["params"];
        if !state.belongs_to_turn(params) {
            continue;
        }
        state.notification(io, &frame)?;
        match frame.get("method").and_then(Value::as_str) {
            Some("turn/started") => {
                if let Some(id) = params.pointer("/turn/id").and_then(Value::as_str) {
                    state.turn_id = Some(id.to_owned());
                }
            }
            Some("turn/completed") => {
                let turn = &params["turn"];
                // turn/completed uses turn.id rather than params.turnId.
                if state.turn_id.as_deref().is_some_and(|expected| {
                    turn.get("id").and_then(Value::as_str) != Some(expected)
                }) {
                    continue;
                }
                match turn.get("status").and_then(Value::as_str) {
                    Some("completed") => {
                        if let Some(items) = turn.get("items").and_then(Value::as_array) {
                            for item in items {
                                state.record_item(io, item, true)?;
                            }
                        }
                        return Ok(AdapterResult {
                            text: state.final_text()?,
                            session_id: Some(thread_id),
                        });
                    }
                    Some("interrupted" | "cancelled") => {
                        return Err("A resposta do Codex foi interrompida.".into())
                    }
                    Some("failed") => {
                        return Err(format!("Codex: {}", error_message(&turn["error"])))
                    }
                    _ => return Err("Codex encerrou com estado de execução inválido.".into()),
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, path::PathBuf};

    #[derive(Default)]
    struct FakeIo {
        incoming: VecDeque<Value>,
        sent: Vec<Value>,
        events: Vec<ProviderEvent>,
    }

    impl AdapterIo for FakeIo {
        fn send(&mut self, value: &Value) -> Result<(), String> {
            self.sent.push(value.clone());
            Ok(())
        }
        fn receive(&mut self) -> Result<Value, String> {
            self.incoming
                .pop_front()
                .ok_or_else(|| "EOF de teste".into())
        }
        fn emit(&mut self, event: ProviderEvent) -> Result<(), String> {
            self.events.push(event);
            Ok(())
        }
    }

    fn config() -> AdapterConfig {
        AdapterConfig {
            executable: PathBuf::from("codex.exe"),
            work_dir: PathBuf::from("C:/IvestFII/chat"),
            mcp_executable: PathBuf::from("C:/Program Files/IvestFII/ivestfii-mcp.exe"),
            mcp_arguments: vec!["--data-dir".into(), "C:/IvestFII/dados".into()],
            data_dir: PathBuf::from("C:/IvestFII/dados"),
            conversation_id: "conversation-1".into(),
            context: "Histórico anterior: objetivo de renda. Pergunta: como estamos?".into(),
            model: Some("modelo-configurado".into()),
        }
    }

    fn inventory() -> Value {
        let mut tools = serde_json::Map::new();
        for name in TOOLS {
            tools.insert(name.into(), json!({"name":name}));
        }
        json!({"data":[{"name":"ivestfii","tools":tools}],"nextCursor":null})
    }

    fn ready(tail: Vec<Value>) -> FakeIo {
        let mut messages = vec![
            json!({"id":1,"result":{}}),
            json!({"id":2,"result":{"data":[{"id":"modelo-configurado","model":"modelo-configurado","isDefault":true,"defaultReasoningEffort":"medium"}],"nextCursor":null}}),
            json!({"method":"mcpServer/startupStatus/updated","params":{"name":"ivestfii","status":"ready"}}),
            json!({"id":3,"result":{"thread":{"id":"thread-1"}}}),
            json!({"id":4,"result":inventory()}),
            json!({"id":5,"result":{"turn":{"id":"turn-1"}}}),
        ];
        messages.extend(tail);
        FakeIo {
            incoming: messages.into(),
            ..FakeIo::default()
        }
    }

    fn completed(status: &str, items: Vec<Value>) -> Value {
        json!({"method":"turn/completed","params":{"threadId":"thread-1","turn":{
            "id":"turn-1","status":status,"items":items,"error":{"message":"limite atingido"}
        }}})
    }

    fn answer(text: &str, phase: Option<&str>) -> Value {
        json!({"type":"agentMessage","id":"answer-1","text":text,"phase":phase})
    }

    #[test]
    fn launch_isolates_mcp_and_does_not_put_context_in_arguments() {
        let config = config();
        let spec = launch(&config).unwrap();
        assert!(spec
            .args
            .iter()
            .any(|arg| arg.contains("mcp_servers={ivestfii=")));
        assert!(spec.args.iter().any(|arg| arg.contains("required=true")));
        assert!(spec
            .args
            .iter()
            .any(|arg| arg.contains("C:/Program Files/IvestFII/ivestfii-mcp.exe")));
        assert!(spec
            .args
            .windows(2)
            .any(|pair| pair == ["--disable", "shell_tool"]));
        assert!(spec
            .args
            .windows(2)
            .any(|pair| pair == ["--enable", "code_mode_host"]));
        assert!(!spec.args.iter().any(|arg| arg.contains(&config.context)));
    }

    #[test]
    fn replays_canonical_context_in_a_new_read_only_thread() {
        let config = config();
        let mut io = ready(vec![completed(
            "completed",
            vec![answer("Análise pronta.", Some("final_answer"))],
        )]);
        let result = run(&config, &mut io).unwrap();
        assert_eq!(result.text, "Análise pronta.");
        assert_eq!(result.session_id.as_deref(), Some("thread-1"));
        let start = io
            .sent
            .iter()
            .find(|message| message["method"] == "thread/start")
            .unwrap();
        assert_eq!(start["params"]["sandbox"], "read-only");
        assert_eq!(start["params"]["ephemeral"], true);
        assert_eq!(start["params"]["model"], "modelo-configurado");
        crate::analysis_instructions::assert_policy(
            start["params"]["baseInstructions"].as_str().unwrap(),
        );
        let turn = io
            .sent
            .iter()
            .find(|message| message["method"] == "turn/start")
            .unwrap();
        assert_eq!(turn["params"]["input"][0]["text"], config.context);
        assert_eq!(turn["params"]["effort"], "medium");
        assert!(!io
            .sent
            .iter()
            .any(|message| message["method"] == "thread/resume"));
    }

    #[test]
    fn chooses_advertised_default_when_no_model_was_requested() {
        let mut config = config();
        config.model = None;
        let mut io = ready(vec![completed("completed", vec![answer("Pronto.", None)])]);
        run(&config, &mut io).unwrap();
        let start = io
            .sent
            .iter()
            .find(|frame| frame["method"] == "thread/start")
            .unwrap();
        assert_eq!(start["params"]["model"], "modelo-configurado");
    }

    #[test]
    fn unsupported_explicit_model_fails_before_submitting_context() {
        let mut config = config();
        config.model = Some("modelo-indisponivel".into());
        let mut io = ready(vec![]);
        let error = run(&config, &mut io).unwrap_err();
        assert!(error.contains("modelo-indisponivel"));
        assert!(error.contains("modelo-configurado"));
        assert!(!io
            .sent
            .iter()
            .any(|frame| frame["method"] == "thread/start"));
    }

    #[test]
    fn reads_all_model_catalog_pages() {
        let mut io = ready(vec![completed("completed", vec![answer("Pronto.", None)])]);
        io.incoming.insert(
            1,
            json!({"id":2,"result":{"data":[{
            "id":"outro-modelo","model":"outro-modelo","isDefault":false
        }],"nextCursor":"page-2"}}),
        );
        run(&config(), &mut io).unwrap();
        let requests: Vec<_> = io
            .sent
            .iter()
            .filter(|frame| frame["method"] == "model/list")
            .collect();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1]["params"]["cursor"], "page-2");
    }

    #[test]
    fn streams_text_and_emits_each_mcp_event_once() {
        let tool = json!({"type":"mcpToolCall","id":"tool-1","server":"ivestfii",
            "tool":"carteira_snapshot","arguments":{},"status":"completed",
            "result":{"content":[{"type":"text","text":"{}"}]}});
        let mut io = ready(vec![
            json!({"method":"item/started","params":{"threadId":"thread-1","turnId":"turn-1","item":tool}}),
            json!({"method":"item/completed","params":{"threadId":"thread-1","turnId":"turn-1","item":tool}}),
            json!({"method":"item/agentMessage/delta","params":{"threadId":"thread-1","turnId":"turn-1","delta":"Carteira"}}),
            completed(
                "completed",
                vec![tool, answer("Carteira conferida.", Some("final_answer"))],
            ),
        ]);
        assert_eq!(run(&config(), &mut io).unwrap().text, "Carteira conferida.");
        assert_eq!(
            io.events
                .iter()
                .filter(|e| matches!(e, ProviderEvent::ToolStarted { .. }))
                .count(),
            1
        );
        assert_eq!(
            io.events
                .iter()
                .filter(|e| matches!(
                    e,
                    ProviderEvent::ToolFinished {
                        is_error: false,
                        ..
                    }
                ))
                .count(),
            1
        );
        assert!(io
            .events
            .iter()
            .any(|e| matches!(e, ProviderEvent::Delta { text } if text == "Carteira")));
    }

    #[test]
    fn declines_approvals_and_unknown_interactions_without_hanging() {
        let mut io = ready(vec![
            json!({"id":"approval-1","method":"item/commandExecution/requestApproval","params":{}}),
            json!({"id":"question-1","method":"item/tool/requestUserInput","params":{}}),
            json!({"id":"elicitation-1","method":"mcpServer/elicitation/request","params":{}}),
            completed("completed", vec![answer("Preciso de mais detalhes.", None)]),
        ]);
        run(&config(), &mut io).unwrap();
        assert!(io
            .sent
            .iter()
            .any(|v| v["id"] == "approval-1" && v["result"]["decision"] == "decline"));
        assert!(io
            .sent
            .iter()
            .any(|v| v["id"] == "question-1" && v["error"]["code"] == -32601));
        assert!(io
            .sent
            .iter()
            .any(|v| v["id"] == "elicitation-1" && v["result"]["action"] == "decline"));
    }

    #[test]
    fn analysis_tool_keeps_refresh_arguments_and_profile_result_in_audit() {
        let arguments = json!({"conversationId":"canonical","refreshPublicDataIfDue":true});
        let tool = json!({"type":"mcpToolCall","id":"analysis-1","server":"ivestfii",
            "tool":"preparar_analise_carteira","arguments":arguments,"status":"completed",
            "result":{"structuredContent":{"profileCompleteness":{"canPersonalize":false}},"content":[]}});
        let mut io = ready(vec![completed(
            "completed",
            vec![tool, answer("Informe seus objetivos.", None)],
        )]);
        run(&config(), &mut io).unwrap();
        assert!(io.events.iter().any(|event| matches!(event, ProviderEvent::ToolStarted {name,input,..} if name == "preparar_analise_carteira" && input == &arguments)));
        assert!(io.events.iter().any(|event| matches!(event, ProviderEvent::ToolFinished {output,..} if output["structuredContent"]["profileCompleteness"]["canPersonalize"] == false)));
    }

    #[test]
    fn contribution_simulation_is_registered_and_preserves_budget_result_in_audit() {
        let arguments = json!({"walletId":"wallet-2","amount":"300.00","allocations":{"fiis":"100.00","fixedIncome":"50.00","cashPockets":"150.00"}});
        let output = json!({"content":[],"structuredContent":{"status":"simulation","kind":"contribution-allocation","simulation":{"budget":"300.00"},"marker":"unchanged-result"}});
        let tool = json!({"type":"mcpToolCall","id":"contribution-1","server":"ivestfii",
            "tool":"simular_aporte","arguments":arguments,"status":"completed","result":output});
        let mut io = ready(vec![completed(
            "completed",
            vec![
                tool,
                answer("Cenário por classe calculado, sem alterar saldos.", None),
            ],
        )]);
        run(&config(), &mut io).unwrap();
        assert!(launch(&config())
            .unwrap()
            .args
            .iter()
            .any(|arg| arg.contains("simular_aporte")));
        assert!(io.events.iter().any(|event| matches!(event, ProviderEvent::ToolStarted {name,input,..} if name == "simular_aporte" && input == &arguments)));
        assert!(io.events.iter().any(|event| matches!(event, ProviderEvent::ToolFinished {name,output:actual,is_error:false,..} if name == "simular_aporte" && actual == &output)));
        let mut old_inventory = inventory();
        old_inventory["data"][0]["tools"]
            .as_object_mut()
            .unwrap()
            .remove("simular_aporte");
        assert!(verify_tools(&old_inventory).is_err());
    }

    #[test]
    fn failed_or_interrupted_turn_never_succeeds_from_partial_text() {
        for status in ["failed", "interrupted"] {
            let mut io = ready(vec![completed(
                status,
                vec![answer("Resposta parcial", None)],
            )]);
            assert!(run(&config(), &mut io).is_err());
        }
    }

    #[test]
    fn commentary_or_deltas_alone_do_not_count_as_final_answer() {
        let mut io = ready(vec![
            json!({"method":"item/agentMessage/delta","params":{"delta":"parcial"}}),
            completed(
                "completed",
                vec![answer("Vou verificar.", Some("commentary"))],
            ),
        ]);
        assert!(run(&config(), &mut io)
            .unwrap_err()
            .contains("resposta final"));
    }

    #[test]
    fn refuses_missing_or_foreign_mcp_before_sending_prompt() {
        for result in [
            json!({"data":[]}),
            json!({"data":[{"name":"outro","tools":{}}]}),
        ] {
            let mut io = ready(vec![]);
            io.incoming[4] = json!({"id":4,"result":result});
            assert!(run(&config(), &mut io).is_err());
            assert!(!io.sent.iter().any(|frame| frame["method"] == "turn/start"));
        }
    }

    #[test]
    fn disabled_host_mcp_with_no_tools_is_accepted() {
        let mut inventory = inventory();
        inventory["data"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name":"node_repl","tools":{}}));
        assert!(verify_tools(&inventory).is_ok());
        inventory["data"][1]["tools"] = json!({"js":{"name":"js"}});
        assert!(verify_tools(&inventory).is_err());
        inventory["data"][1] = json!({"name":"external-resources","tools":{},"resources":[{"uri":"secret://resource"}]});
        assert!(verify_tools(&inventory).is_err());
    }

    #[test]
    fn reports_mcp_startup_failure() {
        let mut io = ready(vec![]);
        io.incoming[2] = json!({"method":"mcpServer/startupStatus/updated","params":{
            "name":"ivestfii","status":"failed","error":"banco indisponível"
        }});
        assert!(run(&config(), &mut io)
            .unwrap_err()
            .contains("banco indisponível"));
    }

    #[test]
    fn reports_non_retrying_error_but_allows_transient_retries() {
        let mut io = ready(vec![
            json!({"method":"error","params":{"error":{"message":"rede instável"},"willRetry":true}}),
            completed("completed", vec![answer("Pronto", None)]),
        ]);
        assert_eq!(run(&config(), &mut io).unwrap().text, "Pronto");
        let mut io = ready(vec![json!({"method":"error","params":{
            "error":{"message":"Faça login na CLI"},"willRetry":false
        }})]);
        assert!(run(&config(), &mut io).unwrap_err().contains("Faça login"));
    }

    #[test]
    fn rejects_foreign_tools_and_marks_local_tool_failures() {
        let failed = json!({"type":"mcpToolCall","id":"tool-1","server":"ivestfii",
            "tool":"projetar_renda","arguments":{},"status":"failed","error":{"message":"premissas inválidas"}});
        let mut io = ready(vec![completed(
            "completed",
            vec![failed, answer("Informe as premissas.", None)],
        )]);
        run(&config(), &mut io).unwrap();
        assert!(io
            .events
            .iter()
            .any(|e| matches!(e, ProviderEvent::ToolFinished { is_error: true, .. })));
        let mut io = ready(vec![completed(
            "completed",
            vec![json!({"type":"mcpToolCall",
            "id":"foreign","server":"outro","tool":"comando","status":"completed"})],
        )]);
        assert!(run(&config(), &mut io).unwrap_err().contains("fora do MCP"));
    }

    #[test]
    fn eof_before_confirmed_completion_is_error() {
        let mut io = ready(vec![json!({"method":"item/completed","params":{
            "item":answer("Texto recebido antes da queda", None)
        }})]);
        assert!(run(&config(), &mut io).unwrap_err().contains("EOF"));
    }
}
