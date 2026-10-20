use ivestfii_assistant::AssistantStore;
use ivestfii_core::{FundInput, RecordInput, Repository};
use ivestfii_mcp::{run, McpServer, MAX_FRAME_BYTES, PROTOCOL_VERSION};
use serde_json::{json, Value};
use std::io::{BufReader, Cursor};

fn fixture() -> (tempfile::TempDir, McpServer, String) {
    let temporary = tempfile::tempdir().unwrap();
    let repository = Repository::open(temporary.path().join("portfolio.sqlite")).unwrap();
    repository
        .upsert_fund(FundInput {
            id: None,
            ticker: "TEST11".into(),
            name: "Fundo sintético".into(),
            sector: "Teste".into(),
        })
        .unwrap();
    let fund_id = repository.snapshot().unwrap().funds[0].id.clone();
    repository
        .save_monthly_record(RecordInput {
            id: None,
            fund_id,
            month: "2025-01".into(),
            cotas_added: "10".into(),
            price_per_cota: "100".into(),
            dividend_per_cota: "1".into(),
            dividend_total: Some("10".into()),
            notes: "Fixture".into(),
        })
        .unwrap();
    let assistant = AssistantStore::open(temporary.path().join("assistant.sqlite")).unwrap();
    let conversation = assistant
        .create_conversation("Objetivo de longo prazo")
        .unwrap();
    assistant
        .append_message(
            &conversation.id,
            "user",
            "Quero renda em vinte anos.",
            Some("codex"),
            None,
        )
        .unwrap();
    assistant
        .append_message(
            &conversation.id,
            "assistant",
            "Vamos comparar premissas.",
            Some("claude"),
            None,
        )
        .unwrap();
    let server = McpServer::open(temporary.path()).unwrap();
    (temporary, server, conversation.id)
}

fn request(server: &mut McpServer, value: Value) -> Value {
    server
        .handle_frame(value.to_string().as_bytes())
        .expect("request must have response")
}

fn ready(server: &mut McpServer) {
    let response = request(
        server,
        json!({"jsonrpc":"2.0","id":"init","method":"initialize","params":{
            "protocolVersion":PROTOCOL_VERSION,"capabilities":{},"clientInfo":{"name":"test","version":"1"}
        }}),
    );
    assert_eq!(response["result"]["protocolVersion"], PROTOCOL_VERSION);
    let before_ready = request(
        server,
        json!({"jsonrpc":"2.0","id":"before-ready","method":"tools/list"}),
    );
    assert_eq!(before_ready["error"]["code"], -32002);
    assert!(server
        .handle_frame(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
        .is_none());
}

#[test]
fn handshake_is_required_and_preserves_string_or_zero_ids() {
    let (_temporary, mut server, _) = fixture();
    let early = request(
        &mut server,
        json!({"jsonrpc":"2.0","id":0,"method":"tools/list"}),
    );
    assert_eq!(early["id"], 0);
    assert_eq!(early["error"]["code"], -32002);
    ready(&mut server);
    let listed = request(
        &mut server,
        json!({"jsonrpc":"2.0","id":"tools-1","method":"tools/list"}),
    );
    assert_eq!(listed["id"], "tools-1");
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 4);
    for tool in tools {
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        assert_eq!(tool["inputSchema"]["additionalProperties"], false);
    }
    let analysis = tools
        .iter()
        .find(|tool| tool["name"] == "preparar_analise_carteira")
        .unwrap();
    for flag in [
        "refreshEconomicData",
        "refreshNews",
        "refreshQuotes",
        "refreshCvmData",
        "refreshPublicDataIfDue",
    ] {
        assert_eq!(
            analysis["inputSchema"]["properties"][flag]["default"],
            false
        );
    }
    let duplicate = request(
        &mut server,
        json!({"jsonrpc":"2.0","id":2,"method":"initialize","params":{}}),
    );
    assert_eq!(duplicate["error"]["code"], -32600);
}

#[test]
fn malformed_requests_and_tool_execution_errors_are_distinct() {
    let (_temporary, mut server, _) = fixture();
    assert_eq!(
        server.handle_frame(b"not json").unwrap()["error"]["code"],
        -32700
    );
    assert_eq!(request(&mut server, json!([]))["error"]["code"], -32600);
    assert!(request(
        &mut server,
        json!({"jsonrpc":"2.0","id":null,"method":"ping"})
    )["id"]
        .is_null());
    assert_eq!(
        request(
            &mut server,
            json!({"jsonrpc":"2.0","id":1.5,"method":"ping"})
        )["error"]["code"],
        -32600
    );
    ready(&mut server);
    let unknown = request(
        &mut server,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"read_any_file","arguments":{}}}),
    );
    assert_eq!(unknown["error"]["code"], -32602);
    let bad_shape = request(
        &mut server,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"projetar_renda","arguments":[]}}),
    );
    assert_eq!(bad_shape["error"]["code"], -32602);
    let invalid = request(
        &mut server,
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"projetar_renda","arguments":{"months":0}}}),
    );
    assert_eq!(invalid["result"]["isError"], true);
    let unknown_arg = request(
        &mut server,
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"carteira_snapshot","arguments":{"path":"secret"}}}),
    );
    assert_eq!(unknown_arg["result"]["isError"], true);
    assert!(server
        .handle_frame(br#"{"jsonrpc":"2.0","method":"unknown-notification"}"#)
        .is_none());
}

