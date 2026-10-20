use ivestfii_assistant::AssistantStore;
use ivestfii_core::{
    CashPocketInput, CashSimulationInput, FundInput, RecordInput, Repository, WalletCatalog,
    WalletInput, DEFAULT_WALLET_ID,
};
use ivestfii_mcp::{McpServer, PROTOCOL_VERSION};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

struct Fixture {
    directory: tempfile::TempDir,
    secondary_id: String,
    main_pocket: String,
    secondary_pocket: String,
    main_fund: String,
    secondary_fund: String,
    conversation_id: String,
}

fn position(repository: &Repository, shares: &str) -> String {
    repository
        .upsert_fund(FundInput {
            id: None,
            ticker: "TEST11".into(),
            name: "Fundo sintético".into(),
            sector: "Teste".into(),
        })
        .unwrap();
    let id = repository.snapshot().unwrap().funds[0].id.clone();
    repository
        .save_monthly_record(RecordInput {
            id: None,
            fund_id: id.clone(),
            month: "2026-01".into(),
            cotas_added: shares.into(),
            price_per_cota: "10".into(),
            dividend_per_cota: "1".into(),
            dividend_total: None,
            notes: "Fixture".into(),
        })
        .unwrap();
    id
}

fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let catalog = WalletCatalog::open(directory.path()).unwrap();
    let main = catalog.repository(None).unwrap();
    let main_fund = position(&main, "3");
    let main_pocket = main
        .upsert_cash_pocket(CashPocketInput {
            id: None,
            name: "Reserva sintética".into(),
            institution: "Instituição de teste".into(),
            balance: "1000".into(),
            total_deposits: None,
            total_withdrawals: None,
            balance_date: None,
            clear_reconciliation: false,
            cdi_percent: "120".into(),
            annual_cdi_rate: Some("10".into()),
            rate_date: Some("2026-10-16".into()),
            rate_source: None,
            rate_source_url: None,
            rate_fetched_at: None,
            notes: "Taxa de cenário, não cotação atual".into(),
        })
        .unwrap()
        .id;
    let secondary = catalog
        .create(WalletInput {
            id: None,
            name: "FIIs no Nubank".into(),
            institution: "Nubank".into(),
        })
        .unwrap();
    let repository = catalog.repository(Some(&secondary.id)).unwrap();
    let secondary_fund = position(&repository, "8");
    let secondary_pocket = repository
        .upsert_cash_pocket(CashPocketInput {
            id: None,
            name: "Outra reserva sintética".into(),
            institution: "Nubank".into(),
            balance: "2000".into(),
            total_deposits: None,
            total_withdrawals: None,
            balance_date: None,
            clear_reconciliation: false,
            cdi_percent: "100".into(),
            annual_cdi_rate: Some("10".into()),
            rate_date: Some("2026-10-16".into()),
            rate_source: None,
            rate_source_url: None,
            rate_fetched_at: None,
            notes: "Cenário sintético".into(),
        })
        .unwrap()
        .id;
    let assistant = AssistantStore::open(directory.path().join("assistant.sqlite")).unwrap();
    let conversation_id = assistant
        .create_conversation("Conversa global entre carteiras")
        .unwrap()
        .id;
    assistant
        .append_message(
            &conversation_id,
            "user",
            "Mensagem anterior usando outra carteira.",
            Some("codex"),
            None,
        )
        .unwrap();
    Fixture {
        directory,
        secondary_id: secondary.id,
        main_pocket,
        secondary_pocket,
        main_fund,
        secondary_fund,
        conversation_id,
    }
}

