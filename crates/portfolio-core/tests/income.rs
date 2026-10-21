use chrono::NaiveDate;
use ivestfii_core::income::{calculate_monthly_income, IncomeEvent, MonthlyIncomeReport};
use ivestfii_core::{FundInput, RecordInput, Repository};
use serde_json::Value;
use tempfile::TempDir;

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 20).unwrap()
}

fn fund(repo: &Repository, ticker: &str) -> String {
    repo.upsert_fund(FundInput {
        id: None,
        ticker: ticker.into(),
        name: "Fundo sintético".into(),
        sector: "Teste".into(),
    })
    .unwrap();
    repo.snapshot()
        .unwrap()
        .funds
        .iter()
        .find(|fund| fund.ticker == ticker)
        .unwrap()
        .id
        .clone()
}

fn record(
    repo: &Repository,
    fund_id: &str,
    month: &str,
    quantity: &str,
    date: Option<&str>,
    total: Option<&str>,
    rate: &str,
) {
    repo.save_monthly_record(RecordInput {
        id: None,
        fund_id: fund_id.into(),
        month: month.into(),
        purchase_date: date.map(Into::into),
        cotas_added: quantity.into(),
        price_per_cota: "10".into(),
        dividend_per_cota: rate.into(),
        dividend_total: total.map(Into::into),
        notes: "Teste sintético".into(),
    })
    .unwrap();
}

fn event(id: &str, rate: &str, cutoff: &str, payment: &str) -> IncomeEvent {
    IncomeEvent {
        symbol: "TEST11".into(),
        amount_per_share: rate.into(),
        record_date: cutoff.into(),
        payment_date: payment.into(),
        source_url: "https://example.test/proventos".into(),
        source: "fixture".into(),
        id: id.into(),
    }
}

fn report(
    repo: &Repository,
    events: &[IncomeEvent],
    start: &str,
    end: &str,
) -> MonthlyIncomeReport {
    calculate_monthly_income(&repo.snapshot().unwrap(), events, start, end, today()).unwrap()
}

#[test]
fn unchanged_holdings_fill_a_missing_month_from_its_real_event_without_writing_a_receipt() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    let id = fund(&repo, "TEST11");
    record(&repo, &id, "2026-05", "55", None, Some("5.5"), "0.1");
    record(&repo, &id, "2026-06", "0", None, Some("5.5"), "0.1");
    record(&repo, &id, "2026-08", "0", None, Some("5.5"), "0.1");
    let snapshot = serde_json::to_value(repo.snapshot().unwrap()).unwrap();
    let backup = repo.export_legacy_json().unwrap();
    let outbox = serde_json::to_value(repo.pending_changes().unwrap()).unwrap();
    let events = [event("july", "0.11", "2026-06-30", "2026-07-15")];
    let result = report(&repo, &events, "2026-05", "2026-08");
    assert_eq!(
        result
            .months
            .iter()
            .map(|point| point.known_total.as_str())
            .collect::<Vec<_>>(),
        vec!["5.5", "5.5", "6.05", "5.5"]
    );
    let july = &result.months[2];
    assert_eq!(july.status, "complete");
    assert_eq!(july.total.as_deref(), Some("6.05"));
    assert_eq!(july.api_amount, "6.05");
    assert_eq!(july.recorded_amount, "0");
    assert_eq!(july.funds[0].status, "api");
    assert_eq!(july.funds[0].holdings, "55");
    assert_eq!(july.funds[0].eligible_shares.as_deref(), Some("55"));
    assert_eq!(july.funds[0].events[0].record_date, "2026-06-30");
    assert_eq!(july.funds[0].events[0].payment_date, "2026-07-15");
    assert_eq!(july.funds[0].events[0].source, "fixture");
    assert!(july.funds[0].manual_record_ids.is_empty());
    assert_eq!(
        serde_json::to_value(repo.snapshot().unwrap()).unwrap(),
        snapshot
    );
    assert_eq!(repo.export_legacy_json().unwrap(), backup);
    assert_eq!(
        serde_json::to_value(repo.pending_changes().unwrap()).unwrap(),
        outbox
    );
    assert_eq!(report(&repo, &events, "2026-05", "2026-08"), result);
}

