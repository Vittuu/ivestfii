use chrono::{Days, FixedOffset, Utc};
use ivestfii_core::{CashPocketInput, FundInput, RecordInput, Repository};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::fs;
use tempfile::TempDir;

fn pocket() -> CashPocketInput {
    CashPocketInput {
        id: None,
        name: "Reserva sintética".into(),
        institution: "Instituição de teste".into(),
        balance: "1200.50".into(),
        total_deposits: None,
        total_withdrawals: None,
        balance_date: None,
        clear_reconciliation: false,
        cdi_percent: "120".into(),
        annual_cdi_rate: Some("10".into()),
        rate_date: Some("2020-01-02".into()),
        rate_source: None,
        rate_source_url: None,
        rate_fetched_at: None,
        notes: "Dados fictícios; saldo informado manualmente".into(),
    }
}

fn reconciled() -> CashPocketInput {
    CashPocketInput {
        total_deposits: Some("1100.00".into()),
        total_withdrawals: Some("50.00".into()),
        balance_date: Some("2020-01-10".into()),
        ..pocket()
    }
}

#[test]
fn aggregate_deposits_overflow_rolls_back_the_new_pocket_and_outbox() {
    let directory = TempDir::new().unwrap();
    let repo = Repository::open(directory.path().join("portfolio.sqlite")).unwrap();
    let mut input = reconciled();
    input.balance = "0".into();
    input.total_deposits = Some("79228162514264337593543950335".into());
    input.total_withdrawals = Some("0".into());
    repo.upsert_cash_pocket(input.clone()).unwrap();
    let before = state(&repo);
    assert!(repo.upsert_cash_pocket(input).is_err());
    assert_eq!(before, state(&repo));
}

#[test]
fn year_zero_is_rejected_before_persisting_reconciliation() {
    let directory = TempDir::new().unwrap();
    let repo = Repository::open(directory.path().join("portfolio.sqlite")).unwrap();
    let mut input = reconciled();
    input.balance_date = Some("0000-01-01".into());
    let before = state(&repo);
    assert!(repo.upsert_cash_pocket(input).is_err());
    assert_eq!(before, state(&repo));
}

fn state(repo: &Repository) -> (String, Value) {
    (
        repo.export_legacy_json().unwrap(),
        serde_json::to_value(repo.pending_changes().unwrap()).unwrap(),
    )
}

#[test]
fn unknown_reconciliation_is_not_inferred_from_balance_and_zero_is_known() {
    let directory = TempDir::new().unwrap();
    let repo = Repository::open(directory.path().join("portfolio.sqlite")).unwrap();
    let unknown = repo.upsert_cash_pocket(pocket()).unwrap();
    assert_eq!(unknown.balance, "1200.5");
    assert_eq!(unknown.total_deposits, None);
    assert_eq!(unknown.total_withdrawals, None);
    assert_eq!(unknown.balance_date, None);
    let mut zero = reconciled();
    zero.balance = "0".into();
    zero.total_deposits = Some("0.00".into());
    zero.total_withdrawals = Some("0".into());
    let zero = repo.upsert_cash_pocket(zero).unwrap();
    assert_eq!(zero.total_deposits.as_deref(), Some("0"));
    assert_eq!(zero.total_withdrawals.as_deref(), Some("0"));
    assert_eq!(zero.balance_date.as_deref(), Some("2020-01-10"));
    let mut loss = reconciled();
    loss.balance = "1".into();
    loss.total_deposits = Some("100".into());
    loss.total_withdrawals = Some("0".into());
    assert!(repo.upsert_cash_pocket(loss).is_ok());
    assert_eq!(repo.snapshot().unwrap().cash_balance, "1201.5");
}

