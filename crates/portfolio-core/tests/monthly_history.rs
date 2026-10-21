use ivestfii_core::{
    portfolio_evolution::PortfolioHistoryObservation, CashPocketInput, FundInput, RecordInput,
    Repository, WalletCatalog, WalletInput,
};
use rusqlite::{types::ValueRef, Connection};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::Path};
use tempfile::TempDir;

fn observation(month: &str, as_of: &str, result: &str) -> PortfolioHistoryObservation {
    PortfolioHistoryObservation {
        month: month.into(),
        as_of: as_of.into(),
        invested: "321.09".into(),
        portfolio_result: result.into(),
        basis_fingerprint: "a".repeat(64),
    }
}

fn financial_rows(path: &Path) -> BTreeMap<String, Vec<Vec<String>>> {
    let connection = Connection::open(path).unwrap();
    let mut tables = connection
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name != 'performance_history' ORDER BY name")
        .unwrap();
    let names = tables
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    names
        .into_iter()
        .map(|name| {
            let mut statement = connection
                .prepare(&format!("SELECT * FROM \"{name}\" ORDER BY rowid"))
                .unwrap();
            let columns = statement.column_count();
            let rows = statement
                .query_map([], |row| {
                    (0..columns)
                        .map(|index| {
                            Ok(match row.get_ref(index)? {
                                ValueRef::Null => "null".into(),
                                ValueRef::Integer(value) => format!("integer:{value}"),
                                ValueRef::Real(value) => format!("real:{value}"),
                                ValueRef::Text(value) => format!("text:{:?}", value),
                                ValueRef::Blob(value) => format!("blob:{:?}", value),
                            })
                        })
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            (name, rows)
        })
        .collect()
}

fn seed_finances(repository: &Repository) {
    repository
        .upsert_fund(FundInput {
            id: None,
            ticker: "TEST11".into(),
            name: "Fundo sintético".into(),
            sector: "Teste".into(),
        })
        .unwrap();
    repository
        .save_monthly_record(RecordInput {
            id: None,
            fund_id: repository.snapshot().unwrap().funds[0].id.clone(),
            month: "2026-01".into(),
            purchase_date: Some("2026-01-05".into()),
            cotas_added: "3".into(),
            price_per_cota: "12.34".into(),
            dividend_per_cota: "0.1".into(),
            dividend_total: Some("0".into()),
            notes: "Valor zero explicitamente confirmado".into(),
        })
        .unwrap();
    repository
        .upsert_cash_pocket(
            serde_json::from_value::<CashPocketInput>(json!({
                "name":"Reserva sintética", "institution":"Instituição sintética",
                "balance":"21.11", "cdiPercent":"100", "notes":"Dados de teste",
                "totalDeposits":"25", "totalWithdrawals":"5", "balanceDate":"2026-02-10",
                "cashFlows":[
                    {"date":"2026-01-05","kind":"deposit","amount":"25"},
                    {"date":"2026-02-09","kind":"withdrawal","amount":"5"}
                ]
            }))
            .unwrap(),
        )
        .unwrap();
}

#[test]
fn schema_eight_upgrade_adds_private_history_without_rewriting_any_existing_rows() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("portfolio.sqlite");
    let repository = Repository::open(&path).unwrap();
    seed_finances(&repository);
    let before_snapshot = serde_json::to_value(repository.snapshot().unwrap()).unwrap();
    let before_export = repository.export_legacy_json().unwrap();
    let before_rows = financial_rows(&path);
    drop(repository);
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch("DROP TABLE performance_history; PRAGMA user_version = 8;")
        .unwrap();
    drop(connection);

    let upgraded = Repository::open_existing(&path).unwrap();
    assert_eq!(financial_rows(&path), before_rows);
    assert_eq!(upgraded.export_legacy_json().unwrap(), before_export);
    assert_eq!(
        serde_json::to_value(upgraded.snapshot().unwrap()).unwrap(),
        before_snapshot
    );
    assert!(upgraded.performance_history().unwrap().is_empty());
    drop(upgraded);
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
            .unwrap(),
        10
    );
}

