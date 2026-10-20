use chrono::NaiveDate;
use ivestfii_core::income::IncomeEvent;
use ivestfii_core::performance::{
    calculate_fii_performance, calculate_fii_performance_with_income, DailyCdiRate, FiiPerformance,
    PerformanceQuote,
};
use ivestfii_core::portfolio_performance::calculate_portfolio_performance;
use ivestfii_core::{CashPocket, FundInput, RecordInput, Repository};
use tempfile::TempDir;

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 5, 20).unwrap()
}

fn record(
    repo: &Repository,
    id: &str,
    month: &str,
    shares: &str,
    day: Option<&str>,
    amount: Option<&str>,
) {
    repo.save_monthly_record(RecordInput {
        id: None,
        fund_id: id.into(),
        month: month.into(),
        purchase_date: day.map(Into::into),
        cotas_added: shares.into(),
        price_per_cota: "10".into(),
        dividend_per_cota: "0".into(),
        dividend_total: amount.map(Into::into),
        notes: String::new(),
    })
    .unwrap();
}

fn fixture() -> (TempDir, Repository, String) {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    repo.upsert_fund(FundInput {
        id: None,
        ticker: "TEST11".into(),
        name: "Fundo sintético".into(),
        sector: "Teste".into(),
    })
    .unwrap();
    let id = repo.snapshot().unwrap().funds[0].id.clone();
    record(&repo, &id, "2026-01", "10", None, Some("2"));
    record(&repo, &id, "2026-02", "0", None, Some("0"));
    (dir, repo, id)
}

fn quote(day: &str) -> Vec<PerformanceQuote> {
    vec![PerformanceQuote {
        ticker: "TEST11".into(),
        price: "12".into(),
        as_of: day.into(),
    }]
}

fn rates() -> Vec<DailyCdiRate> {
    [
        ("2026-01-30", "1"),
        ("2026-02-02", "1"),
        ("2026-03-15", "2"),
        ("2026-04-30", "3"),
    ]
    .into_iter()
    .map(|(date, value)| DailyCdiRate {
        date: date.into(),
        value: value.into(),
    })
    .collect()
}

fn event(id: &str, rate: &str, cutoff: &str, paid: &str) -> IncomeEvent {
    IncomeEvent {
        symbol: "TEST11".into(),
        amount_per_share: rate.into(),
        record_date: cutoff.into(),
        payment_date: paid.into(),
        source_url: "https://example.test/proventos".into(),
        source: "fixture".into(),
        id: id.into(),
    }
}

fn events() -> Vec<IncomeEvent> {
    vec![
        event("manual-month", "9", "2026-01-31", "2026-02-15"),
        event("march-one", "0.1", "2026-02-28", "2026-03-15"),
        event("march-two", "0.05", "2026-03-01", "2026-03-31"),
        event("april", "0.2", "2026-03-31", "2026-04-30"),
        event("later-than-quote", "99", "2026-04-30", "2026-05-15"),
    ]
}