#[test]
fn omitted_or_null_group_from_old_client_preserves_reconciliation_and_identity() {
    let directory = TempDir::new().unwrap();
    let repo = Repository::open(directory.path().join("portfolio.sqlite")).unwrap();
    let saved = repo.upsert_cash_pocket(reconciled()).unwrap();
    let mut input = pocket();
    input.id = Some(saved.id.clone());
    input.name = "Nome editado".into();
    input.balance = "1300.01".into();
    let mut legacy = serde_json::to_value(input).unwrap();
    for key in [
        "totalDeposits",
        "totalWithdrawals",
        "balanceDate",
        "clearReconciliation",
    ] {
        legacy.as_object_mut().unwrap().remove(key);
    }
    for nulls in [false, true] {
        if nulls {
            for key in ["totalDeposits", "totalWithdrawals", "balanceDate"] {
                legacy[key] = Value::Null;
            }
        }
        let saved_again = repo
            .upsert_cash_pocket(serde_json::from_value(legacy.clone()).unwrap())
            .unwrap();
        assert_eq!(saved_again.id, saved.id);
        assert_eq!(saved_again.created_at, saved.created_at);
        assert_eq!(saved_again.balance, "1300.01");
        assert_eq!(saved_again.total_deposits, saved.total_deposits);
        assert_eq!(saved_again.total_withdrawals, saved.total_withdrawals);
        assert_eq!(saved_again.balance_date, saved.balance_date);
    }
    let changes = repo.pending_changes().unwrap();
    assert_eq!(changes[2].payload["revision"], 3);
    assert_eq!(changes[2].payload["totalDeposits"], "1100");
    assert_eq!(changes[2].payload["totalWithdrawals"], "50");
    assert_eq!(changes[2].payload["balanceDate"], "2020-01-10");
    assert!(changes[2].payload.get("clearReconciliation").is_none());
}

#[test]
fn explicit_clear_only_removes_reconciliation_and_conflicting_clear_is_atomic() {
    let directory = TempDir::new().unwrap();
    let repo = Repository::open(directory.path().join("portfolio.sqlite")).unwrap();
    let saved = repo.upsert_cash_pocket(reconciled()).unwrap();
    let before = state(&repo);
    let mut conflict = reconciled();
    conflict.id = Some(saved.id.clone());
    conflict.clear_reconciliation = true;
    assert!(repo.upsert_cash_pocket(conflict).is_err());
    assert_eq!(state(&repo), before);
    let mut clear = pocket();
    clear.id = Some(saved.id.clone());
    clear.clear_reconciliation = true;
    let cleared = repo.upsert_cash_pocket(clear).unwrap();
    assert_eq!(cleared.id, saved.id);
    assert_eq!(cleared.created_at, saved.created_at);
    assert_eq!(cleared.balance, saved.balance);
    assert_eq!(cleared.cdi_percent, saved.cdi_percent);
    assert_eq!(cleared.annual_cdi_rate, saved.annual_cdi_rate);
    assert_eq!(cleared.rate_date, saved.rate_date);
    assert_eq!(cleared.total_deposits, None);
    assert_eq!(cleared.total_withdrawals, None);
    assert_eq!(cleared.balance_date, None);
    let queue = repo.pending_changes().unwrap();
    for key in ["totalDeposits", "totalWithdrawals", "balanceDate"] {
        assert_eq!(queue[1].payload[key], Value::Null);
    }
}

#[test]
fn invalid_groups_money_and_effective_dates_never_change_row_or_outbox() {
    let directory = TempDir::new().unwrap();
    let repo = Repository::open(directory.path().join("portfolio.sqlite")).unwrap();
    let saved = repo.upsert_cash_pocket(reconciled()).unwrap();
    let before = state(&repo);
    let mut valid = reconciled();
    valid.id = Some(saved.id.clone());
    let mut cases = Vec::new();
    for mask in 1..7 {
        let mut partial = valid.clone();
        if mask & 1 == 0 {
            partial.total_deposits = None;
        }
        if mask & 2 == 0 {
            partial.total_withdrawals = None;
        }
        if mask & 4 == 0 {
            partial.balance_date = None;
        }
        cases.push(partial);
    }
    for amount in [
        "-0.01",
        "1.001",
        "NaN",
        "Infinity",
        "",
        "79228162514264337593543950336",
    ] {
        let mut deposits = valid.clone();
        deposits.total_deposits = Some(amount.into());
        cases.push(deposits);
        let mut withdrawals = valid.clone();
        withdrawals.total_withdrawals = Some(amount.into());
        cases.push(withdrawals);
    }
    let today = Utc::now()
        .with_timezone(&FixedOffset::west_opt(3 * 3600).unwrap())
        .date_naive();
    let future = today.checked_add_days(Days::new(1)).unwrap().to_string();
    for date in [
        "2020-1-10",
        "2020-02-30",
        "2020-01-10 ",
        "2020-01-10T12:00:00Z",
        "",
        future.as_str(),
    ] {
        let mut invalid = valid.clone();
        invalid.balance_date = Some(date.into());
        cases.push(invalid);
    }
    let mut overflow = valid.clone();
    overflow.total_withdrawals = Some("79228162514264337593543950335".into());
    cases.push(overflow);
    // Even when an old client omits reconciliation, the new balance may not
    // overflow the already-recorded result after those fields are preserved.
    let mut preserved_overflow = pocket();
    preserved_overflow.id = Some(saved.id);
    preserved_overflow.balance = "79228162514264337593543950335".into();
    cases.push(preserved_overflow);
    for input in cases {
        assert!(repo.upsert_cash_pocket(input).is_err());
        assert_eq!(state(&repo), before);
    }
}

