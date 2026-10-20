use ivestfii_core::{FundInput, ProjectionInput, RecordInput, Repository};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write_fixture(dir: &Path, value: &Value) -> std::path::PathBuf {
    let source = dir.join("original.json");
    fs::write(&source, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    source
}

fn fixture() -> Value {
    json!({"fiis":[{
        "ticker":"TEST11", "name":"Fundo teste", "sector":"Papel",
        "entries":[
            {"month":"2025-01","cotas_added":10,"price_per_cota":9.91,"dividend_per_cota":0.1,"dividend_total":0.97,"notes":"Valor efetivamente recebido"},
            {"month":"2025-02","cotas_added":2,"price_per_cota":10.02,"dividend_per_cota":0.11,"dividend_total":null,"notes":"Automático"},
            {"month":"2025-02","cotas_added":2,"price_per_cota":10.02,"dividend_per_cota":0.11,"dividend_total":null,"notes":"Automático"}
        ]
    }]})
}

fn create_fund(repo: &Repository) -> String {
    repo.upsert_fund(FundInput {
        id: None,
        ticker: "TEST11".into(),
        name: "Teste".into(),
        sector: "Papel".into(),
    })
    .unwrap();
    repo.snapshot().unwrap().funds[0].id.clone()
}

fn record(fund_id: &str, month: &str, cotas: &str, dividend: &str) -> RecordInput {
    RecordInput {
        id: None,
        fund_id: fund_id.into(),
        month: month.into(),
        cotas_added: cotas.into(),
        price_per_cota: "10".into(),
        dividend_per_cota: dividend.into(),
        dividend_total: None,
        notes: String::new(),
    }
}

#[test]
fn migration_is_exactly_backed_up_idempotent_and_preserves_duplicate_months() {
    let dir = TempDir::new().unwrap();
    let source = write_fixture(dir.path(), &fixture());
    let original = fs::read(&source).unwrap();
    let db_path = dir.path().join("new-app").join("portfolio.sqlite");
    let repo = Repository::open(&db_path).unwrap();

    let first = repo.import_legacy_file(&source).unwrap();
    assert_eq!(first.fund_count, 1);
    assert_eq!(first.record_count, 3);
    assert!(!first.already_imported);
    assert_eq!(fs::read(&first.backup_path).unwrap(), original);
    assert_eq!(fs::read(&source).unwrap(), original);
    let snapshot = repo.snapshot().unwrap();
    assert_eq!(snapshot.record_count, 3);
    assert_eq!(
        snapshot.funds[0].entries[0].dividend_total.as_deref(),
        Some("0.97")
    );
    assert_eq!(snapshot.funds[0].entries[1].dividend_total, None);
    assert_ne!(
        snapshot.funds[0].entries[1].id,
        snapshot.funds[0].entries[2].id
    );
    assert_eq!(snapshot.totals.invested, "139.18");
    assert_eq!(snapshot.totals.cotas, "14");
    assert_eq!(snapshot.totals.dividends_received, "4.05");
    let pending = repo.pending_change_count().unwrap();
    assert_eq!(pending, 4);

    let second = repo.import_legacy_file(&source).unwrap();
    assert!(second.already_imported);
    assert_eq!(second.sha256, first.sha256);
    assert_eq!(repo.snapshot().unwrap().record_count, 3);
    assert_eq!(repo.pending_change_count().unwrap(), pending);
    drop(repo);

    let reopened = Repository::open(db_path).unwrap();
    assert_eq!(
        reopened.snapshot().unwrap().funds[0].entries[2].id,
        snapshot.funds[0].entries[2].id
    );
    assert_eq!(reopened.pending_change_count().unwrap(), pending);
}

#[test]
fn malformed_import_keeps_database_empty_and_source_untouched() {
    let dir = TempDir::new().unwrap();
    let mut invalid = fixture();
    invalid["fiis"][0]["entries"][2]["price_per_cota"] = json!("invalid");
    let source = write_fixture(dir.path(), &invalid);
    let original = fs::read(&source).unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    assert!(repo.import_legacy_file(&source).is_err());
    assert_eq!(repo.snapshot().unwrap().record_count, 0);
    assert!(repo.snapshot().unwrap().funds.is_empty());
    assert_eq!(repo.pending_change_count().unwrap(), 0);
    assert_eq!(fs::read(&source).unwrap(), original);
    let backups: Vec<_> = fs::read_dir(dir.path().join("backups")).unwrap().collect();
    assert_eq!(backups.len(), 1);
    assert_eq!(
        fs::read(backups[0].as_ref().unwrap().path()).unwrap(),
        original
    );
}

#[test]
fn arithmetic_failure_rolls_back_every_imported_fund_and_outbox_row() {
    let dir = TempDir::new().unwrap();
    let mut value = fixture();
    value["fiis"][0]["entries"] = json!([{
        "month":"2025-01","cotas_added":"79228162514264337593543950335","price_per_cota":0,"dividend_per_cota":0
    }]);
    let mut second = value["fiis"][0].clone();
    second["ticker"] = json!("OTHER11");
    value["fiis"].as_array_mut().unwrap().push(second);
    let source = write_fixture(dir.path(), &value);
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    assert!(repo.import_legacy_file(&source).is_err());
    assert!(repo.snapshot().unwrap().funds.is_empty());
    assert_eq!(repo.pending_change_count().unwrap(), 0);
    assert!(repo.import_legacy_file(&source).is_err());
}

#[test]
fn a_different_file_cannot_silently_merge_or_overwrite_an_existing_portfolio() {
    let dir = TempDir::new().unwrap();
    let source = write_fixture(dir.path(), &fixture());
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    repo.import_legacy_file(&source).unwrap();
    let before = repo.export_legacy_json().unwrap();
    let pending = repo.pending_change_count().unwrap();
    let mut changed = fixture();
    changed["fiis"][0]["name"] = json!("Outro nome");
    let changed_file = dir.path().join("another.json");
    fs::write(&changed_file, serde_json::to_vec(&changed).unwrap()).unwrap();
    assert!(repo.import_legacy_file(&changed_file).is_err());
    assert_eq!(repo.export_legacy_json().unwrap(), before);
    assert_eq!(repo.pending_change_count().unwrap(), pending);
}

#[test]
fn retroactive_entries_and_edits_use_historical_holdings_and_keep_explicit_totals() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    let fund_id = create_fund(&repo);
    repo.save_monthly_record(record(&fund_id, "2025-03", "100", "0"))
        .unwrap();
    repo.save_monthly_record(record(&fund_id, "2025-01", "10", "0.1"))
        .unwrap();
    repo.save_monthly_record(record(&fund_id, "2025-02", "0", "0.1"))
        .unwrap();
    assert_eq!(repo.snapshot().unwrap().totals.dividends_received, "2");

    let snapshot = repo.snapshot().unwrap();
    let mut january = record(&fund_id, "2025-01", "20", "0.1");
    january.id = Some(snapshot.funds[0].entries[0].id.clone());
    repo.save_monthly_record(january).unwrap();
    assert_eq!(repo.snapshot().unwrap().totals.dividends_received, "4");

    let snapshot = repo.snapshot().unwrap();
    let mut february = record(&fund_id, "2025-02", "0", "0.1");
    february.id = Some(snapshot.funds[0].entries[1].id.clone());
    february.dividend_total = Some("0.73".into());
    repo.save_monthly_record(february).unwrap();
    assert_eq!(repo.snapshot().unwrap().totals.dividends_received, "2.73");

    let snapshot = repo.snapshot().unwrap();
    let mut january = record(&fund_id, "2025-01", "30", "0.1");
    january.id = Some(snapshot.funds[0].entries[0].id.clone());
    repo.save_monthly_record(january).unwrap();
    assert_eq!(repo.snapshot().unwrap().totals.dividends_received, "3.73");
    assert_eq!(
        repo.snapshot().unwrap().funds[0].entries[1]
            .dividend_total
            .as_deref(),
        Some("0.73")
    );
}

