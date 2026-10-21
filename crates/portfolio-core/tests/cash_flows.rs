use ivestfii_core::{CashFlow, CashPocketInput, Repository};
use rusqlite::Connection;
use serde_json::{json, Value};
use tempfile::TempDir;

fn input() -> CashPocketInput {
    serde_json::from_value(json!({
        "name":"Reserva sintética", "institution":"Banco de teste", "balance":"113.50",
        "totalDeposits":"120.00", "totalWithdrawals":"10.00", "balanceDate":"2020-02-03",
        "cashFlows":[
            {"date":"2020-01-02","kind":"deposit","amount":"100.00"},
            {"date":"2020-01-08","kind":"deposit","amount":"20.00"},
            {"date":"2020-01-20","kind":"withdrawal","amount":"10.00"}
        ], "cdiPercent":"120", "notes":"Valores fictícios"
    }))
    .unwrap()
}

fn state(repo: &Repository) -> (String, Value) {
    (
        repo.export_legacy_json().unwrap(),
        serde_json::to_value(repo.pending_changes().unwrap()).unwrap(),
    )
}

#[test]
fn movements_survive_reopen_backup_restore_and_outbox_without_becoming_income_records() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("source.sqlite");
    let repo = Repository::open(&path).unwrap();
    let saved = repo.upsert_cash_pocket(input()).unwrap();
    assert_eq!(saved.cash_flows.as_ref().unwrap()[0].amount, "100");
    assert_eq!(repo.snapshot().unwrap().record_count, 0);
    let before = state(&repo);
    assert_eq!(before.1[0]["payload"]["cashFlows"][2]["kind"], "withdrawal");
    drop(repo);
    let reopened = Repository::open_existing(path).unwrap();
    assert_eq!(reopened.cash_pockets().unwrap(), vec![saved.clone()]);
    assert_eq!(state(&reopened), before);
    let backup = directory.path().join("backup.json");
    std::fs::write(&backup, &before.0).unwrap();
    let restored = Repository::open(directory.path().join("restored.sqlite")).unwrap();
    restored.import_legacy_file(&backup).unwrap();
    assert_eq!(restored.cash_pockets().unwrap(), vec![saved]);
    assert_eq!(restored.export_legacy_json().unwrap(), before.0);
    assert_eq!(restored.snapshot().unwrap().record_count, 0);
}

#[test]
fn old_clients_preserve_movement_dates_and_changed_totals_must_reconcile_atomically() {
    let directory = TempDir::new().unwrap();
    let repo = Repository::open(directory.path().join("portfolio.sqlite")).unwrap();
    let saved = repo.upsert_cash_pocket(input()).unwrap();
    let mut legacy = serde_json::to_value(input()).unwrap();
    legacy["id"] = json!(saved.id);
    legacy["name"] = json!("Reserva editada");
    for key in [
        "cashFlows",
        "totalDeposits",
        "totalWithdrawals",
        "balanceDate",
    ] {
        legacy.as_object_mut().unwrap().remove(key);
    }
    for nulls in [false, true] {
        if nulls {
            legacy["cashFlows"] = Value::Null;
        }
        let edited = repo
            .upsert_cash_pocket(serde_json::from_value(legacy.clone()).unwrap())
            .unwrap();
        assert_eq!(edited.cash_flows, saved.cash_flows);
        assert_eq!(edited.total_deposits, saved.total_deposits);
    }
    let before = state(&repo);
    let mut mismatched = input();
    mismatched.id = Some(saved.id);
    mismatched.cash_flows = None;
    mismatched.total_deposits = Some("200".into());
    assert!(repo.upsert_cash_pocket(mismatched).is_err());
    assert_eq!(state(&repo), before);
}

#[test]
fn invalid_movement_dates_amounts_kinds_and_totals_are_rejected_without_writes() {
    let directory = TempDir::new().unwrap();
    let repo = Repository::open(directory.path().join("portfolio.sqlite")).unwrap();
    let before = state(&repo);
    let mut invalid = Vec::new();
    for date in [
        "2020-01-32",
        "2020-1-02",
        "0000-01-01",
        "2020-01-02 ",
        "2020-02-04",
        "9999-01-01",
    ] {
        let mut value = input();
        value.cash_flows.as_mut().unwrap()[0].date = date.into();
        invalid.push(value);
    }
    for amount in [
        "0",
        "-1",
        "1.001",
        "NaN",
        "",
        "79228162514264337593543950336",
    ] {
        let mut value = input();
        value.cash_flows.as_mut().unwrap()[0].amount = amount.into();
        invalid.push(value);
    }
    for kind in ["income", "Deposit", ""] {
        let mut value = input();
        value.cash_flows.as_mut().unwrap()[0].kind = kind.into();
        invalid.push(value);
    }
    let mut missing = input();
    missing.total_deposits = None;
    missing.total_withdrawals = None;
    missing.balance_date = None;
    invalid.push(missing);
    let mut empty = input();
    empty.cash_flows = Some(vec![]);
    invalid.push(empty);
    let mut mismatch = input();
    mismatch.cash_flows.as_mut().unwrap().remove(2);
    invalid.push(mismatch);
    for value in invalid {
        assert!(repo.upsert_cash_pocket(value).is_err());
        assert_eq!(state(&repo), before);
    }
}