#[test]
fn reconciliation_survives_reopen_backup_import_outbox_and_exact_decimal_values() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("source.sqlite");
    let repo = Repository::open(&path).unwrap();
    let first = repo.upsert_cash_pocket(reconciled()).unwrap();
    let second = repo.upsert_cash_pocket(pocket()).unwrap();
    let mut zero = reconciled();
    zero.balance = "0".into();
    zero.total_deposits = Some("0".into());
    zero.total_withdrawals = Some("0".into());
    let third = repo.upsert_cash_pocket(zero).unwrap();
    let before = state(&repo);
    drop(repo);
    let reopened = Repository::open_existing(&path).unwrap();
    assert_eq!(state(&reopened), before);
    let pockets = reopened.cash_pockets().unwrap();
    for pocket in [&first, &second, &third] {
        assert!(pockets.contains(pocket));
    }
    let backup = directory.path().join("export.json");
    fs::write(&backup, &before.0).unwrap();
    let restored = Repository::open(directory.path().join("restored.sqlite")).unwrap();
    let receipt = restored.import_legacy_file(&backup).unwrap();
    assert_eq!(restored.cash_pockets().unwrap(), pockets);
    assert_eq!(restored.export_legacy_json().unwrap(), before.0);
    assert_eq!(fs::read_to_string(&receipt.backup_path).unwrap(), before.0);
    assert_eq!(fs::read_to_string(&backup).unwrap(), before.0);
    assert_eq!(restored.pending_change_count().unwrap(), 3);
    for change in restored.pending_changes().unwrap() {
        assert!(change.payload.get("totalDeposits").is_some());
        assert!(change.payload.get("totalWithdrawals").is_some());
        assert!(change.payload.get("balanceDate").is_some());
    }
    assert!(
        restored
            .import_legacy_file(&backup)
            .unwrap()
            .already_imported
    );
    assert_eq!(restored.pending_change_count().unwrap(), 3);
}

#[test]
fn legacy_backup_without_group_restores_unknown_values_and_incomplete_group_is_atomic() {
    let directory = TempDir::new().unwrap();
    let source = Repository::open(directory.path().join("source.sqlite")).unwrap();
    let saved = source.upsert_cash_pocket(reconciled()).unwrap();
    let mut legacy: Value = serde_json::from_str(&source.export_legacy_json().unwrap()).unwrap();
    for key in ["totalDeposits", "totalWithdrawals", "balanceDate"] {
        legacy["cash_pockets"][0]
            .as_object_mut()
            .unwrap()
            .remove(key);
    }
    let path = directory.path().join("legacy.json");
    fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let restored = Repository::open(directory.path().join("restored.sqlite")).unwrap();
    restored.import_legacy_file(&path).unwrap();
    let pocket = restored.cash_pockets().unwrap().remove(0);
    assert_eq!(pocket.balance, saved.balance);
    assert_eq!(pocket.total_deposits, None);
    assert_eq!(pocket.total_withdrawals, None);
    assert_eq!(pocket.balance_date, None);
    for (index, (field, value)) in [
        ("totalDeposits", json!("1000")),
        ("totalWithdrawals", json!("0")),
        ("balanceDate", json!("2020-01-10")),
    ]
    .into_iter()
    .enumerate()
    {
        let mut invalid = legacy.clone();
        invalid["cash_pockets"][0][field] = value;
        let file = directory.path().join(format!("invalid-{index}.json"));
        let bytes = serde_json::to_vec(&invalid).unwrap();
        fs::write(&file, &bytes).unwrap();
        let target =
            Repository::open(directory.path().join(format!("invalid-{index}.sqlite"))).unwrap();
        assert!(target.import_legacy_file(&file).is_err());
        assert!(target.cash_pockets().unwrap().is_empty());
        assert_eq!(target.pending_change_count().unwrap(), 0);
        assert_eq!(fs::read(file).unwrap(), bytes);
    }
}

