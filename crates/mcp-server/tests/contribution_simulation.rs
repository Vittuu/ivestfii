//! Full stdio-tool contract with synthetic local balances. No network or keys.
use ivestfii_assistant::AssistantStore;
use ivestfii_core::{CashPocketInput, FundInput, RecordInput, WalletCatalog, WalletInput};
use ivestfii_mcp::{McpServer, PROTOCOL_VERSION};
use serde_json::{json, Value};
use std::{fs, path::Path};

fn ready(path: &Path, wallet: Option<&str>) -> McpServer {
    let mut server = McpServer::open_for_wallet(path, wallet).unwrap();
    let reply = server.handle_frame(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":PROTOCOL_VERSION,"capabilities":{},"clientInfo":{"name":"contribution-fixture","version":"1"}
    }}).to_string().as_bytes()).unwrap();
    assert!(reply["result"]["instructions"]
        .as_str()
        .unwrap()
        .contains("simular_aporte"));
    server.handle_frame(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    server
}

fn call(server: &mut McpServer, arguments: Value) -> Value {
    server
        .handle_frame(
            json!({"jsonrpc":"2.0","id":"simulate","method":"tools/call","params":{
                "name":"simular_aporte","arguments":arguments
            }})
            .to_string()
            .as_bytes(),
        )
        .unwrap()["result"]
        .clone()
}

fn budget() -> Value {
    json!({"amount":"300","allocations":{"fiis":"100","fixedIncome":"100","cashPockets":"100"}})
}

fn content(result: &Value) -> &Value {
    assert_eq!(result["isError"], false, "{result}");
    &result["structuredContent"]
}

fn row<'a>(simulation: &'a Value, kind: &str) -> &'a Value {
    simulation["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["kind"] == kind)
        .unwrap()
}

fn empty() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    WalletCatalog::open(directory.path()).unwrap();
    AssistantStore::open(directory.path().join("assistant.sqlite")).unwrap();
    directory
}

fn populated(directory: &Path) {
    let catalog = WalletCatalog::open(directory).unwrap();
    let repo = catalog.repository(None).unwrap();
    repo.upsert_fund(FundInput {
        id: None,
        ticker: "TEST11".into(),
        name: "Fundo sintético".into(),
        sector: "Teste".into(),
    })
    .unwrap();
    let fund = repo.snapshot().unwrap().funds.remove(0);
    repo.save_monthly_record(RecordInput {
        id: None,
        fund_id: fund.id,
        month: "2020-01".into(),
        purchase_date: Some("2020-01-10".into()),
        cotas_added: "10".into(),
        price_per_cota: "100".into(),
        dividend_per_cota: "0".into(),
        dividend_total: Some("0".into()),
        notes: "Compra sintética".into(),
    })
    .unwrap();
    let pocket: CashPocketInput = serde_json::from_value(json!({"name":"Reserva sintética","institution":"Teste","balance":"400","totalDeposits":"2000","totalWithdrawals":"1600","balanceDate":"2020-02-09","cdiPercent":"100","annualCdiRate":null,"notes":""})).unwrap();
    repo.upsert_cash_pocket(pocket).unwrap();
    let broker = directory.join("broker-fixture.json");
    fs::write(&broker, serde_json::to_vec(&json!({"broker_position":{
        "referenceDate":"2020-02-10","institution":"Teste","importedAt":"2020-02-12T12:00:00Z","sourcePath":"broker-fixture.json","sourceHash":"a".repeat(64),
        "overview":{"updatedTime":null,"capturedTime":null,"totalWithCash":"99999","invested":"1600","cash":"98399","listedFunds":"1100","fixedIncome":"500","pendingIncome":"0"},
        "listedFunds":{"capturedTime":null,"marketValue":"1100","costBasis":"1000","unrealizedResult":"100","dailyResult":"0","holdings":[{"ticker":"TEST11","shares":"10","averagePrice":"100","price":"110","marketValue":"1100","costBasis":"1000","unrealizedResult":"100"}]},
        "fixedIncome":{"name":"Pós sintético","nameTruncated":false,"balance":"500","netBalance":"500","reportedEarnings":"99","return12Months":"15","redemptionConversion":"D+0","redemptionSettlement":"D+0"},"warnings":[]
    }})).unwrap()).unwrap();
    repo.import_broker_position_file(&broker).unwrap();
    fs::create_dir_all(directory.join("public-data")).unwrap();
    fs::write(directory.join("public-data/quotes.json"), serde_json::to_vec(&json!({"version":1,"observations":{"TEST11":{
        "symbol":"TEST11","price":"110","currency":"BRL","quotedAt":"2020-02-11T20:00:00Z","fetchedAt":"2020-02-12T12:00:00Z","source":"brapi","sourceUrl":"https://brapi.dev/api/v2/stocks/quote?symbols=TEST11","delayMinutes":30
    }},"lastAttemptAt":null,"retryAfter":null})).unwrap()).unwrap();
}