#[test]
fn entitlement_uses_the_purchase_day_and_includes_the_record_date_but_not_later_buys() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    let id = fund(&repo, "TEST11");
    record(&repo, &id, "2026-05", "10", None, None, "0");
    record(&repo, &id, "2026-06", "2", Some("2026-06-05"), None, "0");
    record(&repo, &id, "2026-06", "3", Some("2026-06-30"), None, "0");
    record(&repo, &id, "2026-07", "7", Some("2026-07-01"), None, "0");
    record(&repo, &id, "2026-08", "100", None, None, "0");
    let events = [
        event("second", "0.2", "2026-07-01", "2026-07-31"),
        event("first", "0.1", "2026-06-30", "2026-07-15"),
    ];
    let result = report(&repo, &events, "2026-07", "2026-07");
    let july = &result.months[0].funds[0];
    assert_eq!(july.holdings, "22");
    assert_eq!(july.amount.as_deref(), Some("5.9"));
    assert_eq!(july.eligible_shares, None);
    assert_eq!(july.events[0].id, "first");
    assert_eq!(july.events[0].eligible_shares.as_deref(), Some("15"));
    assert_eq!(july.events[0].amount.as_deref(), Some("1.5"));
    assert_eq!(july.events[1].eligible_shares.as_deref(), Some("22"));
    assert_eq!(july.events[1].amount.as_deref(), Some("4.4"));
}

#[test]
fn an_unknown_day_in_the_record_month_stays_ambiguous_until_the_user_supplies_it() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    let id = fund(&repo, "TEST11");
    record(&repo, &id, "2026-05", "10", None, None, "0");
    record(&repo, &id, "2026-06", "2", None, None, "0");
    let events = [event("july", "0.1", "2026-06-14", "2026-07-15")];
    let result = report(&repo, &events, "2026-07", "2026-07");
    let july = &result.months[0];
    assert_eq!(july.status, "missing");
    assert_eq!(july.total, None);
    assert_eq!(july.known_total, "0");
    assert_eq!(july.funds[0].status, "date_required");
    assert_eq!(july.funds[0].amount, None);
    assert_eq!(july.funds[0].events[0].eligible_shares, None);
    let purchase = repo.snapshot().unwrap().funds[0].entries[1].clone();
    assert_eq!(
        july.funds[0].events[0].missing_purchase_dates,
        vec![purchase.id.clone()]
    );
    let mut input = RecordInput {
        id: Some(purchase.id),
        fund_id: id,
        month: purchase.month,
        purchase_date: Some("2026-06-15".into()),
        cotas_added: purchase.cotas_added,
        price_per_cota: purchase.price_per_cota,
        dividend_per_cota: purchase.dividend_per_cota,
        dividend_total: purchase.dividend_total,
        notes: purchase.notes,
    };
    repo.save_monthly_record(input.clone()).unwrap();
    assert_eq!(
        report(&repo, &events, "2026-07", "2026-07").months[0]
            .total
            .as_deref(),
        Some("1")
    );
    input.purchase_date = Some("2026-06-14".into());
    repo.save_monthly_record(input).unwrap();
    assert_eq!(
        report(&repo, &events, "2026-07", "2026-07").months[0]
            .total
            .as_deref(),
        Some("1.2")
    );
    let later = [event("august", "0.1", "2026-07-31", "2026-08-15")];
    assert_eq!(
        report(&repo, &later, "2026-08", "2026-08").months[0]
            .total
            .as_deref(),
        Some("1.2")
    );
}

#[test]
fn manual_totals_including_zero_and_positive_monthly_rates_override_all_api_events() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    let id = fund(&repo, "TEST11");
    record(&repo, &id, "2026-05", "10", None, None, "0");
    record(&repo, &id, "2026-06", "2", None, None, "0");
    record(&repo, &id, "2026-07", "0", None, Some("0"), "0.1");
    record(&repo, &id, "2026-08", "0", None, Some("0.1234"), "0.1");
    record(&repo, &id, "2026-08", "0", None, None, "0.1");
    let events = [
        event("july", "999", "2026-06-30", "2026-07-15"),
        event("august", "999", "2026-07-31", "2026-08-15"),
    ];
    let result = report(&repo, &events, "2026-07", "2026-08");
    assert_eq!(result.months[0].total.as_deref(), Some("0"));
    assert_eq!(result.months[0].funds[0].status, "recorded");
    assert_eq!(result.months[1].total.as_deref(), Some("1.3234"));
    assert_eq!(result.months[1].recorded_amount, "1.3234");
    assert_eq!(result.months[1].api_amount, "0");
    assert_eq!(result.months[1].funds[0].manual_record_ids.len(), 2);
    assert!(result
        .months
        .iter()
        .all(|month| month.funds[0].events.is_empty()));
}