#[test]
fn schema_four_migration_is_additive_and_preserves_legacy_payloads_and_history() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("portfolio.sqlite");
    let repo = Repository::open(&path).unwrap();
    let pocket = repo.upsert_cash_pocket(pocket()).unwrap();
    repo.upsert_fund(FundInput {
        id: None,
        ticker: "TEST11".into(),
        name: "Fundo sintético".into(),
        sector: "Teste".into(),
    })
    .unwrap();
    let fund_id = repo.snapshot().unwrap().funds[0].id.clone();
    repo.save_monthly_record(RecordInput {
        id: None,
        fund_id,
        month: "2020-01".into(),
        cotas_added: "10".into(),
        price_per_cota: "9.91".into(),
        dividend_per_cota: "0.12".into(),
        dividend_total: Some("1.17".into()),
        notes: "Preservar histórico".into(),
    })
    .unwrap();
    let before = repo.export_legacy_json().unwrap();
    let mut expected_queue = repo.pending_changes().unwrap();
    for change in &mut expected_queue {
        if change.entity_type == "cashPocket" {
            for key in ["totalDeposits", "totalWithdrawals", "balanceDate"] {
                change.payload.as_object_mut().unwrap().remove(key);
            }
        }
    }
    drop(repo);
    let old = Connection::open(&path).unwrap();
    old.execute_batch("BEGIN IMMEDIATE; ALTER TABLE cash_pockets DROP COLUMN total_deposits; ALTER TABLE cash_pockets DROP COLUMN total_withdrawals; ALTER TABLE cash_pockets DROP COLUMN balance_date; PRAGMA user_version=4;").unwrap();
    for change in &expected_queue {
        old.execute(
            "UPDATE sync_outbox SET payload=?1 WHERE id=?2",
            params![serde_json::to_string(&change.payload).unwrap(), change.id],
        )
        .unwrap();
    }
    old.execute_batch("COMMIT;").unwrap();
    drop(old);
    let upgraded = Repository::open_existing(&path).unwrap();
    assert_eq!(upgraded.export_legacy_json().unwrap(), before);
    assert_eq!(upgraded.cash_pockets().unwrap(), vec![pocket]);
    assert_eq!(
        serde_json::to_value(upgraded.pending_changes().unwrap()).unwrap(),
        serde_json::to_value(&expected_queue).unwrap()
    );
    assert_eq!(upgraded.pending_change_count().unwrap(), 3);
    assert_eq!(
        upgraded.snapshot().unwrap().funds[0].entries[0]
            .dividend_total
            .as_deref(),
        Some("1.17")
    );
    drop(upgraded);
    let database = Connection::open(&path).unwrap();
    let version: u32 = database
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 6);
    let group: (Option<String>, Option<String>, Option<String>) = database
        .query_row(
            "SELECT total_deposits,total_withdrawals,balance_date FROM cash_pockets",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(group, (None, None, None));
    drop(database);
    let reopened = Repository::open_existing(&path).unwrap();
    assert_eq!(reopened.export_legacy_json().unwrap(), before);
    assert_eq!(
        serde_json::to_value(reopened.pending_changes().unwrap()).unwrap(),
        serde_json::to_value(expected_queue).unwrap()
    );
}

#[test]
fn schema_newer_than_six_is_rejected_without_downgrading() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("future.sqlite");
    drop(Repository::open(&path).unwrap());
    let db = Connection::open(&path).unwrap();
    db.pragma_update(None, "user_version", 7).unwrap();
    drop(db);
    assert!(Repository::open_existing(&path).is_err());
    let db = Connection::open(path).unwrap();
    let version: u32 = db
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 7);
}
