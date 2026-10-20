//! End-to-end MCP regressions using only synthetic, dated cache files. No test
//! refreshes a valid scope: even the error-path refresh flags fail validation
//! before either HTTP connector can run. No real credential is read or written
//! by the fixtures, and no financial database is modified by analysis.
use ivestfii_assistant::AssistantStore;
use ivestfii_core::{FundInput, RecordInput, Repository, WalletCatalog, WalletInput};
use ivestfii_mcp::{McpServer, PROTOCOL_VERSION};
use serde_json::{json, Value};
use std::{fs, path::Path};

const FETCHED: &str = "2020-02-12T12:00:00Z";
const QUOTED: &str = "2020-02-11T20:00:00Z";
const CVM_URL: &str =
    "https://dados.cvm.gov.br/dados/FII/DOC/INF_MENSAL/DADOS/inf_mensal_fii_2020.zip";
const FIAGRO_URL: &str =
    "https://dados.cvm.gov.br/dados/FIAGRO/DOC/INF_MENSAL/DADOS/inf_mensal_fiagro_202609.zip";
const FIAGRO_FETCHED: &str = "2026-10-19T12:00:00Z";
const SECRET_SENTINEL: &str = "synthetic-ignored-secret-never-a-real-credential";

struct Fixture {
    directory: tempfile::TempDir,
    main: String,
    other: String,
    conversation: String,
    cnpjs: Vec<String>,
}

// Deterministic synthetic identification numbers with valid check digits. They
// are only local fixture identities; there is no claim they identify real FIIs.
fn cnpj(base: &str) -> String {
    let mut digits: Vec<u32> = base.bytes().map(|b| (b - b'0') as u32).collect();
    for weights in [
        &[5, 4, 3, 2, 9, 8, 7, 6, 5, 4, 3, 2][..],
        &[6, 5, 4, 3, 2, 9, 8, 7, 6, 5, 4, 3, 2][..],
    ] {
        let rest = digits.iter().zip(weights).map(|(d, w)| d * w).sum::<u32>() % 11;
        digits.push(if rest < 2 { 0 } else { 11 - rest });
    }
    digits
        .into_iter()
        .map(|d| char::from(b'0' + d as u8))
        .collect()
}

fn add_fund(repository: &Repository, ticker: &str) {
    repository
        .upsert_fund(FundInput {
            id: None,
            ticker: ticker.into(),
            name: format!("Fundo sintético {ticker}"),
            sector: "Fixture offline".into(),
        })
        .unwrap();
    let fund_id = repository
        .snapshot()
        .unwrap()
        .funds
        .into_iter()
        .find(|f| f.ticker == ticker)
        .unwrap()
        .id;
    repository
        .save_monthly_record(RecordInput {
            id: None,
            fund_id,
            month: "2020-01".into(),
            purchase_date: None,
            cotas_added: "3".into(),
            price_per_cota: "10".into(),
            dividend_per_cota: "0.1".into(),
            dividend_total: None,
            notes: "Dados exclusivamente sintéticos".into(),
        })
        .unwrap();
}

fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let catalog = WalletCatalog::open(directory.path()).unwrap();
    let main = catalog.get(None).unwrap().id;
    let repository = catalog.repository(Some(&main)).unwrap();
    add_fund(&repository, "TEST11");
    // TEST11 is deliberately present in both ledgers, while BRKR11 exists only
    // in the imported broker position. Quotes must cover the union once each.
    let broker_file = directory.path().join("synthetic-broker.json");
    fs::write(&broker_file, serde_json::to_vec(&json!({"broker_position":{
        "referenceDate":"2020-02-11", "institution":"Corretora sintética",
        "importedAt":FETCHED, "sourcePath":"synthetic-import.json",
        "sourceHash":"a".repeat(64),
        "overview":{"updatedTime":null,"capturedTime":null,"totalWithCash":"50",
            "invested":"50","cash":"0","listedFunds":"50","fixedIncome":"0","pendingIncome":"0"},
        "listedFunds":{"capturedTime":null,"marketValue":"50","costBasis":"50",
            "unrealizedResult":"0","dailyResult":"0","holdings":[
                {"ticker":"TEST11","shares":"2","averagePrice":"10","price":"10","marketValue":"20","costBasis":"20","unrealizedResult":"0"},
                {"ticker":"BRKR11","shares":"3","averagePrice":"10","price":"10","marketValue":"30","costBasis":"30","unrealizedResult":"0"}
            ]},
        "fixedIncome":{"name":"Reserva sintética vazia","nameTruncated":false,
            "balance":"0","netBalance":"0","reportedEarnings":"0","return12Months":"0",
            "redemptionConversion":"D+0","redemptionSettlement":"D+0"},
        "warnings":[]
    }})).unwrap()).unwrap();
    repository.import_broker_position_file(broker_file).unwrap();
    let other = catalog
        .create(WalletInput {
            id: None,
            name: "Outra carteira sintética".into(),
            institution: "Outra instituição".into(),
        })
        .unwrap()
        .id;
    add_fund(&catalog.repository(Some(&other)).unwrap(), "OTHR11");
    let assistant = AssistantStore::open(directory.path().join("assistant.sqlite")).unwrap();
    let conversation = assistant
        .create_conversation("Análise sintética com fontes datadas")
        .unwrap()
        .id;
    assistant
        .append_message(
            &conversation,
            "user",
            "Compare as fontes, preservando as datas.",
            Some("codex"),
            None,
        )
        .unwrap();
    Fixture {
        directory,
        main,
        other,
        conversation,
        cnpjs: ["123456780001", "234567890001", "345678900001"]
            .map(cnpj)
            .to_vec(),
    }
}

fn ready(path: &Path, wallet: Option<&str>) -> McpServer {
    let mut server = McpServer::open_for_wallet(path, wallet).unwrap();
    let response = server
        .handle_frame(
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
                "protocolVersion":PROTOCOL_VERSION,"capabilities":{},
                "clientInfo":{"name":"offline-asset-data-tests","version":"1"}
            }})
            .to_string()
            .as_bytes(),
        )
        .unwrap();
    assert!(response.get("error").is_none(), "{response}");
    let instructions = response["result"]["instructions"].as_str().unwrap();
    for capability in ["refreshQuotes", "refreshCvmData", "FII/FIAGRO", "fundType"] {
        assert!(
            instructions.contains(capability),
            "initialize omitted {capability}"
        );
    }
    assert!(!instructions.contains("Não há cotações"));
    assert!(server
        .handle_frame(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
        .is_none());
    server
}