#[test]
fn edits_address_record_id_rather_than_ambiguous_month() {
    let dir = TempDir::new().unwrap();
    let source = write_fixture(dir.path(), &fixture());
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    repo.import_legacy_file(&source).unwrap();
    let before = repo.snapshot().unwrap();
    let fund = &before.funds[0];
    let mut edited = record(&fund.id, "2025-02", "5", "0.2");
    edited.id = Some(fund.entries[2].id.clone());
    edited.notes = "Somente o segundo lançamento de fevereiro".into();
    repo.save_monthly_record(edited).unwrap();
    let after = repo.snapshot().unwrap();
    assert_eq!(after.record_count, 3);
    assert_eq!(after.funds[0].entries[1].cotas_added, "2");
    assert_eq!(after.funds[0].entries[2].cotas_added, "5");
    let changes = repo.pending_changes().unwrap();
    assert_eq!(changes.last().unwrap().payload["revision"], json!(2));
}

#[test]
fn failed_record_update_does_not_modify_data_or_enqueue_a_change() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    let fund_id = create_fund(&repo);
    repo.save_monthly_record(record(&fund_id, "2025-01", "10", "0.1"))
        .unwrap();
    let before = repo.export_legacy_json().unwrap();
    let pending = repo.pending_change_count().unwrap();
    assert!(repo
        .save_monthly_record(record(
            &fund_id,
            "2025-02",
            "79228162514264337593543950335",
            "1"
        ))
        .is_err());
    assert_eq!(repo.export_legacy_json().unwrap(), before);
    assert_eq!(repo.pending_change_count().unwrap(), pending);
    assert!(repo
        .save_monthly_record(record(&fund_id, "2025-13", "1", "0.1"))
        .is_err());
    assert!(repo
        .save_monthly_record(record(&fund_id, "2025-02", "-1", "0.1"))
        .is_err());
}

