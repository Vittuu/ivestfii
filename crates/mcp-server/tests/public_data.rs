//! Offline regressions: all public observations below are synthetic local
//! fixtures. No test sets a valid refresh flag on a valid wallet/conversation.
use ivestfii_assistant::AssistantStore;
use ivestfii_core::{CashPocketInput, FundInput, RecordInput, Repository, WalletCatalog};
use ivestfii_market_data::{
    EconomicIndicator, EconomicPanel, IndicatorError, NewsItem, NewsSnapshot,
};
use ivestfii_mcp::{McpServer, PROTOCOL_VERSION};
use serde_json::{json, Value};
use std::{fs, path::Path};

struct Fixture {
    directory: tempfile::TempDir,
    conversation: String,
    wallet: String,
}

fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let catalog = WalletCatalog::open(directory.path()).unwrap();
    let wallet = catalog.get(None).unwrap().id;
    let repository = catalog.repository(Some(&wallet)).unwrap();
    repository
        .upsert_fund(FundInput {
            id: None,
            ticker: "TEST11".into(),
            name: "Fundo sintético de teste".into(),
            sector: "Fixture".into(),
        })
        .unwrap();
    let fund_id = repository.snapshot().unwrap().funds[0].id.clone();
    repository
        .save_monthly_record(RecordInput {
            id: None,
            fund_id,
            month: "2020-01".into(),
            purchase_date: None,
            cotas_added: "5".into(),
            price_per_cota: "10.123456789".into(),
            dividend_per_cota: "0.5".into(),
            dividend_total: None,
            notes: "Nenhum dado real da carteira".into(),
        })
        .unwrap();
    repository
        .upsert_cash_pocket(CashPocketInput {
            id: None,
            name: "Reserva sintética".into(),
            institution: "Instituição de teste".into(),
            balance: "1000.12".into(),
            total_deposits: None,
            total_withdrawals: None,
            balance_date: None,
            cash_flows: None,
            clear_reconciliation: false,
            cdi_percent: "120".into(),
            annual_cdi_rate: Some("10".into()),
            rate_date: Some("2020-01-02".into()),
            rate_source: None,
            rate_source_url: None,
            rate_fetched_at: None,
            notes: "A consulta pública não deve atualizar esta referência manual.".into(),
        })
        .unwrap();
    let assistant = AssistantStore::open(directory.path().join("assistant.sqlite")).unwrap();
    let conversation = assistant
        .create_conversation("Conversa sintética para fontes públicas")
        .unwrap()
        .id;
    assistant
        .append_message(
            &conversation,
            "user",
            "Analise os dados datados.",
            Some("codex"),
            None,
        )
        .unwrap();
    Fixture {
        directory,
        conversation,
        wallet,
    }
}

fn ready(path: &Path) -> McpServer {
    let mut server = McpServer::open(path).unwrap();
    let result = server
        .handle_frame(
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
                "protocolVersion":PROTOCOL_VERSION,"capabilities":{},
                "clientInfo":{"name":"offline-public-data-tests","version":"1"}
            }})
            .to_string()
            .as_bytes(),
        )
        .unwrap();
    assert!(result.get("error").is_none(), "{result}");
    assert!(server
        .handle_frame(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
        .is_none());
    server
}

fn call(server: &mut McpServer, arguments: Value) -> Value {
    server
        .handle_frame(
            json!({"jsonrpc":"2.0","id":"analysis","method":"tools/call","params":{
                "name":"preparar_analise_carteira","arguments":arguments
            }})
            .to_string()
            .as_bytes(),
        )
        .unwrap()["result"]
        .clone()
}

fn content(result: &Value) -> &Value {
    assert_eq!(result["isError"], false, "{result}");
    &result["structuredContent"]
}

fn financial_state(path: &Path) -> Value {
    let repository = Repository::open_existing(path.join("portfolio.sqlite")).unwrap();
    json!({
        "export":repository.export_legacy_json().unwrap(),
        "snapshot":repository.snapshot().unwrap(),
        "pendingCount":repository.pending_change_count().unwrap(),
        "pendingChanges":repository.pending_changes().unwrap(),
    })
}