fn call(server: &mut McpServer, arguments: Value) -> Value {
    server
        .handle_frame(
            json!({"jsonrpc":"2.0","id":"assets","method":"tools/call","params":{
                "name":"preparar_analise_carteira","arguments":arguments
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

fn item<'a>(panel: &'a Value, symbol: &str) -> &'a Value {
    panel["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["symbol"] == symbol)
        .unwrap()
}

fn state(fixture: &Fixture) -> Value {
    let catalog = WalletCatalog::open(fixture.directory.path()).unwrap();
    Value::Array([&fixture.main, &fixture.other].into_iter().map(|id| {
        let repository = catalog.repository(Some(id)).unwrap();
        json!({"export":repository.export_legacy_json().unwrap(),
            "snapshot":repository.snapshot().unwrap(),"pending":repository.pending_changes().unwrap(),
            "pendingCount":repository.pending_change_count().unwrap()})
    }).collect())
}

fn write_settings(fixture: &Fixture, main_mapping: Option<&str>) -> Vec<u8> {
    let path = fixture.directory.path().join("public-data");
    fs::create_dir_all(&path).unwrap();
    let bytes = serde_json::to_vec(&json!({"version":1,"cvmMappings":{
        "TEST11":main_mapping,"BRKR11":fixture.cnpjs[1],"OTHR11":fixture.cnpjs[2]},
        // Unknown configuration fields must never be forwarded to the model.
        "brapiToken":SECRET_SENTINEL,"apiKey":SECRET_SENTINEL}))
    .unwrap();
    fs::write(path.join("market-settings.json"), &bytes).unwrap();
    bytes
}

fn caches(fixture: &Fixture) -> (Vec<u8>, Vec<u8>) {
    let path = fixture.directory.path().join("public-data");
    fs::create_dir_all(&path).unwrap();
    let mut observations = serde_json::Map::new();
    let mut funds = serde_json::Map::new();
    let mut reports = serde_json::Map::new();
    for (i, symbol) in ["TEST11", "BRKR11", "OTHR11"].iter().enumerate() {
        observations.insert((*symbol).into(), json!({"symbol":symbol,
            "price":(["10.123456789","40","90"][i]),"currency":"BRL",
            "quotedAt":QUOTED,"fetchedAt":FETCHED,"source":"brapi",
            "sourceUrl":format!("https://brapi.dev/api/v2/stocks/quote?symbols={symbol}"),"delayMinutes":30}));
        funds.insert(
            fixture.cnpjs[i].clone(),
            json!(format!("FII sintético {symbol}")),
        );
        reports.insert(
            fixture.cnpjs[i].clone(),
            json!({"version":2,"report":{
            "cnpj":fixture.cnpjs[i],"fundName":format!("FII sintético {symbol}"),
            "referenceDate":"2020-01-01","filedAt":"2020-02-10",
            "netAssets":"123456.789","equityPerShare":"12.3456789","shareholders":7,
            "issuedShares":"10000","source":"CVM · Informe Mensal Estruturado de FII",
            "sourceUrl":CVM_URL,"fetchedAt":FETCHED}}),
        );
    }
    let quotes = serde_json::to_vec(&json!({"version":1,"observations":observations,
        "lastAttemptAt":FETCHED,"retryAfter":null}))
    .unwrap();
    let cvm = serde_json::to_vec(
        &json!({"version":2,"register":{"fetched_at":FETCHED,"funds":funds,"other_funds":{}},
        "years":{"2020":{"fetched_at":FETCHED,"reports":reports}}}),
    )
    .unwrap();
    fs::write(path.join("quotes.json"), &quotes).unwrap();
    fs::write(path.join("cvm-funds.json"), &cvm).unwrap();
    (quotes, cvm)
}

fn assert_no_secrets(value: &Value) {
    let text = value.to_string();
    assert!(
        !text.contains(SECRET_SENTINEL),
        "synthetic secret leaked into MCP"
    );
    match value {
        Value::Object(fields) => {
            for field in ["brapiToken", "apiKey", "cvmMappings", "Authorization"] {
                assert!(
                    !fields.contains_key(field),
                    "configuration/credential field leaked: {field}"
                );
            }
            for child in fields.values() {
                assert_no_secrets(child);
            }
        }
        Value::Array(items) => {
            for child in items {
                assert_no_secrets(child);
            }
        }
        _ => {}
    }
}

#[test]
fn four_tool_contract_exposes_opt_in_boolean_asset_refresh_flags() {
    let fixture = fixture();
    let mut server = ready(fixture.directory.path(), None);
    let result = server
        .handle_frame(br#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)
        .unwrap();
    let tools = result["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 4);
    let analysis = tools
        .iter()
        .find(|t| t["name"] == "preparar_analise_carteira")
        .unwrap();
    for flag in ["refreshQuotes", "refreshCvmData"] {
        assert_eq!(
            analysis["inputSchema"]["properties"][flag]["type"],
            "boolean"
        );
        assert_eq!(
            analysis["inputSchema"]["properties"][flag]["default"],
            false
        );
    }
    assert!(!fixture.directory.path().join("public-data").exists());
    assert_no_secrets(&result);
}

#[test]
fn cached_observations_keep_dates_exact_values_provenance_and_broker_wallet_scope() {
    let fixture = fixture();
    let before = state(&fixture);
    let settings = write_settings(&fixture, Some(&fixture.cnpjs[0]));
    let (quotes, cvm) = caches(&fixture);
    let mut server = ready(fixture.directory.path(), None);
    let result = call(&mut server, json!({}));
    assert_no_secrets(&result);
    let result_data = data(&result);
    let market = &result_data["marketData"];
    let reports = &result_data["cvmData"];
    assert_eq!(market["refreshRequested"], false);
    assert_eq!(reports["refreshRequested"], false);
    assert_eq!(market["status"], "cached");
    assert_eq!(reports["status"], "cached");
    assert_eq!(market["items"].as_array().unwrap().len(), 2);
    assert_eq!(reports["items"].as_array().unwrap().len(), 2);
    assert_eq!(market["fetchedAt"], FETCHED);
    assert_eq!(reports["fetchedAt"], FETCHED);
    let observation = &item(market, "TEST11")["observation"];
    assert_eq!(observation["price"], "10.123456789");
    assert_eq!(observation["quotedAt"], QUOTED);
    assert_eq!(observation["fetchedAt"], FETCHED);
    assert_eq!(observation["delayMinutes"], 30);
    assert_eq!(observation["source"], "brapi");
    assert_eq!(
        observation["sourceUrl"],
        "https://brapi.dev/api/v2/stocks/quote?symbols=TEST11"
    );
    assert_eq!(item(market, "BRKR11")["observation"]["price"], "40");
    let report = &item(reports, "BRKR11")["report"];
    assert_eq!(report["cnpj"], fixture.cnpjs[1]);
    // Version-2 FII cache files predate the explicit class field.
    assert_eq!(report["fundType"], "FII");
    assert_eq!(report["referenceDate"], "2020-01-01");
    assert_eq!(report["filedAt"], "2020-02-10");
    assert_eq!(report["fetchedAt"], FETCHED);
    assert_eq!(report["equityPerShare"], "12.3456789");
    assert_eq!(report["sourceUrl"], CVM_URL);
    assert_eq!(result_data["portfolio"]["totals"]["invested"], "30");
    assert_eq!(
        result_data["portfolio"]["brokerPosition"]["overview"]["totalWithCash"],
        "50"
    );

    let other = call(&mut server, json!({"walletId":fixture.other}));
    let other = data(&other);
    for key in ["marketData", "cvmData"] {
        assert_eq!(other[key]["items"].as_array().unwrap().len(), 1);
        assert_eq!(other[key]["items"][0]["symbol"], "OTHR11");
    }
    // The server-pinned wallet and an explicit override must agree.
    let mut pinned = ready(fixture.directory.path(), Some(&fixture.other));
    let pinned = call(&mut pinned, json!({}));
    assert_eq!(data(&pinned)["marketData"], other["marketData"]);
    assert_eq!(data(&pinned)["cvmData"], other["cvmData"]);
    assert_eq!(state(&fixture), before);
    for (file, bytes) in [
        ("quotes.json", quotes),
        ("cvm-funds.json", cvm),
        ("market-settings.json", settings),
    ] {
        assert_eq!(
            fs::read(fixture.directory.path().join("public-data").join(file)).unwrap(),
            bytes
        );
    }
}

#[test]
fn fiagro_monthly_report_keeps_class_source_dates_and_scope_without_financial_mutations() {
    let fixture = fixture();
    let before = state(&fixture);
    let settings = write_settings(&fixture, Some(&fixture.cnpjs[0]));
    let (quotes, previous_cvm) = caches(&fixture);
    let mut cache: Value = serde_json::from_slice(&previous_cvm).unwrap();
    let fiagro_cnpj = &fixture.cnpjs[1];
    cache["register"]["funds"]
        .as_object_mut()
        .unwrap()
        .remove(fiagro_cnpj);
    cache["register"]["fiagroFunds"] = json!({fiagro_cnpj.clone(): "FIAGRO sintético BRKR11"});
    let report = json!({
        "cnpj":fiagro_cnpj,"fundName":"FIAGRO sintético BRKR11","fundType":"FIAGRO",
        "referenceDate":"2026-09-01","filedAt":"2026-10-15",
        "netAssets":"456789.123456789","equityPerShare":"9.87654321",
        "shareholders":11,"issuedShares":"12345.6789",
        "source":"CVM · Informe Mensal de FIAGRO","sourceUrl":FIAGRO_URL,
        "fetchedAt":FIAGRO_FETCHED
    });
    cache["fiagroMonths"] = json!({"202609":{
        "fetched_at":FIAGRO_FETCHED,"reports":{
            fiagro_cnpj.clone(): {"version":1,"report":report}
        }
    }});
    // A historical FII report for the same CNPJ remains in the annual cache.
    // Current FIAGRO classification must select its own monthly source instead.
    let cvm = serde_json::to_vec(&cache).unwrap();
    fs::write(
        fixture.directory.path().join("public-data/cvm-funds.json"),
        &cvm,
    )
    .unwrap();
    let mut server = ready(fixture.directory.path(), None);
    let result = call(&mut server, json!({"conversationId":fixture.conversation}));
    assert_no_secrets(&result);
    let result = data(&result);
    assert_eq!(result["cvmData"]["refreshRequested"], false);
    assert_eq!(result["cvmData"]["status"], "cached");
    assert_eq!(result["cvmData"]["items"].as_array().unwrap().len(), 2);
    let selected = item(&result["cvmData"], "BRKR11");
    assert_eq!(selected["status"], "cached");
    assert!(selected["error"].is_null());
    assert_eq!(selected["cnpj"], *fiagro_cnpj);
    assert_eq!(selected["report"], report);
    assert_eq!(
        item(&result["cvmData"], "TEST11")["report"]["fundType"],
        "FII"
    );
    assert_eq!(result["storedSourceCount"], 4);
    let sources = result["storedSources"].clone();
    let source = sources
        .as_array()
        .unwrap()
        .iter()
        .find(|source| source["url"] == FIAGRO_URL)
        .unwrap();
    assert!(source["title"].as_str().unwrap().starts_with("BRKR11"));
    assert_eq!(
        serde_json::from_str::<Value>(source["excerpt"].as_str().unwrap()).unwrap(),
        report
    );

    let other = call(&mut server, json!({"walletId":fixture.other}));
    let other = data(&other);
    assert_eq!(other["cvmData"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(other["cvmData"]["items"][0]["symbol"], "OTHR11");
    assert_eq!(other["cvmData"]["items"][0]["report"]["fundType"], "FII");
    drop(server);
    let mut restarted = ready(fixture.directory.path(), None);
    let reread = call(
        &mut restarted,
        json!({"conversationId":fixture.conversation}),
    );
    assert_eq!(data(&reread)["cvmData"], result["cvmData"]);
    assert_eq!(data(&reread)["storedSources"], sources);
    assert_eq!(data(&reread)["storedSourceCount"], 4);
    assert_eq!(state(&fixture), before);
    for (filename, expected) in [
        ("quotes.json", quotes),
        ("cvm-funds.json", cvm),
        ("market-settings.json", settings),
    ] {
        assert_eq!(
            fs::read(fixture.directory.path().join("public-data").join(filename)).unwrap(),
            expected
        );
    }
}

#[test]
fn removed_or_changed_cnpj_mapping_cannot_reuse_a_previously_selected_report() {
    let fixture = fixture();
    write_settings(&fixture, Some(&fixture.cnpjs[0]));
    let (_, cached) = caches(&fixture);
    let mut server = ready(fixture.directory.path(), None);
    let first = call(&mut server, json!({"conversationId":fixture.conversation}));
    assert_eq!(
        item(&data(&first)["cvmData"], "TEST11")["report"]["cnpj"],
        fixture.cnpjs[0]
    );

    write_settings(&fixture, None);
    let disabled = call(&mut server, json!({}));
    let disabled = item(&data(&disabled)["cvmData"], "TEST11");
    assert_eq!(disabled["status"], "mapping_required");
    assert!(disabled["cnpj"].is_null());
    assert!(disabled["report"].is_null());

    // Existing cached reports are keyed by CNPJ, not by the former ticker link.
    write_settings(&fixture, Some(&fixture.cnpjs[2]));
    let changed = call(&mut server, json!({"conversationId":fixture.conversation}));
    let changed = data(&changed);
    assert_eq!(
        item(&changed["cvmData"], "TEST11")["report"]["cnpj"],
        fixture.cnpjs[2]
    );
    assert_eq!(
        item(&changed["cvmData"], "TEST11")["report"]["fundName"],
        "FII sintético OTHR11"
    );
    // Two funds in the same annual archive can share all three dates and the
    // source URL. A corrected CNPJ must still add its own attributable source.
    assert_eq!(changed["storedSourceCount"], 5);
    assert!(changed["storedSources"]
        .as_array()
        .unwrap()
        .iter()
        .any(|source| {
            source["title"].as_str().unwrap().starts_with("TEST11")
                && serde_json::from_str::<Value>(source["excerpt"].as_str().unwrap()).unwrap()
                    ["cnpj"]
                    == fixture.cnpjs[2]
        }));

    let file = fixture
        .directory
        .path()
        .join("public-data/market-settings.json");
    fs::write(file, br#"{"version":1,"cvmMappings":{}}"#).unwrap();
    let unconfigured = call(&mut server, json!({}));
    let unconfigured = item(&data(&unconfigured)["cvmData"], "TEST11");
    assert_eq!(unconfigured["status"], "mapping_required");
    assert!(unconfigured["report"].is_null());
    assert_eq!(
        fs::read(fixture.directory.path().join("public-data/cvm-funds.json")).unwrap(),
        cached
    );
}

#[test]
fn dated_quote_and_cvm_sources_persist_once_without_touching_financial_rows() {
    let fixture = fixture();
    let before = state(&fixture);
    write_settings(&fixture, Some(&fixture.cnpjs[0]));
    caches(&fixture);
    let mut server = ready(fixture.directory.path(), None);
    let first = call(&mut server, json!({"conversationId":fixture.conversation}));
    assert_no_secrets(&first);
    let first = data(&first);
    assert_eq!(first["storedSourceCount"], 4);
    let saved = first["storedSources"].clone();
    let mut quotes = 0;
    let mut reports = 0;
    for source in saved.as_array().unwrap() {
        let metadata: Value = serde_json::from_str(source["excerpt"].as_str().unwrap()).unwrap();
        assert_eq!(metadata["fetchedAt"], FETCHED);
        match source["publisher"].as_str().unwrap() {
            "brapi" => {
                quotes += 1;
                assert_eq!(metadata["quotedAt"], QUOTED);
                assert!(metadata["price"].is_string());
                assert!(source["url"]
                    .as_str()
                    .unwrap()
                    .starts_with("https://brapi.dev/"));
            }
            publisher if publisher.starts_with("CVM") => {
                reports += 1;
                assert_eq!(metadata["referenceDate"], "2020-01-01");
                assert_eq!(metadata["filedAt"], "2020-02-10");
                assert!(metadata["cnpj"].is_string());
                assert_eq!(source["url"], CVM_URL);
            }
            publisher => panic!("unexpected synthetic publisher: {publisher}"),
        }
    }
    assert_eq!((quotes, reports), (2, 2));
    drop(server);
    let mut restarted = ready(fixture.directory.path(), None);
    let second = call(
        &mut restarted,
        json!({"conversationId":fixture.conversation}),
    );
    assert_eq!(data(&second)["storedSources"], saved);
    assert_eq!(data(&second)["storedSourceCount"], 4);
    let assistant =
        AssistantStore::open(fixture.directory.path().join("assistant.sqlite")).unwrap();
    assert_eq!(assistant.sources(&fixture.conversation).unwrap().len(), 4);
    assert_eq!(assistant.messages(&fixture.conversation).unwrap().len(), 1);
    assert_eq!(state(&fixture), before);
}

#[test]
fn empty_or_corrupt_asset_caches_are_unavailable_without_fabricating_values_or_repairing_files() {
    let fixture = fixture();
    let before = state(&fixture);
    let mut server = ready(fixture.directory.path(), None);
    let absent = call(&mut server, json!({}));
    assert_no_secrets(&absent);
    let absent = data(&absent);
    assert_eq!(absent["marketData"]["status"], "unavailable");
    assert_eq!(absent["cvmData"]["status"], "unavailable");
    for entry in absent["marketData"]["items"].as_array().unwrap() {
        assert!(entry["observation"].is_null());
        // On an unconfigured machine this explicitly requests configuration;
        // an inherited environment credential only changes presence, not price.
        if absent["marketData"]["credentialConfigured"] == false {
            assert_eq!(entry["status"], "not_configured");
        }
    }
    for entry in absent["cvmData"]["items"].as_array().unwrap() {
        assert_eq!(entry["status"], "mapping_required");
        assert!(entry["report"].is_null());
    }
    assert!(!fixture.directory.path().join("public-data").exists());
    fs::create_dir_all(fixture.directory.path().join("public-data")).unwrap();
    for filename in ["quotes.json", "cvm-funds.json"] {
        fs::write(
            fixture.directory.path().join("public-data").join(filename),
            b"broken synthetic cache",
        )
        .unwrap();
    }
    let broken = call(&mut server, json!({"conversationId":fixture.conversation}));
    let broken = data(&broken);
    for key in ["marketData", "cvmData"] {
        assert_eq!(broken[key]["status"], "unavailable");
        assert!(!broken[key]["errors"].as_array().unwrap().is_empty());
    }
    assert_eq!(broken["storedSourceCount"], 0);
    for filename in ["quotes.json", "cvm-funds.json"] {
        assert_eq!(
            fs::read(fixture.directory.path().join("public-data").join(filename)).unwrap(),
            b"broken synthetic cache"
        );
    }
    assert_eq!(state(&fixture), before);
}

#[test]
fn malformed_refresh_flags_and_unknown_scope_fail_before_any_asset_network_or_cache_access() {
    let fixture = fixture();
    let before = state(&fixture);
    let mut server = ready(fixture.directory.path(), None);
    for args in [
        json!({"refreshQuotes":"true","refreshCvmData":true}),
        json!({"refreshQuotes":true,"refreshCvmData":1}),
        json!({"refreshQuotes":null}),
        json!({"refreshCvmData":null}),
        json!({"walletId":"unknown-wallet","refreshQuotes":true,"refreshCvmData":true}),
        json!({"walletId":17,"refreshQuotes":true}),
        json!({"conversationId":"unknown-conversation","refreshQuotes":true,"refreshCvmData":true}),
        json!({"maxChars":100,"refreshQuotes":true}),
        json!({"apiKey":SECRET_SENTINEL,"refreshQuotes":true}),
        json!({"symbols":["TEST11"],"refreshQuotes":true}),
        json!({"sourceUrl":"https://example.invalid/","refreshCvmData":true}),
    ] {
        let result = call(&mut server, args);
        assert_eq!(result["isError"], true, "{result}");
        assert!(!fixture.directory.path().join("public-data").exists());
        assert_no_secrets(&result);
    }
    let assistant =
        AssistantStore::open(fixture.directory.path().join("assistant.sqlite")).unwrap();
    assert!(assistant.sources(&fixture.conversation).unwrap().is_empty());
    assert_eq!(state(&fixture), before);
}