#[test]
fn missing_incomplete_and_future_events_never_copy_a_previous_dividend_or_create_zero() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    let id = fund(&repo, "TEST11");
    record(&repo, &id, "2026-05", "10", None, Some("1"), "0.1");
    let mut incomplete = event("incomplete", "0.1", "", "2026-07-15");
    let mut future = event("future", "0.1", "2026-10-30", "2026-11-15");
    let mut invalid = event("invalid", "bad", "2026-06-30", "2026-07-15");
    incomplete.source_url.clear();
    future.source_url = "https://example.test/proventos".into();
    invalid.payment_date = "2026-07-40".into();
    let result = report(&repo, &[incomplete, future, invalid], "2026-06", "2026-07");
    assert_eq!(result.ignored_event_count, 3);
    assert!(result
        .months
        .iter()
        .all(|point| point.total.is_none() && point.status == "missing"));
    assert!(result
        .months
        .iter()
        .all(|point| point.funds[0].amount.is_none()));
}

#[test]
fn multiple_events_are_rounded_individually_and_zero_eligible_shares_are_known_zero() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    let id = fund(&repo, "TEST11");
    record(&repo, &id, "2026-06", "1", Some("2026-06-30"), None, "0");
    let events = [
        event("before", "0.1", "2026-06-29", "2026-07-01"),
        event("one", "0.015", "2026-06-30", "2026-07-15"),
        event("two", "0.015", "2026-06-30", "2026-07-31"),
    ];
    let result = report(&repo, &events, "2026-05", "2026-07");
    assert_eq!(result.months[0].status, "not_held");
    assert_eq!(result.months[0].total.as_deref(), Some("0"));
    let july = &result.months[2].funds[0];
    assert_eq!(july.events[0].eligible_shares.as_deref(), Some("0"));
    assert_eq!(july.events[0].amount.as_deref(), Some("0"));
    assert_eq!(july.events[1].amount.as_deref(), Some("0.02"));
    assert_eq!(july.events[2].amount.as_deref(), Some("0.02"));
    assert_eq!(july.amount.as_deref(), Some("0.04"));
}

#[test]
fn partial_month_exposes_known_origin_totals_without_pretending_all_funds_are_complete() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    let first = fund(&repo, "TEST11");
    let second = fund(&repo, "OTRO11");
    let third = fund(&repo, "TRES11");
    for id in [&first, &second, &third] {
        record(&repo, id, "2026-05", "10", None, None, "0");
    }
    record(&repo, &second, "2026-07", "0", None, Some("0"), "0");
    let result = report(
        &repo,
        &[event("july", "0.1", "2026-06-30", "2026-07-15")],
        "2026-07",
        "2026-07",
    );
    let july = &result.months[0];
    assert_eq!(july.status, "partial");
    assert_eq!(july.total, None);
    assert_eq!(july.known_total, "1");
    assert_eq!(july.recorded_amount, "0");
    assert_eq!(july.api_amount, "1");
    assert_eq!(
        (
            july.recorded_fund_count,
            july.api_fund_count,
            july.missing_fund_count
        ),
        (1, 1, 1)
    );
}