fn state(path: &Path) -> Value {
    let catalog = WalletCatalog::open(path).unwrap();
    let repo = catalog.repository(None).unwrap();
    json!({"export":repo.export_legacy_json().unwrap(),"snapshot":repo.snapshot().unwrap(),"pending":repo.pending_change_count().unwrap()})
}

fn fresh_quotes(path: &Path, include_other: bool) {
    let now = chrono::Utc::now();
    let mut observations = json!({"TEST11":{
        "symbol":"TEST11","price":"110","currency":"BRL","quotedAt":(now-chrono::Duration::seconds(20)).to_rfc3339(),"fetchedAt":(now-chrono::Duration::seconds(10)).to_rfc3339(),"source":"brapi","sourceUrl":"https://brapi.dev/api/v2/stocks/quote?symbols=TEST11","delayMinutes":30
    }});
    if include_other {
        observations["OTHER11"] = json!({"symbol":"OTHER11","price":"21","currency":"BRL","quotedAt":(now-chrono::Duration::seconds(20)).to_rfc3339(),"fetchedAt":(now-chrono::Duration::seconds(10)).to_rfc3339(),"source":"brapi","sourceUrl":"https://brapi.dev/api/v2/stocks/quote?symbols=OTHER11","delayMinutes":30});
    }
    fs::write(path.join("public-data/quotes.json"),serde_json::to_vec(&json!({"version":1,"observations":observations,"lastAttemptAt":null,"retryAfter":null})).unwrap()).unwrap();
}

fn detailed_budget(path: &Path) -> Value {
    let fund = WalletCatalog::open(path)
        .unwrap()
        .repository(None)
        .unwrap()
        .snapshot()
        .unwrap()
        .funds
        .into_iter()
        .find(|fund| fund.ticker == "TEST11")
        .unwrap();
    json!({"amount":"300","allocations":{"fiis":"250","fixedIncome":"25","cashPockets":"25"},"fundAllocations":[{"fundId":fund.id,"amount":"250"}]})
}

