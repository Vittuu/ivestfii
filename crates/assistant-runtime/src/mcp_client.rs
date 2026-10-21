//! Host-managed MCP connection for CLIs without a usable MCP loading surface.
//! This client speaks the same stdio protocol as the desktop's --mcp mode;
//! it never bypasses the MCP server by querying application stores directly.

use serde_json::{json, Value};
use std::{collections::HashSet, time::Duration};

const PROTOCOL: &str = "2025-11-25";
const TOOLS: [&str; 5] = [
    "carteira_snapshot",
    "projetar_renda",
    "recuperar_contexto",
    "preparar_analise_carteira",
    "simular_aporte",
];

pub(super) trait Transport {
    fn send(&mut self, value: &Value) -> Result<(), String>;
    fn receive(&mut self) -> Result<Value, String>;
    fn set_timeout(&mut self, timeout: Duration);
}

pub(super) struct Client<T: Transport> {
    transport: T,
    next_id: u64,
    usable: bool,
    definitions: Value,
}

impl<T: Transport> Client<T> {
    pub fn connect(transport: T) -> Result<Self, String> {
        let mut client = Self {
            transport,
            next_id: 1,
            usable: false,
            definitions: Value::Null,
        };
        let initialized = client.request(
            "initialize",
            json!({
                "protocolVersion":PROTOCOL,
                "capabilities":{},
                "clientInfo":{"name":"ivestfii-managed-client","version":env!("CARGO_PKG_VERSION")}
            }),
            Duration::from_secs(30),
        )?;
        if initialized.get("protocolVersion").and_then(Value::as_str) != Some(PROTOCOL) {
            return Err("O MCP local negociou uma versão de protocolo incompatível.".into());
        }
        if !initialized
            .pointer("/capabilities/tools")
            .is_some_and(Value::is_object)
            || initialized
                .pointer("/serverInfo/name")
                .and_then(Value::as_str)
                != Some("ivestfii-local")
        {
            return Err(
                "O processo iniciado não confirmou o servidor MCP de leitura IvestFII.".into(),
            );
        }
        client
            .transport
            .send(&json!({"jsonrpc":"2.0","method":"notifications/initialized","params":{}}))?;
        let inventory = client.request("tools/list", json!({}), Duration::from_secs(30))?;
        client.verify_inventory(&inventory)?;
        client.definitions = inventory["tools"].clone();
        client.usable = true;
        Ok(client)
    }

    pub fn definitions(&self) -> Result<Value, String> {
        if !self.usable {
            return Err("A conexão MCP foi interrompida.".into());
        }
        Ok(self.definitions.clone())
    }

    pub fn is_usable(&self) -> bool {
        self.usable
    }

    fn verify_inventory(&self, result: &Value) -> Result<(), String> {
        let tools = result
            .get("tools")
            .and_then(Value::as_array)
            .ok_or("O MCP local retornou um inventário inválido.")?;
        let names: HashSet<&str> = tools
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .collect();
        if result
            .get("nextCursor")
            .is_some_and(|cursor| !cursor.is_null())
            || tools.len() != TOOLS.len()
            || names.len() != TOOLS.len()
            || TOOLS.iter().any(|name| !names.contains(name))
            || tools.iter().any(|tool| {
                tool.pointer("/annotations/readOnlyHint")
                    .and_then(Value::as_bool)
                    != Some(true)
            })
        {
            return Err(
                "O MCP local precisa expor exatamente as cinco ferramentas de leitura autorizadas."
                    .into(),
            );
        }
        Ok(())
    }