#[test]
fn duplicate_events_are_not_added_twice_and_conflicting_id_values_are_not_guessed() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    let id = fund(&repo, "TEST11");
    record(&repo, &id, "2026-05", "10", None, None, "0");
    let one = event("july", "0.1", "2026-06-30", "2026-07-15");
    let result = report(&repo, &[one.clone(), one.clone()], "2026-07", "2026-07");
    assert_eq!(result.months[0].total.as_deref(), Some("1"));
    assert_eq!(result.months[0].funds[0].events.len(), 1);
    assert_eq!(result.ignored_event_count, 1);
    let mut conflict = one.clone();
    conflict.amount_per_share = "0.2".into();
    let result = report(&repo, &[one.clone(), conflict, one], "2026-07", "2026-07");
    assert_eq!(result.months[0].status, "missing");
    assert_eq!(result.months[0].total, None);
    assert_eq!(result.ignored_event_count, 3);
}

#[test]
fn invalid_ranges_invalid_purchase_dates_and_decimal_overflow_are_errors_without_mutation() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    let id = fund(&repo, "TEST11");
    record(&repo, &id, "2026-05", "10", None, None, "0");
    let snapshot = repo.snapshot().unwrap();
    for (start, end) in [
        ("2026-08", "2026-07"),
        ("2026-07", "2026-11"),
        ("1900-01", "2026-07"),
        ("2026-00", "2026-07"),
    ] {
        assert!(calculate_monthly_income(&snapshot, &[], start, end, today()).is_err());
    }
    let mut invalid = snapshot.clone();
    invalid.funds[0].entries[0].purchase_date = Some("2026-06-01".into());
    assert!(calculate_monthly_income(&invalid, &[], "2026-07", "2026-07", today()).is_err());
    let overflow = event(
        "large",
        "79228162514264337593543950335",
        "2026-06-30",
        "2026-07-15",
    );
    assert!(
        calculate_monthly_income(&snapshot, &[overflow], "2026-07", "2026-07", today()).is_err()
    );
    assert_eq!(
        serde_json::to_value(repo.snapshot().unwrap()).unwrap(),
        serde_json::to_value(snapshot).unwrap()
    );
}

#[test]
fn report_fields_serialize_as_camel_case_and_public_events_keep_source_dates() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    let id = fund(&repo, "TEST11");
    record(&repo, &id, "2026-05", "10", None, None, "0");
    let result = report(
        &repo,
        &[event("july", "0.1", "2026-06-30", "2026-07-15")],
        "2026-07",
        "2026-07",
    );
    let value = serde_json::to_value(&result).unwrap();
    assert_eq!(value["asOf"], "2026-10-20");
    assert_eq!(value["months"][0]["apiAmount"], "1");
    assert_eq!(value["months"][0]["funds"][0]["eligibleShares"], "10");
    assert_eq!(
        value["months"][0]["funds"][0]["events"][0]["amountPerShare"],
        "0.1"
    );
    assert_eq!(
        value["months"][0]["funds"][0]["events"][0]["sourceUrl"],
        "https://example.test/proventos"
    );
    assert!(value.get("start_month").is_none());
    assert_eq!(
        serde_json::from_value::<MonthlyIncomeReport>(value).unwrap(),
        result
    );
    let output: Value = serde_json::to_value(result).unwrap();
    assert!(output["months"][0]["funds"][0]["manualRecordIds"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn a_rate_without_any_owned_shares_does_not_create_a_recorded_payment_or_coverage() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    let id = fund(&repo, "TEST11");
    record(&repo, &id, "2026-07", "0", None, None, "0.1");
    let result = report(&repo, &[], "2026-07", "2026-07");
    assert_eq!(result.months[0].status, "not_held");
    assert_eq!(result.months[0].recorded_fund_count, 0);
    assert_eq!(result.months[0].api_fund_count, 0);
    assert!(result.months[0].funds[0].manual_record_ids.is_empty());
    assert_eq!(result.months[0].funds[0].status, "not_held");
    record(&repo, &id, "2026-08", "0", None, Some("0"), "0.1");
    let explicit = report(&repo, &[], "2026-08", "2026-08");
    assert_eq!(explicit.months[0].recorded_fund_count, 1);
    assert_eq!(explicit.months[0].funds[0].status, "recorded");
    assert_eq!(explicit.months[0].funds[0].amount.as_deref(), Some("0"));
}

#[test]
fn incomplete_or_conflicting_events_cannot_make_a_partial_event_total_look_complete() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    let id = fund(&repo, "TEST11");
    record(&repo, &id, "2026-05", "10", None, None, "0");
    let one = event("july-one", "0.1", "2026-06-30", "2026-07-15");
    let incomplete = event("july-two", "0.2", "", "2026-07-31");
    let result = report(&repo, &[one.clone(), incomplete], "2026-07", "2026-07");
    assert_eq!(result.months[0].total, None);
    assert_eq!(result.months[0].funds[0].status, "missing");
    let other = event("july-two", "0.2", "2026-07-01", "2026-07-31");
    let mut conflicting = other.clone();
    conflicting.amount_per_share = "0.3".into();
    let result = report(&repo, &[one, other, conflicting], "2026-07", "2026-07");
    assert_eq!(result.months[0].total, None);
    assert_eq!(result.months[0].funds[0].status, "missing");
}