fn public_caches(path: &Path) -> (Vec<u8>, Vec<u8>) {
    let directory = path.join("public-data");
    fs::create_dir_all(&directory).unwrap();
    let economics = EconomicPanel {
        indicators: vec![
            EconomicIndicator {
                id: "cdi".into(),
                name: "CDI anualizado (base 252)".into(),
                value: "4.4".into(),
                unit: "percent_per_year".into(),
                reference_date: "2020-01-02".into(),
                source_id: "bcb-sgs-4389".into(),
                source: "Banco Central do Brasil · SGS 4389".into(),
                source_url: "https://api.bcb.gov.br/dados/serie/bcdata.sgs.4389/dados/ultimos/1?formato=json".into(),
                fetched_at: "2020-01-03T12:00:00Z".into(),
            },
            EconomicIndicator {
                id: "ipca".into(),
                name: "IPCA mensal".into(),
                value: "-0.1".into(),
                unit: "percent_per_month".into(),
                reference_date: "2019-12-01".into(),
                source_id: "bcb-sgs-433".into(),
                source: "Banco Central do Brasil · SGS 433".into(),
                source_url: "https://api.bcb.gov.br/dados/serie/bcdata.sgs.433/dados/ultimos/1?formato=json".into(),
                fetched_at: "2020-01-03T12:00:00Z".into(),
            },
        ],
        errors: vec![IndicatorError {
            id: "selic".into(),
            message: "Falha sintética anterior; nenhuma taxa inventada.".into(),
        }],
    };
    let url = "https://agenciabrasil.ebc.com.br/economia/noticia/2020-01/manchete-sintetica";
    let news = NewsSnapshot {
        items: vec![NewsItem {
            id: url.into(),
            title: "Manchete sintética para verificar proveniência".into(),
            url: url.into(),
            published_at: Some("2020-01-02T18:00:00Z".into()),
            source: "Agência Brasil".into(),
        }],
        source: "Agência Brasil".into(),
        source_url: "https://agenciabrasil.ebc.com.br/rss/economia/feed.xml".into(),
        fetched_at: "2020-01-03T12:00:00.000Z".into(),
    };
    let economic_bytes = serde_json::to_vec(&json!({"version":1,"panel":economics})).unwrap();
    let news_bytes = serde_json::to_vec(&news).unwrap();
    fs::write(directory.join("economics.json"), &economic_bytes).unwrap();
    fs::write(directory.join("news.json"), &news_bytes).unwrap();
    (economic_bytes, news_bytes)
}