#[test]
fn composite_flow_reads_real_stores_and_reports_missing_external_data() {
    let (temporary, mut server, conversation_id) = fixture();
    let repository = Repository::open(temporary.path().join("portfolio.sqlite")).unwrap();
    let before = repository.export_legacy_json().unwrap();
    let pending = repository.pending_change_count().unwrap();
    ready(&mut server);
    let prepared = request(
        &mut server,
        json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{
            "name":"preparar_analise_carteira","arguments":{"conversationId":conversation_id,"maxChars":1000}
        }}),
    );
    assert_eq!(prepared["result"]["isError"], false);
    let data = &prepared["result"]["structuredContent"];
    assert_eq!(data["portfolio"]["funds"][0]["ticker"], "TEST11");
    assert_eq!(data["portfolio"]["totals"]["invested"], "1000");
    assert_eq!(data["portfolio"]["brokerPosition"], Value::Null);
    assert_eq!(
        data["portfolio"]["dataScope"]["brokerPosition"]["status"],
        "unavailable"
    );
    assert_eq!(
        data["portfolio"]["dataScope"]["brokerPosition"]["referenceDate"],
        Value::Null
    );
    assert_eq!(data["context"]["messages"][0]["provider"], "codex");
    assert_eq!(data["context"]["messages"][1]["provider"], "claude");
    assert_eq!(data["marketData"]["status"], "unavailable");
    assert_eq!(data["marketData"]["scope"], "external-delayed-quotes");
    assert_eq!(data["news"]["status"], "unavailable");
    assert_eq!(data["analysis"]["status"], "not-generated");
    let text_data: Value =
        serde_json::from_str(prepared["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(&text_data, data);
    let projection = request(
        &mut server,
        json!({"jsonrpc":"2.0","id":10,"method":"tools/call","params":{
            "name":"projetar_renda","arguments":{"months":2,"monthlyCotas":"1","reinvest":false}
        }}),
    );
    let projection = &projection["result"]["structuredContent"];
    assert_eq!(projection["points"].as_array().unwrap().len(), 2);
    assert_eq!(projection["assumptions"]["monthlyCotas"], "1");
    assert_eq!(projection["marketDataAvailable"], false);
    assert_eq!(repository.export_legacy_json().unwrap(), before);
    assert_eq!(repository.pending_change_count().unwrap(), pending);
}

#[test]
fn imported_observation_is_dated_and_never_combined_with_ledger_totals() {
    let (temporary, mut server, _) = fixture();
    let repository = Repository::open(temporary.path().join("portfolio.sqlite")).unwrap();
    // All figures are synthetic. The panorama and detailed positions have
    // different capture times and deliberately different market values.
    let position = json!({
        "referenceDate":"2020-02-03", "institution":"Corretora de teste",
        "importedAt":"2020-02-04T12:00:00Z", "sourcePath":"synthetic-position.json",
        "sourceHash":"a".repeat(64),
        "overview":{
            "updatedTime":"02:00", "capturedTime":"02:01",
            "totalWithCash":"151.50", "invested":"141.50", "cash":"10",
            "listedFunds":"89.50", "fixedIncome":"50", "pendingIncome":"2"
        },
        "listedFunds":{
            "capturedTime":"02:02", "marketValue":"90", "costBasis":"1000",
            "unrealizedResult":"-910", "dailyResult":"1",
            "holdings":[{"ticker":"TEST11", "shares":"10", "averagePrice":"100",
                "price":"9", "marketValue":"90", "costBasis":"1000", "unrealizedResult":"-910"}]
        },
        "fixedIncome":{
            "name":"Fundo sintético", "nameTruncated":false, "balance":"50",
            "netBalance":"49", "reportedEarnings":"4", "return12Months":"5",
            "redemptionConversion":"D+0", "redemptionSettlement":"D+0"
        },
        "warnings":[]
    });
    let source = temporary.path().join("synthetic-position.json");
    std::fs::write(&source, position.to_string()).unwrap();
    repository.import_broker_position_file(&source).unwrap();
    let before = repository.export_legacy_json().unwrap();
    let pending = repository.pending_change_count().unwrap();
    ready(&mut server);
    let direct = request(
        &mut server,
        json!({
            "jsonrpc":"2.0", "id":"dated-position", "method":"tools/call",
            "params":{"name":"carteira_snapshot", "arguments":{}}
        }),
    );
    assert_eq!(direct["result"]["isError"], false);
    let snapshot = &direct["result"]["structuredContent"];
    assert_eq!(snapshot["totals"]["invested"], "1000");
    assert_eq!(snapshot["totals"]["dividendsReceived"], "10");
    assert_eq!(snapshot["funds"][0]["entries"][0]["cotasAdded"], "10");
    assert_eq!(
        snapshot["brokerPosition"]["overview"]["totalWithCash"],
        "151.5"
    );
    assert_eq!(snapshot["brokerPosition"]["overview"]["pendingIncome"], "2");
    assert_eq!(
        snapshot["brokerPosition"]["overview"]["listedFunds"],
        "89.5"
    );
    assert_eq!(
        snapshot["brokerPosition"]["listedFunds"]["marketValue"],
        "90"
    );
    assert_eq!(
        snapshot["dataScope"]["brokerPosition"]["referenceDate"],
        "2020-02-03"
    );
    assert_eq!(
        snapshot["dataScope"]["brokerPosition"]["isLiveQuote"],
        false
    );
    assert_eq!(
        snapshot["dataScope"]["aggregationRules"],
        json!({
            "addHistoricalCostToBrokerTotal":false,
            "addHistoricalDividendsToBrokerTotal":false,
            "addPendingIncomeToReceivedDividends":false,
            "combineDifferentCapturedTimes":false
        })
    );
    let prepared = request(
        &mut server,
        json!({
            "jsonrpc":"2.0", "id":"prepared-position", "method":"tools/call",
            "params":{"name":"preparar_analise_carteira", "arguments":{}}
        }),
    );
    let data = &prepared["result"]["structuredContent"];
    assert_eq!(&data["portfolio"], snapshot);
    assert_eq!(data["marketData"]["status"], "unavailable");
    assert_eq!(data["marketData"]["scope"], "external-delayed-quotes");
    assert_eq!(repository.export_legacy_json().unwrap(), before);
    assert_eq!(repository.pending_change_count().unwrap(), pending);
}

#[test]
fn oversized_frame_is_drained_and_next_request_still_works() {
    let (_temporary, mut server, _) = fixture();
    let mut input = vec![b'x'; MAX_FRAME_BYTES + 1];
    input.extend_from_slice(b"\n{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"ping\"}\n");
    let mut output = Vec::new();
    // A small reader buffer exercises multi-chunk draining instead of a single
    // read that accidentally allocates an entire hostile input frame.
    run(
        &mut server,
        BufReader::with_capacity(17, Cursor::new(input)),
        &mut output,
    )
    .unwrap();
    let output = String::from_utf8(output).unwrap();
    let responses: Vec<Value> = output
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["error"]["code"], -32600);
    assert_eq!(responses[1]["id"], 7);
    assert_eq!(responses[1]["result"], json!({}));
}