#[test]
fn cross_source_mirrors_with_different_ids_and_decimal_formats_count_once_in_either_order() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    let id = fund(&repo, "TEST11");
    record(&repo, &id, "2026-05", "10", None, None, "0");
    let before = serde_json::to_value(repo.snapshot().unwrap()).unwrap();
    let export = repo.export_legacy_json().unwrap();
    let outbox = serde_json::to_value(repo.pending_changes().unwrap()).unwrap();
    let mut b3 = event("public-feed-id", "0.1000", "2026-06-30", "2026-07-15");
    b3.source = "B3 · eventos corporativos públicos de fundos".into();
    let mut document = event("fundosnet:document-id", "0.1", "2026-06-30", "2026-07-15");
    document.source = "Administrador · documento de rendimento".into();
    let mut mirror = document.clone();
    mirror.id = "another-cache-id".into();
    let events = [document, b3, mirror];
    let result = report(&repo, &events, "2026-07", "2026-07");
    let july = &result.months[0];
    assert_eq!(july.total.as_deref(), Some("1"));
    assert_eq!(july.api_amount, "1");
    assert_eq!(july.funds[0].events.len(), 1);
    assert_eq!(july.funds[0].events[0].id, "public-feed-id");
    assert_eq!(july.funds[0].events[0].amount_per_share, "0.1");
    assert_eq!(result.ignored_event_count, 2);
    let mut reverse = events.to_vec();
    reverse.reverse();
    assert_eq!(report(&repo, &reverse, "2026-07", "2026-07"), result);
    assert_eq!(
        serde_json::to_value(repo.snapshot().unwrap()).unwrap(),
        before
    );
    assert_eq!(repo.export_legacy_json().unwrap(), export);
    assert_eq!(
        serde_json::to_value(repo.pending_changes().unwrap()).unwrap(),
        outbox
    );
}

#[test]
fn different_rates_for_the_same_payout_are_unresolved_even_across_different_sources_or_ids() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    let id = fund(&repo, "TEST11");
    record(&repo, &id, "2026-05", "10", None, None, "0");
    let one = event("one", "0.1", "2026-06-30", "2026-07-15");
    for source in ["fixture", "another-source"] {
        let mut two = event("different-id", "0.2", "2026-06-30", "2026-07-15");
        two.source = source.into();
        let result = report(&repo, &[one.clone(), two], "2026-07", "2026-07");
        assert_eq!(result.months[0].total, None);
        assert_eq!(result.months[0].funds[0].status, "missing");
        assert_eq!(result.ignored_event_count, 2);
    }
    record(&repo, &id, "2026-07", "0", None, Some("0"), "0");
    let mut two = event("different-id", "0.2", "2026-06-30", "2026-07-15");
    two.source = "another-source".into();
    let manual = report(&repo, &[one, two], "2026-07", "2026-07");
    assert_eq!(manual.months[0].total.as_deref(), Some("0"));
    assert_eq!(manual.months[0].funds[0].status, "recorded");
    assert_eq!(manual.months[0].api_amount, "0");
}