#[test]
fn omitted_flags_read_only_dated_cache_and_keep_partial_failures_visible() {
    let fixture = fixture();
    let path = fixture.directory.path();
    let before = financial_state(path);
    let (economic_bytes, news_bytes) = public_caches(path);
    let mut server = ready(path);
    let result = call(&mut server, json!({}));
    let result = content(&result);
    assert_eq!(result["status"], "prepared-context");
    assert_eq!(result["economicData"]["status"], "cached");
    assert_eq!(result["economicData"]["refreshRequested"], false);
    assert_eq!(
        result["economicData"]["indicators"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(result["economicData"]["indicators"][0]["value"], "4.4");
    assert_eq!(
        result["economicData"]["indicators"][0]["referenceDate"],
        "2020-01-02"
    );
    assert_eq!(
        result["economicData"]["indicators"][0]["fetchedAt"],
        "2020-01-03T12:00:00Z"
    );
    assert_eq!(
        result["economicData"]["indicators"][0]["sourceId"],
        "bcb-sgs-4389"
    );
    assert_eq!(
        result["economicData"]["indicators"][1]["unit"],
        "percent_per_month"
    );
    assert_eq!(result["economicData"]["errors"][0]["id"], "selic");
    assert!(result["economicData"]["errors"][0]["message"]
        .as_str()
        .unwrap()
        .contains("sintética"));
    assert_eq!(result["news"]["status"], "cached");
    assert_eq!(result["news"]["refreshRequested"], false);
    assert_eq!(
        result["news"]["items"][0]["publishedAt"],
        "2020-01-02T18:00:00Z"
    );
    assert_eq!(result["news"]["fetchedAt"], "2020-01-03T12:00:00.000Z");
    assert_eq!(result["marketData"]["status"], "unavailable");
    assert_eq!(result["portfolio"]["cashPockets"][0]["annualCdiRate"], "10");
    assert_eq!(
        fs::read(path.join("public-data/economics.json")).unwrap(),
        economic_bytes
    );
    assert_eq!(
        fs::read(path.join("public-data/news.json")).unwrap(),
        news_bytes
    );
    assert_eq!(financial_state(path), before);
    assert_eq!(result["storedSourceCount"], 0);
    assert_eq!(result["publicDataRefresh"]["requested"], false);
    assert!(result["publicDataRefresh"]["checkedAt"].is_null());
    assert!(result["investorProfile"].is_null());
    assert_eq!(result["profileCompleteness"]["canPersonalize"], false);
    assert_eq!(
        result["profileCompleteness"]["missingFields"]
            .as_array()
            .unwrap()
            .len(),
        7
    );
    assert_eq!(result["performance"]["status"], "calculated");
    assert_eq!(
        result["performance"]["report"]["portfolio"]["cash"]["balance"],
        "1000.12"
    );
    assert!(result["performance"]["report"]["portfolio"]["cash"]["income"].is_null());
    assert!(!path
        .join("public-data/economics.refresh-policy.json")
        .exists());
}

#[test]
fn absent_or_corrupted_cache_is_explicitly_unavailable_and_never_repaired_by_a_read() {
    let fixture = fixture();
    let path = fixture.directory.path();
    let before = financial_state(path);
    let mut server = ready(path);
    let empty = call(&mut server, json!({}));
    let empty = content(&empty);
    for key in ["economicData", "news"] {
        assert_eq!(empty[key]["status"], "unavailable");
        assert_eq!(empty[key]["refreshRequested"], false);
    }
    assert_eq!(empty["economicData"]["indicators"], json!([]));
    assert_eq!(empty["news"]["items"], json!([]));
    assert!(!path.join("public-data").exists());

    fs::create_dir_all(path.join("public-data")).unwrap();
    for filename in ["economics.json", "news.json"] {
        fs::write(
            path.join("public-data").join(filename),
            b"malformed synthetic cache",
        )
        .unwrap();
    }
    let broken = call(&mut server, json!({"conversationId":fixture.conversation}));
    let broken = content(&broken);
    for key in ["economicData", "news"] {
        assert_eq!(broken[key]["status"], "unavailable");
        assert!(!broken[key]["error"].as_str().unwrap().is_empty());
    }
    assert_eq!(broken["storedSourceCount"], 0);
    for filename in ["economics.json", "news.json"] {
        assert_eq!(
            fs::read(path.join("public-data").join(filename)).unwrap(),
            b"malformed synthetic cache"
        );
    }
    assert_eq!(financial_state(path), before);
}

#[test]
fn invalid_flags_and_unknown_scope_are_rejected_before_fetch_or_cache_creation() {
    let fixture = fixture();
    let path = fixture.directory.path();
    let before = financial_state(path);
    let mut server = ready(path);
    for arguments in [
        json!({"refreshEconomicData":"true","refreshNews":true}),
        json!({"refreshEconomicData":true,"refreshNews":1}),
        json!({"refreshEconomicData":null}),
        json!({"refreshNews":null}),
        json!({"refreshPublicDataIfDue":null}),
        json!({"refreshPublicDataIfDue":"true"}),
        json!({"refreshDividends":"true"}),
        json!({"refreshDividends":null}),
        json!({"walletId":"unknown-wallet","refreshDividends":true}),
        json!({"walletId":"unknown-wallet","refreshPublicDataIfDue":true}),
        json!({"walletId":"unknown-wallet","refreshEconomicData":true,"refreshNews":true}),
        json!({"walletId":17,"refreshEconomicData":true}),
        json!({"conversationId":"unknown-conversation","refreshEconomicData":true,"refreshNews":true}),
        json!({"maxChars":100,"refreshEconomicData":true}),
    ] {
        let result = call(&mut server, arguments.clone());
        assert_eq!(
            result["isError"], true,
            "unexpected success: {arguments}: {result}"
        );
        assert!(
            !path.join("public-data").exists(),
            "invalid arguments accessed public data: {arguments}"
        );
    }
    let assistant = AssistantStore::open(path.join("assistant.sqlite")).unwrap();
    assert!(assistant.sources(&fixture.conversation).unwrap().is_empty());
    assert_eq!(financial_state(path), before);
}

#[test]
fn monthly_income_uses_dated_public_events_carries_shares_and_preserves_manual_zero() {
    use chrono::{FixedOffset, Months, Utc};
    use ivestfii_market_data::dividends::parse_b3_dividends;
    let fixture = fixture();
    let path = fixture.directory.path();
    let catalog = WalletCatalog::open(path).unwrap();
    let repository = catalog.repository(Some(&fixture.wallet)).unwrap();
    let fund_id = repository.snapshot().unwrap().funds[0].id.clone();
    let today = Utc::now()
        .with_timezone(&FixedOffset::west_opt(3 * 3600).unwrap())
        .date_naive();
    let month = today
        .checked_sub_months(Months::new(1))
        .unwrap()
        .format("%Y-%m")
        .to_string();
    // A later purchase increases the current holding, but not entitlement to
    // the event with data-com on the first day of that month.
    repository
        .save_monthly_record(RecordInput {
            id: None,
            fund_id: fund_id.clone(),
            month: month.clone(),
            purchase_date: Some(format!("{month}-02")),
            cotas_added: "2".into(),
            price_per_cota: "10".into(),
            dividend_per_cota: "0".into(),
            dividend_total: None,
            notes: "Compra sintética posterior à data-com".into(),
        })
        .unwrap();
    let fetched = Utc::now();
    let observation = parse_b3_dividends(
        &serde_json::to_vec(&json!({"code":"TEST","cashDividends":[{
            "isinCode":"BRTESTCTF000","label":"RENDIMENTO","rate":"0,123456789",
            "lastDatePrior":format!("{month}-01"),"paymentDate":format!("{month}-15"),
            "approvedOn":format!("{month}-01")
        }]}))
        .unwrap(),
        "TEST11",
        fetched,
    )
    .unwrap();
    let event_id = observation.events[0].event_id.clone();
    let source_url = observation.source_url.clone();
    let bytes = serde_json::to_vec(&json!({"version":1,"items":{"TEST11":{
        "observation":observation,"checkedAt":fetched.to_rfc3339(),
        "status":"available","error":null}},"retryAfter":null}))
    .unwrap();
    fs::create_dir_all(path.join("public-data")).unwrap();
    fs::write(path.join("public-data/dividends.json"), &bytes).unwrap();
    let before = financial_state(path);
    let mut server = ready(path);
    let first = call(&mut server, json!({"conversationId":fixture.conversation}));
    let first = content(&first);
    assert_eq!(first["dividendData"]["status"], "cached");
    assert_eq!(first["dividendData"]["refreshRequested"], false);
    assert_eq!(first["monthlyIncome"]["status"], "calculated");
    let point = first["monthlyIncome"]["report"]["months"]
        .as_array()
        .unwrap()
        .iter()
        .find(|point| point["month"] == month)
        .unwrap();
    assert_eq!(point["total"], "0.62");
    assert_eq!(point["apiAmount"], "0.62");
    assert_eq!(point["recordedAmount"], "0");
    assert_eq!(point["funds"][0]["holdings"], "7");
    assert_eq!(point["funds"][0]["eligibleShares"], "5");
    assert_eq!(point["funds"][0]["status"], "api");
    assert_eq!(point["funds"][0]["events"][0]["id"], event_id);
    assert_eq!(point["funds"][0]["events"][0]["sourceUrl"], source_url);
    assert_eq!(financial_state(path), before);
    assert_eq!(
        fs::read(path.join("public-data/dividends.json")).unwrap(),
        bytes
    );
    assert_eq!(first["storedSourceCount"], 1);
    let repeat = call(&mut server, json!({"conversationId":fixture.conversation}));
    assert_eq!(content(&repeat)["storedSourceCount"], 1);
    assert_eq!(content(&repeat)["monthlyIncome"], first["monthlyIncome"]);

    repository
        .save_monthly_record(RecordInput {
            id: None,
            fund_id,
            month: month.clone(),
            purchase_date: None,
            cotas_added: "0".into(),
            price_per_cota: "0".into(),
            dividend_per_cota: "0".into(),
            dividend_total: Some("0".into()),
            notes: "Zero confirmado sintético tem prioridade sobre a API".into(),
        })
        .unwrap();
    let before_override_read = financial_state(path);
    let overridden = call(&mut server, json!({}));
    let point = content(&overridden)["monthlyIncome"]["report"]["months"]
        .as_array()
        .unwrap()
        .iter()
        .find(|point| point["month"] == month)
        .unwrap();
    assert_eq!(point["funds"][0]["status"], "recorded");
    assert_eq!(point["total"], "0");
    assert_eq!(point["apiAmount"], "0");
    assert_eq!(financial_state(path), before_override_read);
    assert_eq!(
        fs::read(path.join("public-data/dividends.json")).unwrap(),
        bytes
    );
    assert!(!path
        .join("public-data/dividends.refresh-policy.json")
        .exists());
    fs::write(
        path.join("public-data/dividends.json"),
        b"invalid-synthetic-cache",
    )
    .unwrap();
    let degraded = call(&mut server, json!({}));
    let degraded = content(&degraded);
    assert_eq!(degraded["dividendData"]["status"], "unavailable");
    assert!(degraded["monthlyIncome"]["sourceError"].is_string());
    let point = degraded["monthlyIncome"]["report"]["months"]
        .as_array()
        .unwrap()
        .iter()
        .find(|point| point["month"] == month)
        .unwrap();
    assert_eq!(point["total"], "0");
    assert_eq!(point["funds"][0]["status"], "recorded");
    assert_eq!(financial_state(path), before_override_read);
    assert_eq!(
        fs::read(path.join("public-data/dividends.json")).unwrap(),
        b"invalid-synthetic-cache"
    );
}

#[test]
fn declared_profile_is_wallet_scoped_and_missing_fields_are_not_inferred_from_holdings_or_chat() {
    use ivestfii_core::{InvestmentProfileInput, WalletInput};
    let fixture = fixture();
    let path = fixture.directory.path();
    let catalog = WalletCatalog::open(path).unwrap();
    let repository = catalog.repository(Some(&fixture.wallet)).unwrap();
    let mut server = ready(path);
    let input: InvestmentProfileInput = serde_json::from_value(json!({
        "riskTolerance":"conservative", "shortTermObjective":"Reserva para despesas",
        "shortTermMonths":12, "longTermObjective":"Renda na aposentadoria", "longTermMonths":240,
        "liquidityNeeds":"Reserva em D+0", "emergencyReserveStatus":"partial",
        "constraints":"Sem ativos de baixa liquidez", "notes":"Dado pessoal sintético"
    }))
    .unwrap();
    repository.save_investment_profile(input).unwrap();
    let before = financial_state(path);
    let prepared = call(
        &mut server,
        json!({"walletId":fixture.wallet,"conversationId":fixture.conversation}),
    );
    let prepared = content(&prepared);
    assert_eq!(prepared["investorProfile"]["riskTolerance"], "conservative");
    assert_eq!(prepared["investorProfile"]["longTermMonths"], 240);
    assert!(prepared["investorProfile"]["updatedAt"].is_string());
    assert_eq!(prepared["profileCompleteness"]["canPersonalize"], true);
    assert_eq!(
        prepared["profileCompleteness"]["requiredQuestions"],
        json!([])
    );
    assert!(prepared["investorProfile"]["monthlyContribution"].is_null());
    assert_eq!(financial_state(path), before);
    let other = catalog
        .create(WalletInput {
            id: None,
            name: "Outro perfil".into(),
            institution: "".into(),
        })
        .unwrap();
    let missing = call(
        &mut server,
        json!({"walletId":other.id,"conversationId":fixture.conversation}),
    );
    let missing = content(&missing);
    assert!(missing["investorProfile"].is_null());
    assert_eq!(missing["profileCompleteness"]["canPersonalize"], false);
    assert_eq!(
        missing["profileCompleteness"]["requiredQuestions"]
            .as_array()
            .unwrap()
            .len(),
        7
    );
    let partial: InvestmentProfileInput =
        serde_json::from_value(json!({"riskTolerance":"moderate"})).unwrap();
    repository.save_investment_profile(partial).unwrap();
    let partial_before = financial_state(path);
    let partial = call(&mut server, json!({}));
    let partial = content(&partial);
    assert_eq!(partial["profileCompleteness"]["canPersonalize"], false);
    assert_eq!(
        partial["profileCompleteness"]["missingFields"]
            .as_array()
            .unwrap()
            .len(),
        6
    );
    assert!(!partial["profileCompleteness"]["missingFields"]
        .as_array()
        .unwrap()
        .contains(&json!("riskTolerance")));
    assert_eq!(financial_state(path), partial_before);
    assert!(!path.join("public-data").exists());
    let assistant = AssistantStore::open(path.join("assistant.sqlite")).unwrap();
    assert_eq!(assistant.messages(&fixture.conversation).unwrap().len(), 1);
    assert!(assistant.sources(&fixture.conversation).unwrap().is_empty());
}

#[test]
fn performance_and_daily_cdi_use_shared_calculations_and_original_dates_without_writes() {
    use chrono::{Days, FixedOffset, NaiveDate, Utc};
    use ivestfii_market_data::dividends::parse_b3_dividends;
    let fixture = fixture();
    let path = fixture.directory.path();
    public_caches(path);
    let fetched = "2020-02-12T12:00:00Z";
    let quoted = "2020-02-11T20:00:00Z";
    let quote_bytes = serde_json::to_vec(&json!({"version":1,"observations":{"TEST11":{
        "symbol":"TEST11","price":"9","currency":"BRL","quotedAt":quoted,"fetchedAt":fetched,
        "source":"brapi","sourceUrl":"https://brapi.dev/api/v2/stocks/quote?symbols=TEST11","delayMinutes":30
    }},"lastAttemptAt":fetched,"retryAfter":null})).unwrap();
    fs::write(path.join("public-data/quotes.json"), &quote_bytes).unwrap();
    let mut observations = Vec::new();
    let mut day = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();
    let end = NaiveDate::from_ymd_opt(2020, 2, 11).unwrap();
    while day <= end {
        observations.push(json!({"date":day.to_string(),"value":"0.04"}));
        day = day.checked_add_days(Days::new(1)).unwrap();
    }
    let history_url = "https://api.bcb.gov.br/dados/serie/bcdata.sgs.12/dados?formato=json&dataInicial=01/01/2020&dataFinal=11/02/2020";
    let history_bytes = serde_json::to_vec(&json!({"version":1,"snapshot":{
        "seriesId":12,"unit":"% a.d.","source":"Banco Central do Brasil · SGS 12",
        "sourceUrl":history_url,"requestedStartDate":"2020-01-01","requestedEndDate":"2020-02-11",
        "observations":observations,"fetchedAt":fetched,"stale":true
    }}))
    .unwrap();
    fs::write(path.join("public-data/cdi-history.json"), &history_bytes).unwrap();
    let dividend_observation = parse_b3_dividends(
        &serde_json::to_vec(&json!({"code":"TEST","cashDividends":[
            {"isinCode":"BRTESTCTF000","label":"RENDIMENTO","rate":"2,00",
                "lastDatePrior":"2020-01-01","paymentDate":"2020-01-15"},
            {"isinCode":"BRTESTCTF000","label":"RENDIMENTO","rate":"0,40",
                "lastDatePrior":"2020-02-01","paymentDate":"2020-02-10"},
            {"isinCode":"BRTESTCTF000","label":"RENDIMENTO","rate":"9,00",
                "lastDatePrior":"2020-02-01","paymentDate":"2020-02-12"}
        ]}))
        .unwrap(),
        "TEST11",
        fetched.parse().unwrap(),
    )
    .unwrap();
    let dividend_source_url = dividend_observation.source_url.clone();
    let dividend_bytes = serde_json::to_vec(&json!({"version":1,"items":{"TEST11":{
        "observation":dividend_observation,"checkedAt":fetched,
        "status":"available","error":null}},"retryAfter":null}))
    .unwrap();
    fs::write(path.join("public-data/dividends.json"), &dividend_bytes).unwrap();
    let before = financial_state(path);
    let mut server = ready(path);
    let result = call(&mut server, json!({"conversationId":fixture.conversation}));
    let result = content(&result);
    let snapshot = Repository::open_existing(path.join("portfolio.sqlite"))
        .unwrap()
        .snapshot()
        .unwrap();
    let today = Utc::now()
        .with_timezone(&FixedOffset::west_opt(10800).unwrap())
        .date_naive();
    let shared = ivestfii_market_data::performance::cached_report(&snapshot, path, today).unwrap();
    assert_eq!(
        result["performance"]["report"],
        serde_json::to_value(shared).unwrap()
    );
    assert_eq!(result["performance"]["report"]["asOf"], "2020-02-11");
    assert_eq!(result["performance"]["report"]["quoteFetchedAt"], fetched);
    assert_eq!(
        result["performance"]["report"]["totalResult"],
        "-1.117283945"
    );
    let performance = &result["performance"]["report"];
    assert_eq!(performance["dividends"], "4.5");
    assert_eq!(performance["income"]["recordedAmount"], "2.5");
    assert_eq!(performance["income"]["apiAmount"], "2");
    assert_eq!(performance["income"]["asOf"], "2020-02-11");
    assert_eq!(performance["dividendSource"], "b3");
    assert_eq!(performance["dividendFetchedAt"], fetched);
    assert_eq!(
        performance["dividendSources"][0]["sourceUrl"],
        dividend_source_url
    );
    // The API fills February only. January is already locally recorded, and
    // the February 12 event is beyond the quote's February 11 cutoff.
    assert_eq!(result["portfolio"]["totals"]["dividendsReceived"], "2.5");
    assert!(result["performance"]["report"]["cdi"].is_object());
    assert_eq!(result["cdiHistory"]["history"]["fetchedAt"], fetched);
    assert_eq!(
        result["cdiHistory"]["history"]["requestedEndDate"],
        "2020-02-11"
    );
    assert_eq!(result["cdiHistory"]["history"]["stale"], true);
    assert_eq!(result["portfolio"]["cashPockets"][0]["annualCdiRate"], "10");
    let source = result["storedSources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|source| source["url"] == history_url)
        .unwrap();
    let metadata: Value = serde_json::from_str(source["excerpt"].as_str().unwrap()).unwrap();
    assert_eq!(metadata["fetchedAt"], fetched);
    assert_eq!(metadata["seriesId"], 12);
    assert!(!source.to_string().contains("Dado pessoal"));
    assert_eq!(
        fs::read(path.join("public-data/quotes.json")).unwrap(),
        quote_bytes
    );
    assert_eq!(
        fs::read(path.join("public-data/cdi-history.json")).unwrap(),
        history_bytes
    );
    assert_eq!(
        fs::read(path.join("public-data/dividends.json")).unwrap(),
        dividend_bytes
    );
    assert!(!path
        .join("public-data/dividends.refresh-policy.json")
        .exists());
    assert_eq!(financial_state(path), before);
    let repeated = call(&mut server, json!({}));
    assert_eq!(content(&repeated)["performance"], result["performance"]);
    let repository = Repository::open_existing(path.join("portfolio.sqlite")).unwrap();
    repository
        .save_monthly_record(RecordInput {
            id: None,
            fund_id: snapshot.funds[0].id.clone(),
            month: "2020-02".into(),
            purchase_date: None,
            cotas_added: "0".into(),
            price_per_cota: "0".into(),
            dividend_per_cota: "0".into(),
            dividend_total: Some("0".into()),
            notes: "Zero sintético informado pelo usuário substitui a API".into(),
        })
        .unwrap();
    let after_manual_zero = financial_state(path);
    let overridden = call(&mut server, json!({}));
    let overridden = &content(&overridden)["performance"]["report"];
    assert_eq!(overridden["income"]["apiAmount"], "0");
    assert_eq!(overridden["dividends"], "2.5");
    assert_eq!(overridden["totalResult"], "-3.117283945");
    assert_eq!(overridden["cdi"], performance["cdi"]);
    assert_ne!(overridden["difference"], performance["difference"]);
    assert_eq!(financial_state(path), after_manual_zero);
    assert_eq!(
        fs::read(path.join("public-data/dividends.json")).unwrap(),
        dividend_bytes
    );
    let assistant = AssistantStore::open(path.join("assistant.sqlite")).unwrap();
    assert_eq!(assistant.messages(&fixture.conversation).unwrap().len(), 1);
}

#[test]
fn opt_in_reports_blocked_refresh_and_preserves_cached_dates_without_network() {
    let fixture = fixture();
    let path = fixture.directory.path();
    let (economic_bytes, news_bytes) = public_caches(path);
    // A corrupt persistent reservation blocks each source before HTTP. This
    // exercises the new flag offline without consuming any API budget.
    for source in [
        "economics",
        "news",
        "quotes",
        "cvm-funds",
        "cdi-history",
        "dividends",
    ] {
        fs::write(
            path.join(format!("public-data/{source}.refresh-policy.json")),
            b"blocked synthetic policy",
        )
        .unwrap();
    }
    let before = financial_state(path);
    let mut server = ready(path);
    let result = call(&mut server, json!({"refreshPublicDataIfDue":true}));
    let result = content(&result);
    assert_eq!(result["publicDataRefresh"]["requested"], true);
    assert!(result["publicDataRefresh"]["checkedAt"].is_string());
    for source in ["economics", "news"] {
        let outcome = result["publicDataRefresh"]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == source)
            .unwrap();
        assert_eq!(outcome["status"], "cached");
        assert!(outcome["message"].is_string());
    }
    let dividends = result["publicDataRefresh"]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|source| source["id"] == "dividends")
        .unwrap();
    assert_eq!(dividends["status"], "cached");
    assert!(dividends["message"].is_string());
    assert_eq!(result["dividendData"]["status"], "unavailable");
    assert_eq!(
        result["economicData"]["indicators"][0]["fetchedAt"],
        "2020-01-03T12:00:00Z"
    );
    assert_eq!(
        result["news"]["items"][0]["publishedAt"],
        "2020-01-02T18:00:00Z"
    );
    assert_eq!(
        fs::read(path.join("public-data/economics.json")).unwrap(),
        economic_bytes
    );
    assert_eq!(
        fs::read(path.join("public-data/news.json")).unwrap(),
        news_bytes
    );
    assert_eq!(financial_state(path), before);
    // A malformed local mapping cannot hide the other source outcomes. Their
    // corrupt reservation policies above still block HTTP before any request.
    fs::write(
        path.join("public-data/market-settings.json"),
        b"malformed local mapping",
    )
    .unwrap();
    let partial = call(&mut server, json!({"refreshPublicDataIfDue":true}));
    let partial = content(&partial);
    let outcomes = partial["publicDataRefresh"]["sources"].as_array().unwrap();
    assert!(outcomes.iter().any(|item| item["id"] == "economics"));
    assert!(outcomes
        .iter()
        .any(|item| item["id"] == "cvmMappings" && item["status"] == "unavailable"));
    assert_eq!(
        fs::read(path.join("public-data/market-settings.json")).unwrap(),
        b"malformed local mapping"
    );
    assert_eq!(financial_state(path), before);
}

#[test]
fn cached_sources_persist_across_servers_and_are_idempotent_without_financial_mutations() {
    let fixture = fixture();
    let path = fixture.directory.path();
    let before = financial_state(path);
    public_caches(path);
    let mut server = ready(path);
    let first = call(
        &mut server,
        json!({"conversationId":fixture.conversation,"walletId":fixture.wallet}),
    );
    let first = content(&first);
    assert_eq!(first["storedSourceCount"], 3);
    assert_eq!(first["storedSourcesTruncated"], false);
    let saved = first["storedSources"].clone();
    assert!(saved
        .as_array()
        .unwrap()
        .iter()
        .any(|source| source["title"]
            .as_str()
            .unwrap()
            .contains("referência 2020-01-02")));
    assert!(saved
        .as_array()
        .unwrap()
        .iter()
        .any(|source| source["publisher"] == "Agência Brasil"));
    for source in saved.as_array().unwrap() {
        let metadata: Value = serde_json::from_str(source["excerpt"].as_str().unwrap()).unwrap();
        if source["publisher"] == "Agência Brasil" {
            assert_eq!(metadata["fetchedAt"], "2020-01-03T12:00:00.000Z");
            assert_eq!(metadata["publishedAt"], "2020-01-02T18:00:00Z");
        } else {
            assert_eq!(metadata["fetchedAt"], "2020-01-03T12:00:00Z");
            assert!(metadata["referenceDate"].is_string());
            assert!(metadata["value"].is_string());
        }
    }
    drop(server);
    let assistant = AssistantStore::open(path.join("assistant.sqlite")).unwrap();
    assert_eq!(assistant.sources(&fixture.conversation).unwrap().len(), 3);
    assert_eq!(assistant.messages(&fixture.conversation).unwrap().len(), 1);
    let mut server = ready(path);
    let second = call(&mut server, json!({"conversationId":fixture.conversation}));
    let second = content(&second);
    assert_eq!(second["storedSourceCount"], 3);
    assert_eq!(second["storedSources"], saved);
    assert_eq!(assistant.sources(&fixture.conversation).unwrap().len(), 3);
    assert_eq!(financial_state(path), before);
}
