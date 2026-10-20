use ivestfii_core::{CashPocketInput, CashSimulationInput, Repository};
use serde_json::{json, Value};
use std::fs;
use tempfile::TempDir;

fn pocket(balance: &str, percentage: &str, rate: Option<&str>) -> CashPocketInput {
    CashPocketInput {
        id: None,
        name: "Reserva de emergência".into(),
        institution: "Instituição de teste".into(),
        balance: balance.into(),
        total_deposits: None,
        total_withdrawals: None,
        balance_date: None,
        clear_reconciliation: false,
        cdi_percent: percentage.into(),
        annual_cdi_rate: rate.map(str::to_string),
        rate_date: rate.map(|_| "2026-10-16".into()),
        rate_source: None,
        rate_source_url: None,
        rate_fetched_at: None,
        notes: "Saldo conferido manualmente".into(),
    }
}

fn simulate(repo: &Repository, id: &str, days: u32) -> ivestfii_core::CashSimulation {
    repo.simulate_cash_pocket(CashSimulationInput {
        pocket_id: id.into(),
        business_days: days,
    })
    .unwrap()
}

#[test]
fn records_exact_manual_balances_and_updates_without_adding_interest() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("portfolio.sqlite");
    let repo = Repository::open(&path).unwrap();
    let first = repo
        .upsert_cash_pocket(pocket("1234.56", "120.0000", Some("10.0")))
        .unwrap();
    assert_eq!(first.balance, "1234.56");
    assert_eq!(first.cdi_percent, "120");
    assert_eq!(first.annual_cdi_rate.as_deref(), Some("10"));
    assert_eq!(repo.snapshot().unwrap().cash_balance, "1234.56");
    assert_eq!(repo.snapshot().unwrap().funds.len(), 0);
    assert_eq!(repo.pending_change_count().unwrap(), 1);
    let mut updated = pocket("1300.01", "100", Some("9.5"));
    updated.id = Some(first.id.clone());
    updated.name = "Reserva atualizada".into();
    let second = repo.upsert_cash_pocket(updated).unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(first.created_at, second.created_at);
    assert_eq!(repo.cash_pockets().unwrap(), vec![second.clone()]);
    assert_eq!(repo.snapshot().unwrap().cash_balance, "1300.01");
    let changes = repo.pending_changes().unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[1].entity_type, "cashPocket");
    assert_eq!(changes[1].payload["revision"], 2);
    assert_eq!(changes[1].payload["balance"], "1300.01");
    drop(repo);
    assert_eq!(
        Repository::open(path).unwrap().cash_pockets().unwrap(),
        vec![second]
    );
}

#[test]
fn cdi_percentage_applies_to_daily_equivalent_and_projection_is_read_only() {
    let directory = TempDir::new().unwrap();
    let repo = Repository::open(directory.path().join("portfolio.sqlite")).unwrap();
    let first = repo
        .upsert_cash_pocket(pocket("10000", "100", Some("10")))
        .unwrap();
    let boosted = repo
        .upsert_cash_pocket(pocket("10000", "120", Some("10")))
        .unwrap();
    let before = repo.export_legacy_json().unwrap();
    let pending = repo.pending_change_count().unwrap();
    let standard = simulate(&repo, &first.id, 252);
    assert_eq!(standard.ending_balance, "11000");
    assert_eq!(standard.gross_earnings, "1000");
    // Independently calculated to 60 decimal places: daily equivalent of 10%
    // annual, multiplied by 1.2, compounded over 252 business days.
    let result = simulate(&repo, &boosted.id, 252);
    assert_eq!(result.ending_balance, "11211.65");
    assert_eq!(result.gross_earnings, "1211.65");
    assert_ne!(result.ending_balance, "11200");
    assert_eq!(result.business_days, 252);
    assert_eq!(result.rate_date.as_deref(), Some("2026-10-16"));
    assert!(result
        .assumptions
        .iter()
        .any(|value| value.contains("Valores brutos")));
    assert_eq!(repo.export_legacy_json().unwrap(), before);
    assert_eq!(repo.pending_change_count().unwrap(), pending);
}