#[test]
fn whole_shares_leave_change_separate_and_use_actual_spend_for_portfolio_weights() {
    let directory = empty();
    populated(directory.path());
    fresh_quotes(directory.path(), false);
    let before = state(directory.path());
    let cache = fs::read(directory.path().join("public-data/quotes.json")).unwrap();
    let mut server = ready(directory.path(), None);
    let result = call(&mut server, detailed_budget(directory.path()));
    let response = content(&result);
    let funds = &response["fundSimulation"];
    let purchase = &funds["rows"][0];
    let allocation = &funds["portfolioAllocation"];
    assert_eq!(funds["status"], "calculated", "{funds}");
    assert_eq!(funds["budget"], "250");
    assert_eq!(purchase["currentShares"], "10");
    assert_eq!(purchase["sharesToBuy"], "2");
    assert_eq!(purchase["afterShares"], "12");
    assert_eq!(purchase["price"], "110");
    assert_eq!(purchase["spentAmount"], "220");
    assert_eq!(purchase["remainder"], "30");
    assert_eq!(purchase["beforeFiiPercent"], "100");
    assert_eq!(purchase["afterFiiPercent"], "100");
    assert_eq!(purchase["beforePortfolioPercent"], "55");
    assert_eq!(purchase["afterPortfolioPercent"], "57.39");
    assert_eq!(funds["spentAmount"], "220");
    assert_eq!(funds["remainder"], "30");
    assert_eq!(allocation["totalBefore"], "2000");
    assert_eq!(allocation["totalAfter"], "2300");
    assert_eq!(allocation["unallocatedCash"], "30");
    assert_eq!(row(allocation, "fiis")["afterValue"], "1320");
    assert_eq!(row(allocation, "fixedIncome")["afterValue"], "525");
    assert_eq!(row(allocation, "cashPockets")["afterValue"], "425");
    assert_eq!(row(allocation, "unallocatedCash")["afterValue"], "30");
    assert_eq!(row(&response["simulation"], "fiis")["afterValue"], "1350"); // Old mode remains a reservation, not whole shares.
    assert_eq!(response["quotePolicy"]["noNetworkRequests"], true);
    assert_eq!(
        content(&call(&mut server, detailed_budget(directory.path())))["fundSimulation"],
        *funds
    );
    assert_eq!(state(directory.path()), before);
    assert_eq!(
        fs::read(directory.path().join("public-data/quotes.json")).unwrap(),
        cache
    );
    assert!(!directory
        .path()
        .join("public-data/quotes.refresh-policy.json")
        .exists());
}