#[test]
fn cross_source_record_and_payment_date_revisions_do_not_turn_into_extra_income() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    let id = fund(&repo, "TEST11");
    record(&repo, &id, "2026-05", "10", None, None, "0");
    let one = event("feed-id", "0.1", "2026-06-29", "2026-07-15");
    let mut record_revision = event("document-id", "0.1", "2026-06-30", "2026-07-15");
    record_revision.source = "another-source".into();
    let result = report(&repo, &[one.clone(), record_revision], "2026-07", "2026-07");
    assert_eq!(result.months[0].total, None);
    assert_eq!(result.ignored_event_count, 2);
    let mut payment_revision = event("document-id", "0.1", "2026-06-29", "2026-08-15");
    payment_revision.source = "another-source".into();
    let result = report(&repo, &[one, payment_revision], "2026-07", "2026-08");
    assert!(result.months.iter().all(|month| month.total.is_none()));
    assert_eq!(result.ignored_event_count, 2);
}

#[test]
fn same_source_distinct_dated_distributions_remain_distinct_with_a_partial_source_mirror() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    let id = fund(&repo, "TEST11");
    record(&repo, &id, "2026-05", "10", None, None, "0");
    let one = event("ordinary", "0.1", "2026-06-29", "2026-07-15");
    let two = event("extraordinary", "0.2", "2026-06-30", "2026-07-15");
    let mut mirror = one.clone();
    mirror.source = "another-source".into();
    mirror.id = "document-ordinary".into();
    let result = report(&repo, &[one, two, mirror], "2026-07", "2026-07");
    assert_eq!(result.months[0].total.as_deref(), Some("3"));
    assert_eq!(result.months[0].funds[0].events.len(), 2);
    assert_eq!(result.ignored_event_count, 1);
}

#[test]
fn same_source_installments_are_not_a_cross_source_payment_date_revision() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    let id = fund(&repo, "TEST11");
    record(&repo, &id, "2026-05", "10", None, None, "0");
    let one = event("first-installment", "0.1", "2026-06-30", "2026-07-15");
    let two = event("second-installment", "0.2", "2026-06-30", "2026-08-15");
    let mut mirror = one.clone();
    mirror.source = "another-source".into();
    mirror.id = "document-first-installment".into();
    let result = report(&repo, &[one, two, mirror], "2026-07", "2026-08");
    assert_eq!(result.months[0].total.as_deref(), Some("1"));
    assert_eq!(result.months[1].total.as_deref(), Some("2"));
    assert_eq!(result.ignored_event_count, 1);
}

#[test]
fn a_literal_calendar_month_end_resolves_undated_purchase_eligibility_without_changing_the_record()
{
    for (month, cutoff, payment) in [
        ("2024-02", "2024-02-29", "2024-03-15"),
        ("2025-12", "2025-12-31", "2026-01-15"),
        ("2026-06", "2026-06-30", "2026-07-15"),
    ] {
        let dir = TempDir::new().unwrap();
        let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
        let id = fund(&repo, "TEST11");
        record(&repo, &id, month, "12", None, None, "0");
        let before = serde_json::to_value(repo.snapshot().unwrap()).unwrap();
        let end = &payment[..7];
        let result = report(
            &repo,
            &[event("calendar-end", "0.1", cutoff, payment)],
            end,
            end,
        );
        assert_eq!(result.months[0].total.as_deref(), Some("1.2"));
        assert_eq!(
            result.months[0].funds[0].eligible_shares.as_deref(),
            Some("12")
        );
        assert!(result.months[0].funds[0].events[0]
            .missing_purchase_dates
            .is_empty());
        assert_eq!(
            serde_json::to_value(repo.snapshot().unwrap()).unwrap(),
            before
        );
        let earlier = NaiveDate::parse_from_str(cutoff, "%Y-%m-%d")
            .unwrap()
            .pred_opt()
            .unwrap()
            .to_string();
        let ambiguous = report(
            &repo,
            &[event("earlier", "0.1", &earlier, payment)],
            end,
            end,
        );
        assert_eq!(ambiguous.months[0].total, None);
        assert_eq!(ambiguous.months[0].funds[0].status, "date_required");
    }
}