#[test]
fn zero_period_zero_rate_and_fractional_month_keep_cent_precision() {
    let directory = TempDir::new().unwrap();
    let repo = Repository::open(directory.path().join("portfolio.sqlite")).unwrap();
    let saved = repo
        .upsert_cash_pocket(pocket("1234.56", "120", Some("10")))
        .unwrap();
    let zero = simulate(&repo, &saved.id, 0);
    assert_eq!(zero.starting_balance, "1234.56");
    assert_eq!(zero.ending_balance, "1234.56");
    assert_eq!(zero.gross_earnings, "0");
    let month = simulate(&repo, &saved.id, 21);
    assert_eq!(month.ending_balance, "1246.38");
    assert_eq!(month.gross_earnings, "11.82");
    let unchanged = repo
        .upsert_cash_pocket(pocket("1234.56", "120", Some("0")))
        .unwrap();
    assert_eq!(
        simulate(&repo, &unchanged.id, 2520).ending_balance,
        "1234.56"
    );
    let empty = repo
        .upsert_cash_pocket(pocket("0", "120", Some("10")))
        .unwrap();
    assert_eq!(simulate(&repo, &empty.id, 252).gross_earnings, "0");
}

#[test]
fn an_unquoted_cash_pocket_can_be_saved_but_does_not_invent_a_reference_rate() {
    let directory = TempDir::new().unwrap();
    let repo = Repository::open(directory.path().join("portfolio.sqlite")).unwrap();
    let saved = repo.upsert_cash_pocket(pocket("500", "120", None)).unwrap();
    assert_eq!(saved.annual_cdi_rate, None);
    assert_eq!(saved.rate_date, None);
    let error = repo
        .simulate_cash_pocket(CashSimulationInput {
            pocket_id: saved.id,
            business_days: 21,
        })
        .unwrap_err();
    assert!(error.to_string().contains("taxa anual"));
    assert_eq!(repo.snapshot().unwrap().cash_balance, "500");
}

#[test]
fn rejects_invalid_values_and_date_pairs_without_any_mutation() {
    let directory = TempDir::new().unwrap();
    let repo = Repository::open(directory.path().join("portfolio.sqlite")).unwrap();
    for input in [
        pocket("-0.01", "120", Some("10")),
        pocket("10.001", "120", Some("10")),
        pocket("10", "0", Some("10")),
        pocket("10", "-120", Some("10")),
        pocket("10", "1000.0001", Some("10")),
        pocket("10", "120.00001", Some("10")),
        pocket("10", "120", Some("-1")),
        pocket("10", "120", Some("100.01")),
        pocket("10", "120", Some("10.000000001")),
        pocket("NaN", "120", Some("10")),
    ] {
        assert!(repo.upsert_cash_pocket(input).is_err());
    }
    let mut invalid_date = pocket("10", "120", Some("10"));
    invalid_date.rate_date = Some("2026-02-30".into());
    assert!(repo.upsert_cash_pocket(invalid_date).is_err());
    let mut missing_date = pocket("10", "120", Some("10"));
    missing_date.rate_date = None;
    assert!(repo.upsert_cash_pocket(missing_date).is_err());
    let mut missing_rate = pocket("10", "120", None);
    missing_rate.rate_date = Some("2026-10-16".into());
    assert!(repo.upsert_cash_pocket(missing_rate).is_err());
    let mut missing_name = pocket("10", "120", None);
    missing_name.name = " ".into();
    assert!(repo.upsert_cash_pocket(missing_name).is_err());
    assert!(repo.cash_pockets().unwrap().is_empty());
    assert_eq!(repo.pending_change_count().unwrap(), 0);
}

#[test]
fn pockets_are_scoped_to_the_wallet_database_and_unknown_ids_cannot_be_upserted() {
    let directory = TempDir::new().unwrap();
    let original = Repository::open(directory.path().join("original.sqlite")).unwrap();
    let nubank = Repository::open(directory.path().join("nubank.sqlite")).unwrap();
    let saved = original
        .upsert_cash_pocket(pocket("50.01", "120", None))
        .unwrap();
    let mut foreign = pocket("80.01", "100", None);
    foreign.id = Some(saved.id.clone());
    assert!(nubank.upsert_cash_pocket(foreign).is_err());
    assert!(nubank
        .simulate_cash_pocket(CashSimulationInput {
            pocket_id: saved.id,
            business_days: 21,
        })
        .is_err());
    let other = nubank
        .upsert_cash_pocket(pocket("80.01", "100", None))
        .unwrap();
    assert_ne!(original.cash_pockets().unwrap()[0].id, other.id);
    assert_eq!(original.snapshot().unwrap().cash_balance, "50.01");
    assert_eq!(nubank.snapshot().unwrap().cash_balance, "80.01");
    assert_eq!(nubank.pending_change_count().unwrap(), 1);
}