fn ready(directory: &std::path::Path, wallet: Option<&str>) -> McpServer {
    let mut server = McpServer::open_for_wallet(directory, wallet).unwrap();
    let initialized = server
        .handle_frame(
            json!({"jsonrpc":"2.0","id":"initialize","method":"initialize","params":{
                "protocolVersion":PROTOCOL_VERSION,"capabilities":{},
                "clientInfo":{"name":"multiwallet-test","version":"1"}
            }})
            .to_string()
            .as_bytes(),
        )
        .unwrap();
    assert!(initialized.get("error").is_none());
    assert!(server
        .handle_frame(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
        .is_none());
    server
}

fn call(server: &mut McpServer, name: &str, arguments: Value) -> Value {
    server
        .handle_frame(
            json!({"jsonrpc":"2.0","id":"tool","method":"tools/call","params":{
                "name":name,"arguments":arguments
            }})
            .to_string()
            .as_bytes(),
        )
        .unwrap()["result"]
        .clone()
}

fn data(result: &Value) -> &Value {
    assert_eq!(result["isError"], false, "{result}");
    &result["structuredContent"]
}

#[test]
fn selected_wallet_and_explicit_override_keep_same_ticker_positions_separate() {
    let fixture = fixture();
    let mut server = ready(fixture.directory.path(), Some(&fixture.secondary_id));
    let selected = call(&mut server, "carteira_snapshot", json!({}));
    let selected = data(&selected);
    assert_eq!(selected["wallet"]["id"], fixture.secondary_id);
    assert_eq!(selected["wallet"]["institution"], "Nubank");
    assert_eq!(selected["funds"][0]["ticker"], "TEST11");
    assert_eq!(selected["funds"][0]["id"], fixture.secondary_fund);
    assert_eq!(selected["totals"]["cotas"], "8");
    assert_eq!(selected["totals"]["invested"], "80");
    assert_eq!(selected["cashBalance"], "2000");
    assert_eq!(selected["cashPockets"][0]["id"], fixture.secondary_pocket);
    assert_eq!(selected["wallets"].as_array().unwrap().len(), 2);
    assert_eq!(selected["wealthOverview"]["totals"]["fiiInvested"], "110");
    assert_eq!(selected["wealthOverview"]["totals"]["cashBalance"], "3000");
    assert_eq!(
        selected["dataScope"]["cashPockets"]["addCashPocketsToBrokerTotal"],
        false
    );
    assert_eq!(
        selected["dataScope"]["wealthOverview"]["addBrokerPositionsToOverview"],
        false
    );

    let main = call(
        &mut server,
        "carteira_snapshot",
        json!({"walletId":DEFAULT_WALLET_ID}),
    );
    let main = data(&main);
    assert_eq!(main["wallet"]["id"], DEFAULT_WALLET_ID);
    assert_eq!(main["funds"][0]["id"], fixture.main_fund);
    assert_eq!(main["totals"]["cotas"], "3");
    assert_eq!(main["cashBalance"], "1000");
    assert_eq!(main["cashPockets"][0]["cdiPercent"], "120");

    let selected_again = call(&mut server, "carteira_snapshot", json!({}));
    assert_eq!(data(&selected_again)["wallet"]["id"], fixture.secondary_id);
}

#[test]
fn composite_analysis_uses_selected_wallet_and_global_canonical_context() {
    let fixture = fixture();
    let mut server = ready(fixture.directory.path(), Some(&fixture.secondary_id));
    let prepared = call(
        &mut server,
        "preparar_analise_carteira",
        json!({"walletId":DEFAULT_WALLET_ID,"conversationId":fixture.conversation_id,"maxChars":10000}),
    );
    let prepared = data(&prepared);
    assert_eq!(prepared["portfolio"]["wallet"]["id"], DEFAULT_WALLET_ID);
    assert_eq!(prepared["portfolio"]["cashBalance"], "1000");
    assert_eq!(
        prepared["context"]["messages"][0]["content"],
        "Mensagem anterior usando outra carteira."
    );
    assert_eq!(prepared["marketData"]["status"], "unavailable");
    assert_eq!(prepared["news"]["status"], "unavailable");
    let global = call(
        &mut server,
        "recuperar_contexto",
        json!({"conversationId":fixture.conversation_id,"maxChars":10000}),
    );
    assert_eq!(
        data(&global)["messages"][0]["content"],
        "Mensagem anterior usando outra carteira."
    );
    assert_eq!(
        call(
            &mut server,
            "recuperar_contexto",
            json!({"conversationId":fixture.conversation_id,"maxChars":10000,"walletId":DEFAULT_WALLET_ID})
        )["isError"],
        true
    );
}

#[test]
fn cdi_simulation_is_scoped_gross_and_does_not_change_financial_rows() {
    let fixture = fixture();
    let catalog = WalletCatalog::open(fixture.directory.path()).unwrap();
    let main = catalog.repository(None).unwrap();
    let secondary = catalog.repository(Some(&fixture.secondary_id)).unwrap();
    let main_before = main.export_legacy_json().unwrap();
    let secondary_before = secondary.export_legacy_json().unwrap();
    let main_pending = main.pending_change_count().unwrap();
    let secondary_pending = secondary.pending_change_count().unwrap();
    let expected = main
        .simulate_cash_pocket(CashSimulationInput {
            pocket_id: fixture.main_pocket.clone(),
            business_days: 252,
        })
        .unwrap();
    let mut server = ready(fixture.directory.path(), Some(&fixture.secondary_id));
    let result = call(
        &mut server,
        "projetar_renda",
        json!({"walletId":DEFAULT_WALLET_ID,"cashPocketId":fixture.main_pocket,"businessDays":252}),
    );
    let result = data(&result);
    assert_eq!(result["kind"], "cash");
    assert_eq!(result["status"], "simulation");
    assert_eq!(result["wallet"]["id"], DEFAULT_WALLET_ID);
    assert_eq!(result["marketDataAvailable"], false);
    assert_eq!(result["rateSource"], "stored-annual-cdi-reference");
    assert_eq!(result["simulation"]["rateSource"], "manual");
    assert_eq!(
        result["simulation"],
        serde_json::to_value(expected).unwrap()
    );
    assert_eq!(result["simulation"]["annualCdiRate"], "10");
    assert_eq!(result["simulation"]["cdiPercent"], "120");
    assert!(!result["assumptions"].as_array().unwrap().is_empty());
    let pinned = call(
        &mut server,
        "projetar_renda",
        json!({"cashPocketId":fixture.secondary_pocket,"businessDays":252}),
    );
    assert_eq!(data(&pinned)["simulation"]["endingBalance"], "2200");
    assert_eq!(data(&pinned)["simulation"]["grossEarnings"], "200");
    assert_eq!(
        call(
            &mut server,
            "projetar_renda",
            json!({"cashPocketId":fixture.main_pocket,"businessDays":252})
        )["isError"],
        true
    );

    let fii = call(&mut server, "projetar_renda", json!({"months":1}));
    assert_eq!(data(&fii)["kind"], "fii");
    assert_eq!(data(&fii)["points"][0]["projectedIncome"], "8");
    let fii_main = call(
        &mut server,
        "projetar_renda",
        json!({"walletId":DEFAULT_WALLET_ID,"months":1}),
    );
    assert_eq!(data(&fii_main)["points"][0]["projectedIncome"], "3");
    assert_eq!(main.export_legacy_json().unwrap(), main_before);
    assert_eq!(secondary.export_legacy_json().unwrap(), secondary_before);
    assert_eq!(main.pending_change_count().unwrap(), main_pending);
    assert_eq!(secondary.pending_change_count().unwrap(), secondary_pending);
}

#[test]
fn ambiguous_inputs_unknown_wallets_and_cross_wallet_ids_are_errors() {
    let fixture = fixture();
    let mut server = ready(fixture.directory.path(), None);
    let unknown = "e2af797b-725d-4a26-835c-4aa7f42d7711";
    for name in [
        "carteira_snapshot",
        "preparar_analise_carteira",
        "projetar_renda",
    ] {
        for wallet in [json!(unknown), json!("../outside"), json!(""), json!(null)] {
            assert_eq!(
                call(&mut server, name, json!({"walletId":wallet}))["isError"],
                true,
                "{name} {wallet}"
            );
        }
    }
    assert!(!fixture
        .directory
        .path()
        .join("wallets")
        .join(unknown)
        .exists());
    assert!(McpServer::open_for_wallet(fixture.directory.path(), Some(unknown)).is_err());
    for field in ["fundId", "months", "monthlyCotas", "window", "reinvest"] {
        let mut arguments = json!({"cashPocketId":fixture.main_pocket,"businessDays":21});
        arguments[field] = Value::Null;
        assert_eq!(
            call(&mut server, "projetar_renda", arguments)["isError"],
            true
        );
    }
    for arguments in [
        json!({"businessDays":21}),
        json!({"cashPocketId":fixture.main_pocket}),
        json!({"cashPocketId":fixture.main_pocket,"businessDays":-1}),
        json!({"cashPocketId":fixture.main_pocket,"businessDays":2521}),
        json!({"cashPocketId":fixture.main_pocket,"businessDays":1.5}),
        json!({"cashPocketId":null,"businessDays":21}),
        json!({"fundId":fixture.secondary_fund,"months":1}),
    ] {
        assert_eq!(
            call(&mut server, "projetar_renda", arguments.clone())["isError"],
            true,
            "{arguments}"
        );
    }
}

#[test]
fn missing_registered_database_and_missing_primary_files_never_become_empty_wallets() {
    let fixture = fixture();
    let missing = fixture
        .directory
        .path()
        .join("wallets")
        .join(&fixture.secondary_id)
        .join("portfolio.sqlite");
    fs::remove_file(&missing).unwrap();
    assert!(
        McpServer::open_for_wallet(fixture.directory.path(), Some(&fixture.secondary_id)).is_err()
    );
    assert!(!missing.exists());

    let incomplete = tempfile::tempdir().unwrap();
    Repository::open(incomplete.path().join("portfolio.sqlite")).unwrap();
    assert!(McpServer::open(incomplete.path()).is_err());
    assert!(!incomplete.path().join("assistant.sqlite").exists());
    assert!(!incomplete.path().join("wallets.sqlite").exists());
}

#[test]
fn standalone_cli_pins_wallet_and_keeps_stdout_jsonrpc_only() {
    let fixture = fixture();
    let mut child = Command::new(env!("CARGO_BIN_EXE_ivestfii-mcp"))
        .args(["--data-dir"])
        .arg(fixture.directory.path())
        .args(["--wallet-id", &fixture.secondary_id])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    for request in [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":PROTOCOL_VERSION,"capabilities":{},"clientInfo":{"name":"cli-test","version":"1"}
        }}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"carteira_snapshot","arguments":{}}}),
    ] {
        writeln!(input, "{request}").unwrap();
    }
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let responses: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(responses.len(), 2);
    assert_eq!(
        responses[1]["result"]["structuredContent"]["wallet"]["id"],
        fixture.secondary_id
    );
    assert_eq!(
        responses[1]["result"]["structuredContent"]["totals"]["cotas"],
        "8"
    );
}