#[test]
fn observations_are_private_sorted_and_keep_latest_date_with_same_date_corrections() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("portfolio.sqlite");
    let repository = Repository::open(&path).unwrap();
    seed_finances(&repository);
    let finances = financial_rows(&path);
    let february = observation("2026-02", "2026-02-10", "-1.23");
    let january = observation("2026-01", "2026-01-30", "0");
    repository
        .record_performance_observation(&february)
        .unwrap();
    repository.record_performance_observation(&january).unwrap();
    repository
        .record_performance_observation(&observation("2026-02", "2026-02-09", "9999"))
        .unwrap();
    assert_eq!(
        repository.performance_history().unwrap(),
        vec![january.clone(), february]
    );
    let corrected = observation("2026-02", "2026-02-10", "2.34");
    repository
        .record_performance_observation(&corrected)
        .unwrap();
    repository
        .record_performance_observation(&corrected)
        .unwrap();
    assert_eq!(
        repository.snapshot().unwrap().performance_history,
        vec![january.clone(), corrected.clone()]
    );
    assert_eq!(financial_rows(&path), finances);
    drop(repository);
    let reopened = Repository::open_existing(&path).unwrap();
    assert_eq!(
        reopened.performance_history().unwrap(),
        vec![january, corrected]
    );
    assert_eq!(financial_rows(&path), finances);
}

#[test]
fn separate_wallets_do_not_share_monthly_observations() {
    let directory = TempDir::new().unwrap();
    let catalog = WalletCatalog::open(directory.path()).unwrap();
    let secondary = catalog
        .create(WalletInput {
            id: None,
            name: "Segunda carteira".into(),
            institution: "Instituição sintética".into(),
        })
        .unwrap();
    let main = catalog.repository(None).unwrap();
    let other = catalog.repository(Some(&secondary.id)).unwrap();
    let first = observation("2026-01", "2026-01-30", "1");
    let second = observation("2026-01", "2026-01-30", "2");
    main.record_performance_observation(&first).unwrap();
    assert!(other.performance_history().unwrap().is_empty());
    other.record_performance_observation(&second).unwrap();
    assert_eq!(main.performance_history().unwrap(), vec![first]);
    assert_eq!(other.performance_history().unwrap(), vec![second]);
    assert_eq!(main.pending_change_count().unwrap(), 0);
    assert_eq!(other.pending_change_count().unwrap(), 0);
}

#[test]
fn backup_roundtrip_preserves_observations_exactly_without_financial_outbox_entries() {
    let directory = TempDir::new().unwrap();
    let source = Repository::open(directory.path().join("source.sqlite")).unwrap();
    seed_finances(&source);
    for row in [
        observation("2026-01", "2026-01-30", "-0.01"),
        observation("2026-02", "2026-02-27", "12.34"),
    ] {
        source.record_performance_observation(&row).unwrap();
    }
    let exported = source.export_legacy_json().unwrap();
    let input = directory.path().join("backup.json");
    fs::write(&input, &exported).unwrap();
    let destination = Repository::open(directory.path().join("destination.sqlite")).unwrap();
    let receipt = destination.import_legacy_file(&input).unwrap();
    assert_eq!(
        destination.performance_history().unwrap(),
        source.performance_history().unwrap()
    );
    assert_eq!(destination.export_legacy_json().unwrap(), exported);
    assert_eq!(
        destination.pending_change_count().unwrap(),
        source.pending_change_count().unwrap()
    );
    assert_eq!(fs::read(&input).unwrap(), exported.as_bytes());
    assert_eq!(fs::read(receipt.backup_path).unwrap(), exported.as_bytes());
}