#[test]
fn api_complement_changes_income_result_and_cdi_difference_once_with_the_same_cutoff() {
    let (_dir, repo, id) = fixture();
    let snapshot = repo.snapshot().unwrap();
    let before = serde_json::to_vec(&snapshot).unwrap();
    let backup = repo.export_legacy_json().unwrap();
    let outbox = serde_json::to_vec(&repo.pending_changes().unwrap()).unwrap();
    let legacy =
        calculate_fii_performance(&snapshot, &quote("2026-04-30"), &rates(), today()).unwrap();
    let mut events = events();
    events.push(events[1].clone());
    let result = calculate_fii_performance_with_income(
        &snapshot,
        &quote("2026-04-30"),
        &rates(),
        &events,
        today(),
    )
    .unwrap();
    assert_eq!(result.status, "available");
    assert_eq!(result.invested, "100");
    assert_eq!(legacy.dividends, "2");
    assert_eq!(result.dividends, "5.5");
    assert_eq!(result.market_value.as_deref(), Some("120"));
    assert_eq!(result.price_result.as_deref(), Some("20"));
    assert_eq!(result.total_result.as_deref(), Some("25.5"));
    assert_eq!(result.total_return_percent.as_deref(), Some("25.5"));
    assert_eq!(result.cdi.as_ref().unwrap().result, "6.1106");
    assert_eq!(
        result.cdi.as_ref().unwrap().result,
        legacy.cdi.as_ref().unwrap().result
    );
    assert_eq!(result.difference.as_deref(), Some("19.3894"));
    let income = result.income.as_ref().unwrap();
    assert_eq!(income.recorded_amount, "2");
    assert_eq!(income.api_amount, "3.5");
    assert_eq!(income.as_of, "2026-04-30");
    assert_eq!(income.start_month, "2026-01");
    assert_eq!(income.end_month, "2026-04");
    assert_eq!(income.status, "complete");
    assert_eq!(income.ignored_event_count, 2);
    assert_eq!(income.by_fund[0].fund_id, id);
    assert_eq!(income.by_fund[0].recorded_amount, "2");
    assert_eq!(income.by_fund[0].api_amount, "3.5");
    assert_eq!(income.by_fund[0].total_amount, "5.5");
    let wallet = calculate_portfolio_performance(&snapshot, &result).unwrap();
    assert_eq!(wallet.status, "available");
    assert_eq!(wallet.income.as_deref(), Some("5.5"));
    assert_eq!(wallet.total_result.as_deref(), Some("25.5"));
    assert_eq!(
        serde_json::to_vec(&repo.snapshot().unwrap()).unwrap(),
        before
    );
    assert_eq!(repo.export_legacy_json().unwrap(), backup);
    assert_eq!(
        serde_json::to_vec(&repo.pending_changes().unwrap()).unwrap(),
        outbox
    );
    assert_eq!(
        serde_json::to_value(
            calculate_fii_performance_with_income(
                &snapshot,
                &quote("2026-04-30"),
                &rates(),
                &events,
                today()
            )
            .unwrap()
        )
        .unwrap(),
        serde_json::to_value(result).unwrap()
    );
}

#[test]
fn manual_replacement_and_explicit_zero_remove_only_that_fund_month_api_complement() {
    let (_dir, repo, id) = fixture();
    record(&repo, &id, "2026-03", "0", None, Some("0"));
    let result = calculate_fii_performance_with_income(
        &repo.snapshot().unwrap(),
        &quote("2026-04-30"),
        &rates(),
        &events(),
        today(),
    )
    .unwrap();
    assert_eq!(result.dividends, "4");
    assert_eq!(result.income.as_ref().unwrap().api_amount, "2");
    assert_eq!(result.total_result.as_deref(), Some("24"));
    assert_eq!(result.difference.as_deref(), Some("17.8894"));
    record(&repo, &id, "2026-04", "0", None, Some("7.123456"));
    let result = calculate_fii_performance_with_income(
        &repo.snapshot().unwrap(),
        &quote("2026-04-30"),
        &rates(),
        &events(),
        today(),
    )
    .unwrap();
    assert_eq!(result.dividends, "9.123456");
    assert_eq!(result.income.as_ref().unwrap().api_amount, "0");
    assert_eq!(result.income.as_ref().unwrap().recorded_amount, "9.123456");
}

fn cash(complete: bool) -> CashPocket {
    CashPocket {
        id: "synthetic-pocket".into(),
        name: "Caixinha sintética".into(),
        institution: "Teste".into(),
        balance: "220".into(),
        total_deposits: complete.then(|| "200".into()),
        total_withdrawals: complete.then(|| "10".into()),
        balance_date: complete.then(|| "2026-04-30".into()),
        cdi_percent: "120".into(),
        annual_cdi_rate: Some("15".into()),
        rate_date: None,
        rate_source: "manual".into(),
        rate_source_url: None,
        rate_fetched_at: None,
        notes: String::new(),
        created_at: String::new(),
        updated_at: String::new(),
    }
}

