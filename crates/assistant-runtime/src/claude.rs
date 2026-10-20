//! Claude Code's documented NDJSON print protocol. The application owns history;
//! every invocation receives a fresh rendering of its canonical conversation.
use crate::{AdapterConfig, AdapterIo, AdapterResult, LaunchSpec, ProviderEvent};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
};

pub(crate) const TOOLS: [&str; 4] = [
    "carteira_snapshot",
    "projetar_renda",
    "recuperar_contexto",
    "preparar_analise_carteira",
];
const MAX_TEXT: usize = 1_048_576;

pub fn launch(config: &AdapterConfig) -> Result<LaunchSpec, String> {
    fs::create_dir_all(&config.work_dir).map_err(|e| format!("Diretório do Claude: {e}"))?;
    let mcp_path = config.work_dir.join("claude-mcp.json");
    let mcp = json!({"mcpServers":{"ivestfii":{
        "type":"stdio", "command":config.mcp_executable, "args":config.mcp_arguments,
    }}});
    fs::write(
        &mcp_path,
        serde_json::to_vec_pretty(&mcp).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("Configuração MCP do Claude: {e}"))?;
    let mut args = vec![
        "--print".into(),
        "--input-format".into(),
        "stream-json".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--include-partial-messages".into(),
        "--no-session-persistence".into(),
        "--restricted".into(),
        "--disable-slash-commands".into(),
        "--no-chrome".into(),
        "--tools".into(),
        String::new(),
        "--strict-mcp-config".into(),
        "--mcp-config".into(),
        mcp_path.to_string_lossy().into_owned(),
        "--setting-sources".into(),
        String::new(),
        "--settings".into(),
        json!({"disableAllHooks":true}).to_string(),
        "--permission-mode".into(),
        "dontAsk".into(),
        "--permission-prompts".into(),
        "none".into(),
        "--allowedTools".into(),
        TOOLS
            .iter()
            .map(|t| format!("mcp__ivestfii__{t}"))
            .collect::<Vec<_>>()
            .join(","),
    ];
    if let Some(model) = &config.model {
        args.extend(["--model".into(), model.clone()]);
    }
    Ok(LaunchSpec {
        executable: config.executable.clone(),
        args,
        environment: vec![(
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".into(),
            "1".into(),
        )],
    })
}

#[derive(Default)]
struct ToolBlock {
    id: String,
    name: String,
    input: Value,
    partial: String,
}

fn checked_tool(name: &str) -> Result<(), String> {
    if TOOLS
        .iter()
        .any(|tool| name == format!("mcp__ivestfii__{tool}"))
    {
        Ok(())
    } else {
        Err(format!(
            "O Claude solicitou uma ferramenta fora do MCP da carteira: {name}"
        ))
    }
}

fn start_tool(
    io: &mut dyn AdapterIo,
    started: &mut HashMap<String, String>,
    tool: &ToolBlock,
) -> Result<(), String> {
    checked_tool(&tool.name)?;
    if !started.contains_key(&tool.id) {
        let input = if tool.partial.is_empty() {
            tool.input.clone()
        } else {
            serde_json::from_str(&tool.partial)
                .map_err(|e| format!("Argumentos da ferramenta Claude inválidos: {e}"))?
        };
        io.emit(ProviderEvent::ToolStarted {
            call_id: tool.id.clone(),
            name: tool.name.clone(),
            input,
        })?;
        started.insert(tool.id.clone(), tool.name.clone());
    }
    Ok(())
}

fn append_text(io: &mut dyn AdapterIo, aggregate: &mut String, text: &str) -> Result<(), String> {
    if aggregate.len().saturating_add(text.len()) > MAX_TEXT {
        return Err("Resposta do Claude excedeu o limite de tamanho.".into());
    }
    aggregate.push_str(text);
    if !text.is_empty() {
        io.emit(ProviderEvent::Delta { text: text.into() })?;
    }
    Ok(())
}