#[test]
fn duplicate_fund_creation_cannot_erase_metadata_or_enqueue_a_change() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    let fund_id = create_fund(&repo);
    repo.save_monthly_record(record(&fund_id, "2025-01", "10", "0.1"))
        .unwrap();
    let before = repo.export_legacy_json().unwrap();
    let pending = repo.pending_change_count().unwrap();

    let result = repo.upsert_fund(FundInput {
        id: None,
        ticker: " test11 ".into(),
        name: String::new(),
        sector: String::new(),
    });
    assert!(result.is_err());
    assert_eq!(repo.export_legacy_json().unwrap(), before);
    assert_eq!(repo.pending_change_count().unwrap(), pending);

    // An explicit edit still permits intentional metadata changes.
    repo.upsert_fund(FundInput {
        id: Some(fund_id.clone()),
        ticker: "TEST11".into(),
        name: "Nome atualizado".into(),
        sector: String::new(),
    })
    .unwrap();
    let after = repo.snapshot().unwrap();
    assert_eq!(after.funds[0].id, fund_id);
    assert_eq!(after.funds[0].name, "Nome atualizado");
    assert_eq!(after.funds[0].sector, "");
    assert_eq!(after.record_count, 1);
    assert_eq!(repo.pending_change_count().unwrap(), pending + 1);
}