#[test]
fn reconciled_pockets_add_real_income_and_missing_pocket_basis_stays_unknown() {
    let (_dir, repo, _id) = fixture();
    let mut snapshot = repo.snapshot().unwrap();
    let result = calculate_fii_performance_with_income(
        &snapshot,
        &quote("2026-04-30"),
        &[],
        &events(),
        today(),
    )
    .unwrap();
    assert!(result.cdi.is_none());
    snapshot.cash_pockets.push(cash(true));
    let wallet = calculate_portfolio_performance(&snapshot, &result).unwrap();
    assert_eq!(wallet.status, "available");
    assert_eq!(wallet.invested.as_deref(), Some("300"));
    assert_eq!(wallet.income.as_deref(), Some("35.5"));
    assert_eq!(wallet.total_result.as_deref(), Some("55.5"));
    assert_eq!(wallet.current_value.as_deref(), Some("340"));
    snapshot.cash_pockets[0] = cash(false);
    let wallet = calculate_portfolio_performance(&snapshot, &result).unwrap();
    assert_eq!(wallet.status, "incomplete");
    assert!(wallet.income.is_none() && wallet.total_result.is_none() && wallet.invested.is_none());
    assert_eq!(wallet.known_income, "5.5");
    assert_eq!(wallet.known_result, "25.5");
    assert_eq!(wallet.cash.missing_basis_count, 1);
}

#[test]
fn unknown_purchase_day_and_missing_events_keep_explicit_partial_income_coverage() {
    let (_dir, repo, id) = fixture();
    record(&repo, &id, "2026-03", "2", None, None);
    let mut events = events();
    events[1].record_date = "2026-03-10".into();
    let result = calculate_fii_performance_with_income(
        &repo.snapshot().unwrap(),
        &quote("2026-04-30"),
        &rates(),
        &events,
        today(),
    )
    .unwrap();
    assert_eq!(result.dividends, "2");
    assert_eq!(result.total_result.as_deref(), Some("26"));
    let income = result.income.as_ref().unwrap();
    assert_eq!(income.status, "partial");
    assert_eq!(income.missing_fund_month_count, 2);
    assert_eq!(income.date_required_fund_month_count, 2);
    assert_eq!(income.by_fund[0].missing_month_count, 2);
    assert_eq!(income.by_fund[0].date_required_month_count, 2);
    assert_eq!(
        result
            .issues
            .iter()
            .filter(|issue| issue.contains("data das compras antigas"))
            .count(),
        1
    );
    let wallet = calculate_portfolio_performance(&repo.snapshot().unwrap(), &result).unwrap();
    assert_eq!(wallet.income.as_deref(), Some("2"));
    assert_eq!(wallet.total_result.as_deref(), Some("26"));
    assert!(wallet.issues.is_empty());
    let no_events = calculate_fii_performance_with_income(
        &repo.snapshot().unwrap(),
        &quote("2026-04-30"),
        &rates(),
        &[],
        today(),
    )
    .unwrap();
    assert_eq!(no_events.income.unwrap().date_required_fund_month_count, 0);
}

#[test]
fn dated_purchases_earn_cdi_after_their_actual_day_and_later_purchases_are_excluded() {
    let (_dir, repo, id) = fixture();
    record(&repo, &id, "2026-03", "2", Some("2026-03-01"), None);
    let events = [event("march", "0.1", "2026-03-10", "2026-03-15")];
    let result = calculate_fii_performance_with_income(
        &repo.snapshot().unwrap(),
        &quote("2026-04-30"),
        &rates(),
        &events,
        today(),
    )
    .unwrap();
    assert_eq!(result.invested, "120");
    assert_eq!(result.dividends, "3.2");
    assert_eq!(result.cdi.as_ref().unwrap().result, "7.1226");
    record(&repo, &id, "2026-04", "100", Some("2026-04-20"), None);
    let result = calculate_fii_performance_with_income(
        &repo.snapshot().unwrap(),
        &quote("2026-04-15"),
        &rates(),
        &events,
        today(),
    )
    .unwrap();
    assert_eq!(result.status, "incomplete");
    assert_eq!(result.invested, "120");
    assert_eq!(result.dividends, "3.2");
    assert!(result.market_value.is_none() && result.total_result.is_none());
    let wallet = calculate_portfolio_performance(&repo.snapshot().unwrap(), &result).unwrap();
    assert!(wallet.invested.is_none() && wallet.income.is_none() && wallet.total_result.is_none());
}