#[test]
fn eof_exits_without_log_or_protocol_garbage_on_stdout() {
    let (_temporary, mut server, _) = fixture();
    let mut output = Vec::new();
    run(&mut server, Cursor::new(Vec::<u8>::new()), &mut output).unwrap();
    assert!(output.is_empty());
    let input = b"{\"jsonrpc\":\"2.0\",\"id\":\"ping\",\"method\":\"ping\"}\r\n";
    run(&mut server, Cursor::new(input), &mut output).unwrap();
    assert_eq!(output.iter().filter(|byte| **byte == b'\n').count(), 1);
    let parsed: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(parsed["id"], "ping");
}

#[test]
fn incorrect_data_directory_does_not_create_an_empty_portfolio() {
    let temporary = tempfile::tempdir().unwrap();
    assert!(McpServer::open(temporary.path()).is_err());
    assert!(!temporary.path().join("portfolio.sqlite").exists());
    assert!(!temporary.path().join("assistant.sqlite").exists());
}

#[test]
fn unsupported_version_receives_explicit_supported_version() {
    let (_temporary, mut server, _) = fixture();
    let response = request(
        &mut server,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"unknown-version","capabilities":{},"clientInfo":{"name":"test","version":"1"}
        }}),
    );
    assert_eq!(response["result"]["protocolVersion"], PROTOCOL_VERSION);
    assert!(response["result"]["capabilities"].get("tasks").is_none());
}

#[test]
fn oversized_output_returns_bounded_error_with_the_original_id() {
    let (temporary, mut server, conversation_id) = fixture();
    let assistant = AssistantStore::open(temporary.path().join("assistant.sqlite")).unwrap();
    assistant
        .add_source(
            &conversation_id,
            None,
            "Fonte extensa de teste",
            "https://example.com",
            None,
            Some(&"x".repeat(MAX_FRAME_BYTES)),
        )
        .unwrap();
    ready(&mut server);
    let input = json!({"jsonrpc":"2.0","id":"large-response","method":"tools/call","params":{
        "name":"preparar_analise_carteira","arguments":{"conversationId":conversation_id}
    }})
    .to_string()
        + "\n";
    let mut output = Vec::new();
    run(&mut server, Cursor::new(input), &mut output).unwrap();
    assert!(output.len() < 1024);
    let response: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(response["id"], "large-response");
    assert_eq!(response["error"]["code"], -32603);
}