#[test]
fn weekends_and_verified_historical_equity_closures_prove_eligibility_without_inventing_purchase_dates(
) {
    for (prior, month, cutoff, payment, has_closure_source) in [
        ("2023-08", "2023-09", "2023-09-29", "2023-10-06", false),
        ("2024-02", "2024-03", "2024-03-28", "2024-04-05", true),
        ("2024-11", "2024-12", "2024-12-30", "2025-01-14", true),
    ] {
        let dir = TempDir::new().unwrap();
        let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
        let id = fund(&repo, "TEST11");
        record(&repo, &id, prior, "10", None, None, "0");
        record(&repo, &id, month, "2", None, None, "0");
        let before = serde_json::to_value(repo.snapshot().unwrap()).unwrap();
        let exported = repo.export_legacy_json().unwrap();
        let outbox = serde_json::to_value(repo.pending_changes().unwrap()).unwrap();
        let paid_month = &payment[..7];
        let result = report(
            &repo,
            &[event("dated-public-payout", "0.1", cutoff, payment)],
            paid_month,
            paid_month,
        );
        let fund = &result.months[0].funds[0];
        assert_eq!(fund.amount.as_deref(), Some("1.2"));
        assert_eq!(fund.eligible_shares.as_deref(), Some("12"));
        assert!(fund.events[0].missing_purchase_dates.is_empty());
        assert!(fund.events[0]
            .reasons
            .iter()
            .any(|reason| reason.contains("A data de cada compra foi preservada")));
        assert_eq!(
            fund.events[0]
                .reasons
                .iter()
                .any(|reason| reason.contains("https://www.b3.com.br/")
                    && reason.contains("calendario-de-negociacao/feriados/")),
            has_closure_source
        );
        assert_eq!(
            serde_json::to_value(repo.snapshot().unwrap()).unwrap(),
            before
        );
        assert_eq!(repo.export_legacy_json().unwrap(), exported);
        assert_eq!(
            serde_json::to_value(repo.pending_changes().unwrap()).unwrap(),
            outbox
        );
        assert!(repo.snapshot().unwrap().funds[0]
            .entries
            .iter()
            .all(|entry| entry.purchase_date.is_none()));
        // An earlier cutoff leaves an actual open weekday before month-end.
        let earlier = NaiveDate::parse_from_str(cutoff, "%Y-%m-%d")
            .unwrap()
            .pred_opt()
            .unwrap()
            .to_string();
        let ambiguous = report(
            &repo,
            &[event("earlier-public-payout", "0.1", &earlier, payment)],
            paid_month,
            paid_month,
        );
        assert_eq!(ambiguous.months[0].total, None);
        assert_eq!(ambiguous.months[0].funds[0].status, "date_required");
    }
}

#[test]
fn a_reported_later_purchase_day_overrides_the_month_bound_and_unverified_years_stay_ambiguous() {
    for (month, cutoff, payment) in [
        ("2023-09", "2023-09-29", "2023-10-06"),
        ("2024-03", "2024-03-28", "2024-04-05"),
        ("2024-12", "2024-12-30", "2025-01-14"),
    ] {
        let dir = TempDir::new().unwrap();
        let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
        let id = fund(&repo, "TEST11");
        record(&repo, &id, month, "10", None, None, "0");
        let later = NaiveDate::parse_from_str(cutoff, "%Y-%m-%d")
            .unwrap()
            .succ_opt()
            .unwrap()
            .to_string();
        record(&repo, &id, month, "2", Some(&later), None, "0");
        let paid_month = &payment[..7];
        let result = report(
            &repo,
            &[event("dated-public-payout", "0.1", cutoff, payment)],
            paid_month,
            paid_month,
        );
        let fund = &result.months[0].funds[0];
        assert_eq!(fund.holdings, "12");
        assert_eq!(fund.eligible_shares.as_deref(), Some("10"));
        assert_eq!(fund.amount.as_deref(), Some("1"));
        assert_eq!(
            repo.snapshot().unwrap().funds[0].entries[1]
                .purchase_date
                .as_deref(),
            Some(later.as_str())
        );
    }
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    let id = fund(&repo, "TEST11");
    record(&repo, &id, "2025-12", "10", None, None, "0");
    let result = report(
        &repo,
        &[event(
            "no-calendar-extrapolation",
            "0.1",
            "2025-12-30",
            "2026-01-14",
        )],
        "2026-01",
        "2026-01",
    );
    assert_eq!(result.months[0].total, None);
    assert_eq!(result.months[0].funds[0].status, "date_required");
}