#[test]
fn old_backups_and_old_snapshot_payloads_have_empty_history() {
    let directory = TempDir::new().unwrap();
    let input = directory.path().join("legacy.json");
    fs::write(&input, br#"{"fiis":[]}"#).unwrap();
    let repository = Repository::open(directory.path().join("portfolio.sqlite")).unwrap();
    repository.import_legacy_file(input).unwrap();
    assert!(repository.performance_history().unwrap().is_empty());
    let mut snapshot = serde_json::to_value(repository.snapshot().unwrap()).unwrap();
    snapshot
        .as_object_mut()
        .unwrap()
        .remove("performanceHistory");
    let restored: ivestfii_core::PortfolioSnapshot = serde_json::from_value(snapshot).unwrap();
    assert!(restored.performance_history.is_empty());
    let backup: Value = serde_json::from_str(&repository.export_legacy_json().unwrap()).unwrap();
    assert!(backup.get("performanceHistory").is_none());
}

#[test]
fn invalid_observations_do_not_mutate_saved_history_or_financial_rows() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("portfolio.sqlite");
    let repository = Repository::open(&path).unwrap();
    seed_finances(&repository);
    let valid = observation("2026-01", "2026-01-30", "1.23");
    repository.record_performance_observation(&valid).unwrap();
    let before = financial_rows(&path);
    let mut invalids = Vec::new();
    for (field, value) in [
        ("month", "2026-1"),
        ("month", "2026-02"),
        ("asOf", "2026-01-32"),
        ("asOf", "2026-1-30"),
        ("invested", "-0.01"),
        ("invested", "NaN"),
        ("portfolioResult", "Infinity"),
        ("basisFingerprint", "not-a-fingerprint"),
    ] {
        let mut row = serde_json::to_value(&valid).unwrap();
        row[field] = json!(value);
        invalids.push(serde_json::from_value(row).unwrap());
    }
    invalids.push(observation("2999-01", "2999-01-01", "1"));
    for invalid in invalids {
        assert!(repository.record_performance_observation(&invalid).is_err());
        assert_eq!(
            repository.performance_history().unwrap(),
            vec![valid.clone()]
        );
        assert_eq!(financial_rows(&path), before);
    }
}

#[test]
fn invalid_or_duplicate_history_in_backup_is_rejected_before_importing_other_data() {
    let directory = TempDir::new().unwrap();
    let source = Repository::open(directory.path().join("source.sqlite")).unwrap();
    seed_finances(&source);
    let valid = serde_json::to_value(observation("2026-01", "2026-01-30", "1")).unwrap();
    let mut bad = valid.clone();
    bad["invested"] = json!("-1");
    for (index, invalid) in [
        json!([valid.clone(), bad]),
        json!([valid.clone(), valid.clone()]),
        json!({"month":"2026-01"}),
        Value::Null,
    ]
    .into_iter()
    .enumerate()
    {
        let mut backup: Value =
            serde_json::from_str(&source.export_legacy_json().unwrap()).unwrap();
        backup["performanceHistory"] = invalid;
        let input = directory.path().join(format!("invalid-{index}.json"));
        fs::write(&input, serde_json::to_vec(&backup).unwrap()).unwrap();
        let path = directory.path().join(format!("destination-{index}.sqlite"));
        let repository = Repository::open(&path).unwrap();
        let before = financial_rows(&path);
        assert!(repository.import_legacy_file(input).is_err());
        assert_eq!(financial_rows(&path), before);
        assert!(repository.performance_history().unwrap().is_empty());
    }
}

#[test]
fn stored_payload_key_mismatch_is_reported_instead_of_becoming_a_fake_month() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("portfolio.sqlite");
    let repository = Repository::open(&path).unwrap();
    let row = observation("2026-01", "2026-01-30", "1");
    repository.record_performance_observation(&row).unwrap();
    let connection = Connection::open(&path).unwrap();
    connection
        .execute("UPDATE performance_history SET month='2026-02'", [])
        .unwrap();
    assert!(repository.performance_history().is_err());
    assert!(repository.snapshot().is_err());
    assert_eq!(repository.pending_change_count().unwrap(), 0);
}