#[test]
fn invalid_events_do_not_invent_income_and_old_reports_deserialize_without_provenance() {
    let (_dir, repo, _id) = fixture();
    let mut events = events();
    events[1].amount_per_share = "invalid".into();
    events[3].record_date = "2026-02-30".into();
    let result = calculate_fii_performance_with_income(
        &repo.snapshot().unwrap(),
        &quote("2026-04-30"),
        &rates(),
        &events,
        today(),
    )
    .unwrap();
    assert_eq!(result.dividends, "2");
    assert_eq!(result.income.as_ref().unwrap().api_amount, "0");
    assert_eq!(result.income.as_ref().unwrap().ignored_event_count, 3);
    let mut serialized = serde_json::to_value(result).unwrap();
    assert_eq!(serialized["income"]["recordedAmount"], "2");
    assert_eq!(serialized["income"]["byFund"][0]["totalAmount"], "2");
    serialized.as_object_mut().unwrap().remove("income");
    let legacy = serde_json::from_value::<FiiPerformance>(serialized).unwrap();
    assert!(legacy.income.is_none());
    let mut invalid = legacy;
    invalid.income = Some(serde_json::from_value(serde_json::json!({
        "recordedAmount":"2", "apiAmount":"999", "asOf":"2026-04-30", "startMonth":"2026-01", "endMonth":"2026-04",
        "status":"complete", "missingFundMonthCount":0, "dateRequiredFundMonthCount":0,
        "ignoredEventCount":0, "byFund":[]
    })).unwrap());
    assert!(calculate_portfolio_performance(&repo.snapshot().unwrap(), &invalid).is_err());
}

#[test]
fn manual_priority_applies_to_each_fund_and_does_not_hide_another_funds_api_payment() {
    let (_dir, repo, _id) = fixture();
    repo.upsert_fund(FundInput {
        id: None,
        ticker: "OTRO11".into(),
        name: "Outro fundo sintético".into(),
        sector: "Teste".into(),
    })
    .unwrap();
    let other = repo
        .snapshot()
        .unwrap()
        .funds
        .iter()
        .find(|fund| fund.ticker == "OTRO11")
        .unwrap()
        .id
        .clone();
    record(&repo, &other, "2026-01", "3", Some("2026-01-31"), Some("0"));
    let mut events = events();
    events.push(IncomeEvent {
        symbol: "OTRO11".into(),
        ..event("other-february", "0.33", "2026-01-31", "2026-02-15")
    });
    let mut quotes = quote("2026-04-30");
    quotes.push(PerformanceQuote {
        ticker: "OTRO11".into(),
        price: "10".into(),
        as_of: "2026-04-30".into(),
    });
    let snapshot = repo.snapshot().unwrap();
    let result =
        calculate_fii_performance_with_income(&snapshot, &quotes, &rates(), &events, today())
            .unwrap();
    assert_eq!(result.dividends, "6.49");
    assert_eq!(result.invested, "130");
    assert_eq!(result.market_value.as_deref(), Some("150"));
    assert_eq!(result.total_result.as_deref(), Some("26.49"));
    let income = result.income.as_ref().unwrap();
    assert_eq!(income.recorded_amount, "2");
    assert_eq!(income.api_amount, "4.49");
    assert_eq!(income.status, "partial");
    let other = income
        .by_fund
        .iter()
        .find(|fund| fund.symbol == "OTRO11")
        .unwrap();
    assert_eq!(other.total_amount, "0.99");
    assert_eq!(other.api_amount, "0.99");
    assert_eq!(other.recorded_amount, "0");
    assert_eq!(other.missing_month_count, 2);
    let wallet = calculate_portfolio_performance(&snapshot, &result).unwrap();
    assert_eq!(wallet.income.as_deref(), Some("6.49"));
    assert_eq!(wallet.total_result.as_deref(), Some("26.49"));
}

#[test]
fn a_cache_without_common_quotes_can_expose_known_income_but_never_a_market_result() {
    let (_dir, repo, _id) = fixture();
    let snapshot = repo.snapshot().unwrap();
    let result =
        calculate_fii_performance_with_income(&snapshot, &[], &rates(), &events(), today())
            .unwrap();
    assert_eq!(result.dividends, "995.5");
    assert_eq!(result.income.as_ref().unwrap().api_amount, "993.5");
    assert_eq!(result.income.as_ref().unwrap().as_of, "2026-05-20");
    assert!(
        result.as_of.is_none() && result.market_value.is_none() && result.total_result.is_none()
    );
    assert!(result.cdi.is_none() && result.difference.is_none());
    let wallet = calculate_portfolio_performance(&snapshot, &result).unwrap();
    assert_eq!(wallet.income.as_deref(), Some("995.5"));
    assert!(wallet.total_result.is_none());
}