pub fn run(config: &AdapterConfig, io: &mut dyn AdapterIo) -> Result<AdapterResult, String> {
    io.send(
        &json!({"type":"user", "message":{"role":"user", "content":format!("{}\n\n{}", crate::analysis_instructions::POLICY, config.context)},
        "parent_tool_use_id":null, "session_id":""}),
    )?;
    let mut session_id = None;
    let mut aggregate = String::new();
    let mut current_streamed = String::new();
    let mut blocks: HashMap<u64, ToolBlock> = HashMap::new();
    let mut started = HashMap::new();
    let mut finished = HashSet::new();
    loop {
        let value = io.receive()?;
        if let Some(id) = value.get("session_id").and_then(Value::as_str) {
            session_id = Some(id.to_owned());
        }
        match value.get("type").and_then(Value::as_str).unwrap_or("") {
            "system" => {
                if value["subtype"] == "init" {
                    if let Some(servers) = value["mcp_servers"].as_array() {
                        if !servers
                            .iter()
                            .any(|s| s["name"] == "ivestfii" && s["status"] == "connected")
                        {
                            return Err(
                                "O Claude não conseguiu conectar ao MCP local da carteira.".into(),
                            );
                        }
                    }
                    io.emit(ProviderEvent::Status {
                        message: "Claude conectado; histórico do aplicativo enviado.".into(),
                    })?;
                }
            }
            "stream_event" => {
                let event = &value["event"];
                let index = event["index"].as_u64().unwrap_or(0);
                match event["type"].as_str().unwrap_or("") {
                    "message_start" => current_streamed.clear(),
                    "content_block_start" => {
                        let block = &event["content_block"];
                        if block["type"] == "tool_use" {
                            let name = block["name"].as_str().unwrap_or("").to_owned();
                            checked_tool(&name)?;
                            blocks.insert(
                                index,
                                ToolBlock {
                                    id: block["id"].as_str().unwrap_or("").into(),
                                    name,
                                    input: block.get("input").cloned().unwrap_or_else(|| json!({})),
                                    partial: String::new(),
                                },
                            );
                        } else if block["type"] == "text" {
                            let text = block["text"].as_str().unwrap_or("");
                            append_text(io, &mut aggregate, text)?;
                            current_streamed.push_str(text);
                        }
                    }
                    "content_block_delta" => {
                        let delta = &event["delta"];
                        if delta["type"] == "text_delta" {
                            let text = delta["text"].as_str().unwrap_or("");
                            append_text(io, &mut aggregate, text)?;
                            current_streamed.push_str(text);
                        } else if delta["type"] == "input_json_delta" {
                            if let Some(tool) = blocks.get_mut(&index) {
                                let partial = delta["partial_json"].as_str().unwrap_or("");
                                if tool.partial.len().saturating_add(partial.len()) > MAX_TEXT {
                                    return Err(
                                        "Argumentos MCP do Claude excederam o limite.".into()
                                    );
                                }
                                tool.partial.push_str(partial);
                            }
                        }
                    }
                    "content_block_stop" => {
                        if let Some(tool) = blocks.remove(&index) {
                            start_tool(io, &mut started, &tool)?;
                        }
                    }
                    _ => {}
                }
            }
            "assistant" => {
                if let Some(error) = value.get("error").filter(|v| !v.is_null()) {
                    return Err(format!(
                        "Claude: {}",
                        error.as_str().unwrap_or("erro do provedor")
                    ));
                }
                if let Some(content) = value["message"]["content"].as_array() {
                    let mut full_text = String::new();
                    for block in content {
                        match block["type"].as_str().unwrap_or("") {
                            "text" => full_text.push_str(block["text"].as_str().unwrap_or("")),
                            "tool_use" => start_tool(
                                io,
                                &mut started,
                                &ToolBlock {
                                    id: block["id"].as_str().unwrap_or("").into(),
                                    name: block["name"].as_str().unwrap_or("").into(),
                                    input: block.get("input").cloned().unwrap_or_else(|| json!({})),
                                    partial: String::new(),
                                },
                            )?,
                            _ => {}
                        }
                    }
                    // The full assistant message repeats partial stream events.
                    if current_streamed.is_empty() {
                        append_text(io, &mut aggregate, &full_text)?;
                    } else if let Some(suffix) = full_text.strip_prefix(&current_streamed) {
                        append_text(io, &mut aggregate, suffix)?;
                    }
                    current_streamed.clear();
                }
            }
            "user" => {
                if let Some(content) = value["message"]["content"].as_array() {
                    for block in content {
                        if block["type"] == "tool_result" {
                            let id = block["tool_use_id"].as_str().unwrap_or("");
                            let name = started
                                .get(id)
                                .ok_or("O Claude retornou resultado de ferramenta desconhecida.")?;
                            if finished.insert(id.to_owned()) {
                                io.emit(ProviderEvent::ToolFinished {
                                    call_id: id.into(),
                                    name: name.clone(),
                                    output: block.get("content").cloned().unwrap_or(Value::Null),
                                    is_error: block["is_error"].as_bool().unwrap_or(false),
                                })?;
                            }
                        }
                    }
                }
            }
            "result" => {
                if value["is_error"].as_bool().unwrap_or(false)
                    || value["subtype"].as_str().is_some_and(|s| s != "success")
                {
                    let detail = value["errors"]
                        .as_array()
                        .map(|errors| {
                            errors
                                .iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join("; ")
                        })
                        .filter(|s| !s.is_empty())
                        .or_else(|| value["result"].as_str().map(str::to_owned))
                        .unwrap_or_else(|| {
                            value["subtype"]
                                .as_str()
                                .unwrap_or("erro do provedor")
                                .into()
                        });
                    return Err(format!("Claude: {detail}"));
                }
                if value["permission_denials"]
                    .as_array()
                    .is_some_and(|v| !v.is_empty())
                {
                    return Err("O Claude negou uma ferramenta MCP. Confira a compatibilidade das permissões da CLI instalada.".into());
                }
                if started.keys().any(|id| !finished.contains(id)) {
                    return Err(
                        "O Claude encerrou sem confirmar o resultado de uma ferramenta MCP.".into(),
                    );
                }
                let text = value["result"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or(&aggregate)
                    .to_owned();
                if text.trim().is_empty() {
                    return Err("O Claude terminou sem uma resposta.".into());
                }
                if text.len() > MAX_TEXT {
                    return Err("Resposta do Claude excedeu o limite de tamanho.".into());
                }
                if aggregate.is_empty() {
                    io.emit(ProviderEvent::Delta { text: text.clone() })?;
                }
                return Ok(AdapterResult { text, session_id });
            }
            "control_request" => {
                // No interactive approvals are permitted in this read-only connector.
                if let Some(request_id) = value.get("request_id") {
                    io.send(&json!({"type":"control_response","response":{"subtype":"success",
                        "request_id":request_id,"response":{"behavior":"deny","message":"Somente ferramentas MCP de leitura da carteira são permitidas."}}}))?;
                }
                return Err(
                    "O Claude solicitou uma permissão interativa fora do conector MCP.".into(),
                );
            }
            "error" => {
                return Err(format!(
                    "Claude: {}",
                    value["message"].as_str().unwrap_or("erro no protocolo")
                ))
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
        input: VecDeque<Value>,
        sent: Vec<Value>,
        events: Vec<ProviderEvent>,
    }
    impl AdapterIo for FakeIo {
        fn send(&mut self, v: &Value) -> Result<(), String> {
            self.sent.push(v.clone());
            Ok(())
        }
        fn receive(&mut self) -> Result<Value, String> {
            self.input.pop_front().ok_or_else(|| "EOF".into())
        }
        fn emit(&mut self, v: ProviderEvent) -> Result<(), String> {
            self.events.push(v);
            Ok(())
        }
    }
    fn config() -> AdapterConfig {
        AdapterConfig {
            executable: PathBuf::from("claude.exe"),
            work_dir: PathBuf::new(),
            mcp_executable: PathBuf::from("ivestfii.exe"),
            mcp_arguments: vec!["--mcp".into()],
            data_dir: PathBuf::new(),
            conversation_id: "local-conversation".into(),
            context: "Histórico entre provedores; pergunta atual".into(),
            model: None,
        }
    }
    #[test]
    fn streams_text_without_repeating_final_message() {
        let mut io = FakeIo {
            input: VecDeque::from([
                json!({"type":"system","subtype":"init","session_id":"native-session","mcp_servers":[{"name":"ivestfii","status":"connected"}]}),
                json!({"type":"stream_event","event":{"type":"message_start"}}),
                json!({"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Olá"}}}),
                json!({"type":"assistant","message":{"content":[{"type":"text","text":"Olá"}]}}),
                json!({"type":"result","subtype":"success","result":"Olá","is_error":false}),
            ]),
            ..Default::default()
        };
        let result = run(&config(), &mut io).unwrap();
        assert_eq!(result.text, "Olá");
        assert_eq!(result.session_id.as_deref(), Some("native-session"));
        let prompt = io.sent[0]["message"]["content"].as_str().unwrap();
        assert!(prompt.ends_with(&config().context));
        crate::analysis_instructions::assert_policy(prompt);
        assert_eq!(
            io.events
                .iter()
                .filter(|e| matches!(e, ProviderEvent::Delta { .. }))
                .count(),
            1
        );
    }
    #[test]
    fn streams_mcp_arguments_and_audits_result_once() {
        let mut io = FakeIo {
            input: VecDeque::from([
                json!({"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t1","name":"mcp__ivestfii__carteira_snapshot","input":{}}}}),
                json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{}"}}}),
                json!({"type":"stream_event","event":{"type":"content_block_stop","index":0}}),
                json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"mcp__ivestfii__carteira_snapshot","input":{}}]}}),
                json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"{\"funds\":[]}"}]}]}}),
                json!({"type":"result","subtype":"success","result":"Carteira vazia."}),
            ]),
            ..Default::default()
        };
        assert_eq!(run(&config(), &mut io).unwrap().text, "Carteira vazia.");
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
                .filter(|e| matches!(e, ProviderEvent::ToolFinished { .. }))
                .count(),
            1
        );
    }
    #[test]
    fn rejects_failed_results_unavailable_mcp_and_unapproved_tools() {
        for value in [
            json!({"type":"result","subtype":"error_max_turns","is_error":true,"errors":["Limite atingido"]}),
            json!({"type":"system","subtype":"init","mcp_servers":[{"name":"ivestfii","status":"failed"}]}),
            json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"x","name":"Bash","input":{"command":"unsafe"}}]}}),
        ] {
            let mut io = FakeIo {
                input: VecDeque::from([value]),
                ..Default::default()
            };
            assert!(run(&config(), &mut io).is_err());
        }
    }

    #[test]
    fn analysis_tool_preserves_refresh_arguments_in_audit() {
        let arguments = json!({"conversationId":"canonical","refreshPublicDataIfDue":true});
        let mut io = FakeIo {
            input: VecDeque::from([
                json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"analysis-1","name":"mcp__ivestfii__preparar_analise_carteira","input":arguments}]}}),
                json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"analysis-1","content":[{"type":"text","text":"{\"profileCompleteness\":{\"canPersonalize\":false}}"}]}]}}),
                json!({"type":"result","subtype":"success","result":"Informe seus objetivos."}),
            ]),
            ..Default::default()
        };
        run(&config(), &mut io).unwrap();
        assert!(io.events.iter().any(|event| matches!(event, ProviderEvent::ToolStarted {name,input,..} if name == "mcp__ivestfii__preparar_analise_carteira" && input == &arguments)));
    }
}