#[test]
fn export_and_reimport_preserve_decimal_values_nulls_and_stable_ids() {
    let dir = TempDir::new().unwrap();
    let source = write_fixture(dir.path(), &fixture());
    let first = Repository::open(dir.path().join("first.sqlite")).unwrap();
    first.import_legacy_file(&source).unwrap();
    let export = first.export_legacy_json().unwrap();
    let value: Value = serde_json::from_str(&export).unwrap();
    assert!(value["fiis"][0]["entries"][0]["price_per_cota"].is_number());
    assert!(value["fiis"][0]["entries"][1]["dividend_total"].is_null());
    let export_path = dir.path().join("export.json");
    fs::write(&export_path, export).unwrap();
    let second = Repository::open(dir.path().join("second.sqlite")).unwrap();
    second.import_legacy_file(export_path).unwrap();
    assert_eq!(
        first.export_legacy_json().unwrap(),
        second.export_legacy_json().unwrap()
    );
    let first_ids: Vec<_> = first.snapshot().unwrap().funds[0]
        .entries
        .iter()
        .map(|e| e.id.clone())
        .collect();
    let second_ids: Vec<_> = second.snapshot().unwrap().funds[0]
        .entries
        .iter()
        .map(|e| e.id.clone())
        .collect();
    assert_eq!(first_ids, second_ids);
}

#[test]
fn projection_keeps_reinvestment_explicit_and_uses_decimal_cash_remainder() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    let fund_id = create_fund(&repo);
    repo.save_monthly_record(record(&fund_id, "2025-12", "10", "1"))
        .unwrap();
    let without = repo
        .project(ProjectionInput {
            fund_id: Some(fund_id.clone()),
            months: 2,
            monthly_cotas: "0".into(),
            window: Some(6),
            reinvest: false,
        })
        .unwrap();
    assert_eq!(without[0].month, "2026-01");
    assert_eq!(without[1].combined_cotas, "10");
    assert_eq!(without[1].cumulative_income, "20");
    let with = repo
        .project(ProjectionInput {
            fund_id: Some(fund_id),
            months: 2,
            monthly_cotas: "0".into(),
            window: Some(6),
            reinvest: true,
        })
        .unwrap();
    assert_eq!(with[0].projected_income, "10");
    assert_eq!(with[0].combined_cotas, "11");
    assert_eq!(with[1].projected_income, "11");
    assert_eq!(with[1].combined_cotas, "12");
    assert_eq!(with[1].cumulative_income, "21");
}

#[test]
fn malformed_or_unrecognized_json_never_silently_becomes_an_empty_portfolio() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    for (name, bytes) in [
        ("broken.json", "{broken"),
        ("other.json", "{}"),
        ("null.json", "null"),
    ] {
        let source = dir.path().join(name);
        fs::write(&source, bytes).unwrap();
        assert!(repo.import_legacy_file(source).is_err());
    }
    assert!(repo.snapshot().unwrap().funds.is_empty());
}

#[test]
fn scientific_notation_is_exact_and_excess_precision_is_rejected() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("scientific.json");
    fs::write(&source, r#"{"fiis":[{"ticker":"TEST11","entries":[{"month":"2025-01","cotas_added":10,"price_per_cota":1.234e1,"dividend_per_cota":1e-2,"dividend_total":9.700e-2}]}]}"#).unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    repo.import_legacy_file(&source).unwrap();
    let snapshot = repo.snapshot().unwrap();
    assert_eq!(snapshot.funds[0].entries[0].price_per_cota, "12.34");
    assert_eq!(snapshot.funds[0].entries[0].dividend_per_cota, "0.01");
    assert_eq!(snapshot.totals.invested, "123.4");
    assert_eq!(snapshot.totals.dividends_received, "0.097");

    let excessive = dir.path().join("excessive.json");
    fs::write(&excessive, r#"{"fiis":[{"ticker":"TEST11","entries":[{"month":"2025-01","price_per_cota":1.234567890123456789012345678901e-1}]}]}"#).unwrap();
    let empty = Repository::open(dir.path().join("empty.sqlite")).unwrap();
    assert!(empty.import_legacy_file(excessive).is_err());
    assert!(empty.snapshot().unwrap().funds.is_empty());
    assert_eq!(empty.pending_change_count().unwrap(), 0);
}