#[test]
fn every_held_fund_is_included_even_without_an_allocation_and_foreign_ids_are_rejected() {
    let directory = empty();
    populated(directory.path());
    let catalog = WalletCatalog::open(directory.path()).unwrap();
    let repo = catalog.repository(None).unwrap();
    repo.upsert_fund(FundInput {
        id: None,
        ticker: "OTHER11".into(),
        name: "Outro fundo".into(),
        sector: "Papel".into(),
    })
    .unwrap();
    let other = repo
        .snapshot()
        .unwrap()
        .funds
        .into_iter()
        .find(|fund| fund.ticker == "OTHER11")
        .unwrap();
    repo.save_monthly_record(RecordInput {
        id: None,
        fund_id: other.id,
        month: "2020-03".into(),
        purchase_date: Some("2020-03-10".into()),
        cotas_added: "5".into(),
        price_per_cota: "20".into(),
        dividend_per_cota: "0".into(),
        dividend_total: Some("0".into()),
        notes: String::new(),
    })
    .unwrap();
    fresh_quotes(directory.path(), true);
    let mut server = ready(directory.path(), None);
    let result = call(&mut server, detailed_budget(directory.path()));
    let funds = &content(&result)["fundSimulation"];
    assert_eq!(funds["rows"].as_array().unwrap().len(), 2);
    assert_eq!(funds["fiiBasisComplete"], true, "{funds}");
    let unchanged = funds["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["symbol"] == "OTHER11")
        .unwrap();
    assert_eq!(unchanged["budget"], "0");
    assert_eq!(unchanged["sharesToBuy"], "0");
    assert_eq!(unchanged["afterShares"], "5");
    assert_eq!(unchanged["currentValue"], "105");
    assert_eq!(unchanged["beforeFiiPercent"], "8.71");
    assert_eq!(unchanged["afterFiiPercent"], "7.37");
    let wallet = catalog
        .create(WalletInput {
            id: None,
            name: "Outra".into(),
            institution: "Teste".into(),
        })
        .unwrap();
    let foreign = catalog.repository(Some(&wallet.id)).unwrap();
    foreign
        .upsert_fund(FundInput {
            id: None,
            ticker: "TEST11".into(),
            name: "Isolado".into(),
            sector: String::new(),
        })
        .unwrap();
    let foreign_id = foreign.snapshot().unwrap().funds.remove(0).id;
    let mut invalid = detailed_budget(directory.path());
    invalid["fundAllocations"][0]["fundId"] = json!(foreign_id);
    assert_eq!(call(&mut server, invalid)["isError"], true);
}

#[test]
fn stale_quote_never_becomes_zero_shares_or_a_known_after_allocation() {
    let directory = empty();
    populated(directory.path());
    let mut server = ready(directory.path(), None);
    let result = call(&mut server, detailed_budget(directory.path()));
    let funds = &content(&result)["fundSimulation"];
    assert_eq!(funds["status"], "incomplete");
    assert_eq!(funds["rows"][0]["quoteStatus"], "stale");
    for field in [
        "price",
        "sharesToBuy",
        "spentAmount",
        "remainder",
        "afterFiiPercent",
        "afterPortfolioPercent",
    ] {
        assert!(funds["rows"][0][field].is_null(), "{field}: {funds}");
    }
    assert!(funds["spentAmount"].is_null() && funds["remainder"].is_null());
    assert!(funds["portfolioAllocation"]["totalAfter"].is_null());
}

#[test]
fn per_fund_input_cannot_override_quotes_quantities_or_scope_and_must_conserve_budget() {
    let directory = empty();
    populated(directory.path());
    fresh_quotes(directory.path(), false);
    let mut server = ready(directory.path(), None);
    let before = state(directory.path());
    let valid = detailed_budget(directory.path());
    for allocations in [
        Value::Null,
        json!([]),
        json!([{"fundId":"unknown","amount":"250"}]),
        json!([{"fundId":valid["fundAllocations"][0]["fundId"],"amount":"249"}]),
        json!([{"fundId":valid["fundAllocations"][0]["fundId"],"amount":"250","price":"1"}]),
        json!([{"fundId":valid["fundAllocations"][0]["fundId"],"amount":"250","sharesToBuy":"999"}]),
        json!([{"fundId":valid["fundAllocations"][0]["fundId"],"amount":"125"},{"fundId":valid["fundAllocations"][0]["fundId"],"amount":"125"}]),
    ] {
        let mut invalid = valid.clone();
        invalid["fundAllocations"] = allocations;
        assert_eq!(call(&mut server, invalid)["isError"], true);
    }
    assert_eq!(state(directory.path()), before);
}

#[test]
fn before_and_after_use_market_balances_and_preserve_wallet_and_public_cache() {
    let directory = empty();
    populated(directory.path());
    let before = state(directory.path());
    let quotes = fs::read(directory.path().join("public-data/quotes.json")).unwrap();
    let mut server = ready(directory.path(), None);
    let result = call(&mut server, budget());
    let response = content(&result);
    let simulation = &response["simulation"];
    assert_eq!(response["kind"], "contribution-allocation");
    assert_eq!(response["refreshRequested"], false);
    assert_eq!(response["recommendationGenerated"], false);
    assert_eq!(response["profileCompleteness"]["canPersonalize"], false);
    assert_eq!(simulation["status"], "calculated", "{simulation}");
    assert_eq!(simulation["totalBefore"], "2000");
    assert_eq!(simulation["totalAfter"], "2300");
    assert_eq!(row(simulation, "fiis")["currentValue"], "1100");
    assert_eq!(row(simulation, "fiis")["afterValue"], "1200");
    assert_eq!(row(simulation, "fiis")["beforePercent"], "55");
    assert_eq!(row(simulation, "fiis")["afterPercent"], "52.17");
    assert_eq!(row(simulation, "fiis")["changePercentagePoints"], "-2.83");
    assert_eq!(row(simulation, "cashPockets")["currentValue"], "400");
    assert_eq!(row(simulation, "fixedIncome")["currentValue"], "500");
    assert_eq!(simulation["valuationBasis"], "mixed-dates");
    assert!(simulation["asOf"].is_null());
    assert_eq!(simulation["fromDate"], "2020-02-09");
    assert_eq!(simulation["toDate"], "2020-02-11");
    assert_eq!(
        content(&call(&mut server, budget()))["simulation"],
        *simulation
    );
    assert_eq!(state(directory.path()), before);
    assert_eq!(
        fs::read(directory.path().join("public-data/quotes.json")).unwrap(),
        quotes
    );
    assert!(!directory
        .path()
        .join("public-data/quotes.refresh-policy.json")
        .exists());
}

#[test]
fn unavailable_quotes_never_become_zero_or_subset_percentages() {
    let directory = empty();
    populated(directory.path());
    fs::remove_file(directory.path().join("public-data/quotes.json")).unwrap();
    let before = state(directory.path());
    let mut server = ready(directory.path(), None);
    let result = call(&mut server, budget());
    let simulation = &content(&result)["simulation"];
    assert_eq!(simulation["status"], "incomplete");
    assert!(simulation["totalBefore"].is_null());
    assert!(simulation["totalAfter"].is_null());
    assert!(row(simulation, "fiis")["afterValue"].is_null());
    assert_eq!(row(simulation, "cashPockets")["afterValue"], "500");
    for row in simulation["rows"].as_array().unwrap() {
        assert!(row["beforePercent"].is_null() && row["afterPercent"].is_null());
    }
    assert_eq!(state(directory.path()), before);
}

#[test]
fn selected_empty_wallet_can_simulate_explicit_split_without_a_profile_or_global_holdings() {
    let directory = empty();
    populated(directory.path());
    let catalog = WalletCatalog::open(directory.path()).unwrap();
    let other = catalog
        .create(WalletInput {
            id: None,
            name: "Outra carteira".into(),
            institution: "Teste".into(),
        })
        .unwrap();
    let before = state(directory.path());
    let mut server = ready(directory.path(), Some(&other.id));
    let result = call(&mut server, budget());
    let response = content(&result);
    assert_eq!(response["wallet"]["id"], other.id);
    assert_eq!(response["simulation"]["totalBefore"], "0");
    assert_eq!(response["simulation"]["totalAfter"], "300");
    for row in response["simulation"]["rows"].as_array().unwrap() {
        assert!(row["beforePercent"].is_null());
        assert_eq!(row["afterPercent"], "33.33");
    }
    let mut override_budget = budget();
    override_budget["walletId"] = json!(catalog.get(None).unwrap().id);
    assert_eq!(
        content(&call(&mut server, override_budget))["simulation"]["totalBefore"],
        "2000"
    );
    assert_eq!(state(directory.path()), before);
}

#[test]
fn rejects_unknown_wallet_malformed_or_unbalanced_budgets_and_extra_fields() {
    let directory = empty();
    let mut server = ready(directory.path(), None);
    for invalid in [
        json!({"amount":"300","allocations":{"fiis":"100","fixedIncome":"100","cashPockets":"99"}}),
        json!({"amount":"300","allocations":{"fiis":"100","fixedIncome":"100","cashPockets":"101"}}),
        json!({"amount":"0","allocations":{"fiis":"0","fixedIncome":"0","cashPockets":"0"}}),
        json!({"amount":"300.001","allocations":{"fiis":"100.001","fixedIncome":"100","cashPockets":"100"}}),
        json!({"amount":300,"allocations":{"fiis":"100","fixedIncome":"100","cashPockets":"100"}}),
        json!({"amount":"300","allocations":{"fiis":"100","fixedIncome":"100","cashPockets":"100","execute":true}}),
        json!({"amount":"300","allocations":{"fiis":"100","fixedIncome":"100","cashPockets":"100"},"refreshPublicDataIfDue":true}),
        json!({"amount":"300","allocations":{"fiis":"100","fixedIncome":"100","cashPockets":"100"},"walletId":"not-a-wallet"}),
    ] {
        assert_eq!(call(&mut server, invalid)["isError"], true);
    }
    assert!(!directory.path().join("public-data").exists());
}