#[test]
fn zero_total_movements_can_be_explicitly_empty_and_clear_removes_saved_dates() {
    let directory = TempDir::new().unwrap();
    let repo = Repository::open(directory.path().join("portfolio.sqlite")).unwrap();
    let saved = repo.upsert_cash_pocket(input()).unwrap();
    let mut zero = input();
    zero.id = Some(saved.id.clone());
    zero.total_deposits = Some("0".into());
    zero.total_withdrawals = Some("0".into());
    zero.cash_flows = Some(vec![]);
    assert_eq!(
        repo.upsert_cash_pocket(zero).unwrap().cash_flows,
        Some(vec![])
    );
    let mut clear = input();
    clear.id = Some(saved.id);
    clear.clear_reconciliation = true;
    clear.total_deposits = None;
    clear.total_withdrawals = None;
    clear.balance_date = None;
    clear.cash_flows = None;
    let cleared = repo.upsert_cash_pocket(clear).unwrap();
    assert_eq!(cleared.cash_flows, None);
    assert_eq!(cleared.total_deposits, None);
    assert_eq!(cleared.balance, "113.5");
    let queue = repo.pending_changes().unwrap();
    assert_eq!(queue.last().unwrap().payload["cashFlows"], Value::Null);
}

#[test]
fn schema_seven_upgrade_adds_unknown_dates_without_rewriting_old_financial_rows_or_outbox() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("old.sqlite");
    let repo = Repository::open(&path).unwrap();
    let mut legacy = input();
    legacy.cash_flows = None;
    let saved = repo.upsert_cash_pocket(legacy).unwrap();
    let before = state(&repo);
    drop(repo);
    let db = Connection::open(&path).unwrap();
    db.execute_batch("ALTER TABLE cash_pockets DROP COLUMN cash_flows; PRAGMA user_version=7;")
        .unwrap();
    drop(db);
    let upgraded = Repository::open_existing(&path).unwrap();
    assert_eq!(upgraded.cash_pockets().unwrap(), vec![saved]);
    assert_eq!(state(&upgraded), before);
    drop(upgraded);
    let db = Connection::open(path).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
            .unwrap(),
        10
    );
    assert_eq!(
        db.query_row("SELECT cash_flows FROM cash_pockets", [], |row| row
            .get::<_, Option<String>>(0))
            .unwrap(),
        None
    );
}

#[test]
fn invalid_dated_movements_in_backup_do_not_import_any_financial_data() {
    let directory = TempDir::new().unwrap();
    let source = Repository::open(directory.path().join("source.sqlite")).unwrap();
    source.upsert_cash_pocket(input()).unwrap();
    let mut backup: Value = serde_json::from_str(&source.export_legacy_json().unwrap()).unwrap();
    backup["cash_pockets"][0]["cashFlows"][0]["amount"] = json!("99.99");
    let path = directory.path().join("invalid.json");
    std::fs::write(&path, serde_json::to_vec(&backup).unwrap()).unwrap();
    let restored = Repository::open(directory.path().join("restored.sqlite")).unwrap();
    assert!(restored.import_legacy_file(path).is_err());
    assert!(restored.cash_pockets().unwrap().is_empty());
    assert_eq!(restored.pending_change_count().unwrap(), 0);
}

#[test]
fn withdrawals_can_include_earnings_beyond_original_principal() {
    let directory = TempDir::new().unwrap();
    let repo = Repository::open(directory.path().join("portfolio.sqlite")).unwrap();
    let mut value = input();
    value.balance = "0".into();
    value.total_deposits = Some("100".into());
    value.total_withdrawals = Some("110".into());
    value.cash_flows = Some(vec![
        CashFlow {
            date: "2020-01-02".into(),
            kind: "deposit".into(),
            amount: "100".into(),
        },
        CashFlow {
            date: "2020-02-03".into(),
            kind: "withdrawal".into(),
            amount: "110".into(),
        },
    ]);
    assert!(repo.upsert_cash_pocket(value).is_ok());
}