    fn request(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        self.transport.set_timeout(timeout);
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or("Limite de chamadas MCP excedido.")?;
        self.transport
            .send(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        loop {
            let frame = self.transport.receive()?;
            if frame.get("jsonrpc").and_then(Value::as_str) != Some("2.0") || !frame.is_object() {
                return Err("O MCP local retornou uma mensagem JSON-RPC inválida.".into());
            }
            if let Some(server_method) = frame.get("method").and_then(Value::as_str) {
                if let Some(server_id) = frame.get("id") {
                    // Never leave an unimplemented request pending invisibly.
                    // Ping is the only server request supported by this client.
                    let response = if server_method == "ping" {
                        json!({"jsonrpc":"2.0","id":server_id,"result":{}})
                    } else {
                        json!({"jsonrpc":"2.0","id":server_id,"error":{"code":-32601,
                            "message":"O cliente IvestFII não oferece interação nem execução de comandos."}})
                    };
                    self.transport.send(&response)?;
                }
                // Progress/log notifications have no response. The operation
                // deadline is unchanged so notifications cannot keep it alive.
                continue;
            }
            if frame.get("id").and_then(Value::as_u64) != Some(id) {
                return Err("O MCP local retornou um identificador de resposta inesperado.".into());
            }
            if let Some(error) = frame.get("error") {
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("erro JSON-RPC sem detalhes");
                return Err(format!("O MCP local recusou {method}: {message}"));
            }
            return frame
                .get("result")
                .filter(|result| result.is_object())
                .cloned()
                .ok_or_else(|| "O MCP local retornou um resultado inválido.".into());
        }
    }

    pub fn call_tool(&mut self, name: &str, arguments: &Value) -> Result<Value, String> {
        if !TOOLS.contains(&name) {
            return Err("Ferramenta fora da lista de leitura autorizada do MCP IvestFII.".into());
        }
        if !arguments.is_object() {
            return Err("Os argumentos da ferramenta MCP devem ser um objeto JSON.".into());
        }
        if !self.usable {
            return Err(
                "A conexão MCP foi interrompida. Inicie um novo turno para reconectar.".into(),
            );
        }
        let timeout =
            if name == "preparar_analise_carteira" && arguments["refreshPublicDataIfDue"] == true {
                Duration::from_secs(120)
            } else if name == "preparar_analise_carteira"
                && ["refreshQuotes", "refreshCvmData"]
                    .iter()
                    .any(|field| arguments[*field] == true)
            {
                Duration::from_secs(90)
            } else {
                Duration::from_secs(60)
            };
        let result = match self.request(
            "tools/call",
            json!({"name":name,"arguments":arguments}),
            timeout,
        ) {
            Ok(result) => result,
            Err(error) => {
                self.usable = false;
                return Err(error);
            }
        };
        if !result.get("content").is_some_and(Value::is_array)
            || result
                .get("isError")
                .is_some_and(|value| !value.is_boolean())
        {
            self.usable = false;
            return Err("A ferramenta MCP retornou conteúdo fora do protocolo.".into());
        }
        // MCP domain errors are valid tool results; return isError unchanged so
        // the adapter can record the failure and ask for corrected arguments.
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::VecDeque,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
    };

    #[derive(Default)]
    struct FakeTransport {
        sent: Vec<Value>,
        incoming: VecDeque<Value>,
        cancelled: Arc<AtomicBool>,
        timeouts: Vec<Duration>,
    }
    impl Transport for FakeTransport {
        fn send(&mut self, value: &Value) -> Result<(), String> {
            if self.cancelled.load(Ordering::SeqCst) {
                return Err("Resposta cancelada.".into());
            }
            self.sent.push(value.clone());
            Ok(())
        }
        fn receive(&mut self) -> Result<Value, String> {
            if self.cancelled.load(Ordering::SeqCst) {
                return Err("Resposta cancelada.".into());
            }
            self.incoming
                .pop_front()
                .ok_or_else(|| "MCP EOF de teste".into())
        }
        fn set_timeout(&mut self, timeout: Duration) {
            self.timeouts.push(timeout);
        }
    }
    fn response(id: u64, result: Value) -> Value {
        json!({"jsonrpc":"2.0","id":id,"result":result})
    }
    fn inventory() -> Value {
        json!({"tools":TOOLS.map(|name|json!({"name":name,"annotations":{"readOnlyHint":true}}))})
    }
    fn ready(tail: Vec<Value>) -> FakeTransport {
        let mut incoming = vec![
            response(
                1,
                json!({"protocolVersion":PROTOCOL,"serverInfo":{"name":"ivestfii-local"},"capabilities":{"tools":{}}}),
            ),
            response(2, inventory()),
        ];
        incoming.extend(tail);
        FakeTransport {
            incoming: incoming.into(),
            ..FakeTransport::default()
        }
    }
    #[test]
    fn negotiates_protocol_and_runs_real_mcp_frames_on_one_connection() {
        let mut client=Client::connect(ready(vec![
            response(3,json!({"content":[{"type":"text","text":"snapshot"}],"structuredContent":{"funds":[]},"isError":false})),
            response(4,json!({"content":[],"structuredContent":{"points":[]}})),
        ])).unwrap();
        let first = client.call_tool("carteira_snapshot", &json!({})).unwrap();
        assert_eq!(first["structuredContent"]["funds"], json!([]));
        client
            .call_tool("projetar_renda", &json!({"months":12}))
            .unwrap();
        assert_eq!(client.transport.sent[0]["method"], "initialize");
        assert_eq!(
            client.transport.sent[0]["params"]["protocolVersion"],
            PROTOCOL
        );
        assert_eq!(
            client.transport.sent[1]["method"],
            "notifications/initialized"
        );
        assert!(client.transport.sent[1].get("id").is_none());
        assert_eq!(client.transport.sent[2]["method"], "tools/list");
        assert_eq!(
            client.transport.sent[3]["params"]["name"],
            "carteira_snapshot"
        );
        assert_eq!(client.transport.sent[4]["id"], 4);
        assert_eq!(client.definitions().unwrap().as_array().unwrap().len(), 5);
        assert_eq!(
            client.transport.timeouts,
            vec![
                Duration::from_secs(30),
                Duration::from_secs(30),
                Duration::from_secs(60),
                Duration::from_secs(60)
            ]
        );
    }
    #[test]
    fn automatic_refresh_argument_is_forwarded_with_a_bounded_transport_budget() {
        let mut client = Client::connect(ready(vec![response(
            3,
            json!({"content":[],"structuredContent":{"publicDataRefresh":{"requested":true}}}),
        )]))
        .unwrap();
        let arguments =
            json!({"conversationId":"canonical-conversation","refreshPublicDataIfDue":true});
        client
            .call_tool("preparar_analise_carteira", &arguments)
            .unwrap();
        assert_eq!(client.transport.sent[3]["params"]["arguments"], arguments);
        assert_eq!(
            *client.transport.timeouts.last().unwrap(),
            Duration::from_secs(120)
        );
    }
    #[test]
    fn contribution_simulation_forwards_money_strings_without_refreshing() {
        let output = json!({"content":[],"structuredContent":{"status":"simulation","kind":"contribution-allocation","simulation":{"budget":"300.00"}},"isError":false});
        let mut client = Client::connect(ready(vec![response(3, output.clone())])).unwrap();
        let arguments = json!({"walletId":"wallet-2","amount":"300.00","allocations":{"fiis":"100.00","fixedIncome":"50.00","cashPockets":"150.00"}});
        assert_eq!(
            client.call_tool("simular_aporte", &arguments).unwrap(),
            output
        );
        assert_eq!(
            client.transport.sent[3]["params"],
            json!({"name":"simular_aporte","arguments":arguments})
        );
        assert_eq!(
            *client.transport.timeouts.last().unwrap(),
            Duration::from_secs(60)
        );
        let mut old_transport = ready(vec![]);
        let mut old_inventory = inventory();
        old_inventory["tools"].as_array_mut().unwrap().pop();
        old_transport.incoming[1] = response(2, old_inventory);
        assert!(Client::connect(old_transport).is_err());
        let before = client.transport.sent.len();
        assert!(client.call_tool("executar_aporte", &arguments).is_err());
        assert_eq!(client.transport.sent.len(), before);
    }

    #[test]
    fn rejects_foreign_or_mutating_inventory() {
        for inventory in [
            json!({"tools":[]}),
            json!({"tools":[{"name":"shell","annotations":{"readOnlyHint":false}}]}),
        ] {
            let mut transport = ready(vec![]);
            transport.incoming[1] = response(2, inventory);
            assert!(Client::connect(transport).is_err());
        }
        let mut transport = ready(vec![]);
        let mut value = inventory();
        value["tools"][0]["annotations"]["readOnlyHint"] = Value::Bool(false);
        transport.incoming[1] = response(2, value);
        assert!(Client::connect(transport).is_err());
    }
    #[test]
    fn rejects_bad_version_before_initialization_notification() {
        let mut transport = ready(vec![]);
        transport.incoming[0]["result"]["protocolVersion"] = json!("1900-01-01");
        assert!(Client::connect(transport).is_err());
    }
    #[test]
    fn refuses_unauthorized_names_and_arguments_without_sending() {
        let mut client = Client::connect(ready(vec![])).unwrap();
        let before = client.transport.sent.len();
        assert!(client.call_tool("execute_shell", &json!({})).is_err());
        assert!(client.call_tool("carteira_snapshot", &Value::Null).is_err());
        assert_eq!(client.transport.sent.len(), before);
    }
    #[test]
    fn domain_error_preserves_connection_and_error_flag() {
        let mut client = Client::connect(ready(vec![
            response(
                3,
                json!({"content":[{"type":"text","text":"premissas inválidas"}],"isError":true}),
            ),
            response(4, json!({"content":[],"isError":false})),
        ]))
        .unwrap();
        assert_eq!(
            client.call_tool("projetar_renda", &json!({})).unwrap()["isError"],
            true
        );
        assert!(client.call_tool("carteira_snapshot", &json!({})).is_ok());
    }
    #[test]
    fn rpc_or_transport_error_invalidates_connection() {
        let mut client=Client::connect(ready(vec![json!({"jsonrpc":"2.0","id":3,"error":{"code":-32602,"message":"argumentos inválidos"}})])).unwrap();
        assert!(client
            .call_tool("projetar_renda", &json!({}))
            .unwrap_err()
            .contains("argumentos inválidos"));
        assert!(client
            .call_tool("carteira_snapshot", &json!({}))
            .unwrap_err()
            .contains("interrompida"));
        let mut client = Client::connect(ready(vec![])).unwrap();
        assert!(client
            .call_tool("carteira_snapshot", &json!({}))
            .unwrap_err()
            .contains("EOF"));
    }
    #[test]
    fn cancellation_interrupts_tool_calls() {
        let transport = ready(vec![]);
        let cancelled = transport.cancelled.clone();
        let mut client = Client::connect(transport).unwrap();
        cancelled.store(true, Ordering::SeqCst);
        assert!(client
            .call_tool("carteira_snapshot", &json!({}))
            .unwrap_err()
            .contains("cancelada"));
    }
    #[test]
    fn malformed_tool_result_is_an_error() {
        for malformed in [
            json!({"structuredContent":{}}),
            json!({"content":[],"isError":"false"}),
        ] {
            let mut client = Client::connect(ready(vec![response(3, malformed)])).unwrap();
            assert!(client
                .call_tool("carteira_snapshot", &json!({}))
                .unwrap_err()
                .contains("protocolo"));
        }
    }
    #[test]
    fn ping_and_unsupported_server_requests_are_answered() {
        let mut transport = ready(vec![response(3, json!({"content":[]}))]);
        transport.incoming.insert(
            2,
            json!({"jsonrpc":"2.0","id":"server-ping","method":"ping"}),
        );
        transport.incoming.insert(
            3,
            json!({"jsonrpc":"2.0","id":"server-input","method":"elicitation/create"}),
        );
        let mut client = Client::connect(transport).unwrap();
        client.call_tool("carteira_snapshot", &json!({})).unwrap();
        assert!(client
            .transport
            .sent
            .iter()
            .any(|frame| frame["id"] == "server-ping" && frame["result"].is_object()));
        assert!(client
            .transport
            .sent
            .iter()
            .any(|frame| frame["id"] == "server-input" && frame["error"]["code"] == -32601));
    }
    #[test]
    fn wrong_response_id_or_missing_jsonrpc_is_rejected() {
        for frame in [
            response(77, json!({"content":[]})),
            json!({"id":3,"result":{"content":[]}}),
        ] {
            let mut client = Client::connect(ready(vec![frame])).unwrap();
            assert!(client.call_tool("carteira_snapshot", &json!({})).is_err());
        }
    }
}