#[test]
fn aggregation_and_projection_overflow_roll_back_instead_of_wrapping() {
    let directory = TempDir::new().unwrap();
    let repo = Repository::open(directory.path().join("portfolio.sqlite")).unwrap();
    let maximum = "79228162514264337593543950335";
    let saved = repo
        .upsert_cash_pocket(pocket(maximum, "100", Some("10")))
        .unwrap();
    assert!(repo.upsert_cash_pocket(pocket("1", "100", None)).is_err());
    assert_eq!(repo.cash_pockets().unwrap().len(), 1);
    assert_eq!(repo.pending_change_count().unwrap(), 1);
    assert!(repo
        .simulate_cash_pocket(CashSimulationInput {
            pocket_id: saved.id.clone(),
            business_days: 1,
        })
        .is_err());
    assert!(repo
        .simulate_cash_pocket(CashSimulationInput {
            pocket_id: saved.id,
            business_days: 2521,
        })
        .is_err());
    assert_eq!(repo.snapshot().unwrap().cash_balance, maximum);
    assert_eq!(repo.pending_change_count().unwrap(), 1);
}

#[test]
fn exported_cash_only_portfolio_roundtrips_and_imports_idempotently() {
    let directory = TempDir::new().unwrap();
    let original = Repository::open(directory.path().join("original.sqlite")).unwrap();
    let saved = original
        .upsert_cash_pocket(pocket("1234.56", "120", Some("10")))
        .unwrap();
    let export = original.export_legacy_json().unwrap();
    let json: Value = serde_json::from_str(&export).unwrap();
    assert_eq!(json["cash_pockets"][0]["id"], saved.id);
    let backup = directory.path().join("backup.json");
    fs::write(&backup, &export).unwrap();
    let restored = Repository::open(directory.path().join("restored.sqlite")).unwrap();
    let receipt = restored.import_legacy_file(&backup).unwrap();
    assert_eq!(receipt.fund_count, 0);
    assert_eq!(restored.cash_pockets().unwrap(), vec![saved]);
    assert_eq!(restored.snapshot().unwrap().cash_balance, "1234.56");
    assert_eq!(restored.pending_change_count().unwrap(), 1);
    assert!(
        restored
            .import_legacy_file(&backup)
            .unwrap()
            .already_imported
    );
    assert_eq!(restored.pending_change_count().unwrap(), 1);
    assert_eq!(fs::read_to_string(&backup).unwrap(), export);
    // A second, different source must not silently merge into a cash-only DB.
    let different = directory.path().join("different.json");
    fs::write(&different, "{\"fiis\":[]}").unwrap();
    assert!(restored.import_legacy_file(different).is_err());
    assert_eq!(restored.snapshot().unwrap().cash_balance, "1234.56");
}

#[test]
fn corrupted_cash_backups_are_rejected_atomically() {
    let directory = TempDir::new().unwrap();
    let repo = Repository::open(directory.path().join("source.sqlite")).unwrap();
    repo.upsert_cash_pocket(pocket("1234.56", "120", Some("10")))
        .unwrap();
    let valid: Value = serde_json::from_str(&repo.export_legacy_json().unwrap()).unwrap();
    let mut cases = Vec::new();
    let mut duplicate = valid.clone();
    duplicate["cash_pockets"]
        .as_array_mut()
        .unwrap()
        .push(valid["cash_pockets"][0].clone());
    cases.push(duplicate);
    for (field, value) in [
        ("balance", json!("-1")),
        ("cdiPercent", json!("0")),
        ("createdAt", json!("invalid")),
        ("updatedAt", json!("2000-01-01T00:00:00Z")),
        ("id", json!("non-uuid")),
    ] {
        let mut invalid = valid.clone();
        invalid["cash_pockets"][0][field] = value;
        cases.push(invalid);
    }
    for (index, invalid) in cases.iter().enumerate() {
        let path = directory.path().join(format!("invalid-{index}.json"));
        fs::write(&path, serde_json::to_string(invalid).unwrap()).unwrap();
        let restored =
            Repository::open(directory.path().join(format!("invalid-{index}.sqlite"))).unwrap();
        assert!(restored.import_legacy_file(path).is_err());
        assert!(restored.snapshot().unwrap().cash_pockets.is_empty());
        assert_eq!(restored.pending_change_count().unwrap(), 0);
    }
}
