use ivestfii_core::{FundInput, RecordInput, Repository};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::fs;
use tempfile::TempDir;

fn record(fund_id: &str) -> RecordInput {
    RecordInput {
        id: None,
        fund_id: fund_id.into(),
        month: "2026-07".into(),
        purchase_date: Some("2026-07-03".into()),
        cotas_added: "10".into(),
        price_per_cota: "9.91".into(),
        dividend_per_cota: "0.10".into(),
        dividend_total: Some("0.97".into()),
        notes: "Compra com data informada pelo usuário".into(),
    }
}

fn fund(repo: &Repository) -> String {
    repo.upsert_fund(FundInput {
        id: None,
        ticker: "TEST11".into(),
        name: "Fundo sintético".into(),
        sector: "Teste".into(),
    })
    .unwrap();
    repo.snapshot().unwrap().funds[0].id.clone()
}

fn outbox(repo: &Repository) -> Value {
    serde_json::to_value(repo.pending_changes().unwrap()).unwrap()
}

#[test]
fn explicit_purchase_date_survives_reopen_and_backup_without_changing_financial_values() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("portfolio.sqlite");
    let repo = Repository::open(&path).unwrap();
    let fund_id = fund(&repo);
    let mut input = record(&fund_id);
    input.purchase_date = None;
    repo.save_monthly_record(input).unwrap();
    let mut before = repo.snapshot().unwrap();
    let entry = &before.funds[0].entries[0];
    assert_eq!(entry.purchase_date, None);
    assert!(serde_json::to_value(entry)
        .unwrap()
        .get("purchaseDate")
        .is_none());
    let mut dated = record(&fund_id);
    dated.id = Some(entry.id.clone());
    repo.save_monthly_record(dated).unwrap();
    before.funds[0].entries[0].purchase_date = Some("2026-07-03".into());
    let snapshot = serde_json::to_value(before).unwrap();
    assert_eq!(
        serde_json::to_value(repo.snapshot().unwrap()).unwrap(),
        snapshot
    );
    let queue = repo.pending_changes().unwrap();
    assert_eq!(queue.last().unwrap().payload["purchaseDate"], "2026-07-03");
    let backup = repo.export_legacy_json().unwrap();
    let value: Value = serde_json::from_str(&backup).unwrap();
    assert_eq!(
        value["fiis"][0]["entries"][0]["purchase_date"],
        "2026-07-03"
    );
    drop(repo);
    let reopened = Repository::open_existing(&path).unwrap();
    assert_eq!(
        serde_json::to_value(reopened.snapshot().unwrap()).unwrap(),
        snapshot
    );
    let backup_path = dir.path().join("backup.json");
    fs::write(&backup_path, backup.as_bytes()).unwrap();
    let restored = Repository::open(dir.path().join("restored.sqlite")).unwrap();
    restored.import_legacy_file(&backup_path).unwrap();
    let actual = restored.snapshot().unwrap();
    assert_eq!(
        actual.funds[0].entries[0].purchase_date.as_deref(),
        Some("2026-07-03")
    );
    assert_eq!(actual.totals.invested, "99.1");
    assert_eq!(actual.totals.cotas, "10");
    assert_eq!(actual.totals.dividends_received, "0.97");
    assert_eq!(restored.export_legacy_json().unwrap(), backup);
}

#[test]
fn invalid_or_unrelated_purchase_dates_are_rejected_atomically() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    let fund_id = fund(&repo);
    let queue = outbox(&repo);
    for (date, shares) in [
        ("2026-07-32", "10"),
        ("2026-02-29", "10"),
        ("2026-7-03", "10"),
        ("2026-07-3", "10"),
        ("03/07/2026", "10"),
        ("2026-07-03T00:00:00Z", "10"),
        ("2026-06-30", "10"),
        ("2026-08-01", "10"),
        ("0000-07-03", "10"),
        ("", "10"),
        ("2026-07-03", "0"),
    ] {
        let mut input = record(&fund_id);
        input.purchase_date = Some(date.into());
        input.cotas_added = shares.into();
        assert!(repo.save_monthly_record(input).is_err(), "{date} {shares}");
        assert_eq!(repo.snapshot().unwrap().record_count, 0);
        assert_eq!(outbox(&repo), queue);
    }
    let mut leap_day = record(&fund_id);
    leap_day.month = "2024-02".into();
    leap_day.purchase_date = Some("2024-02-29".into());
    repo.save_monthly_record(leap_day).unwrap();
    assert_eq!(
        repo.snapshot().unwrap().funds[0].entries[0]
            .purchase_date
            .as_deref(),
        Some("2024-02-29")
    );
}

#[test]
fn legacy_backups_and_inputs_keep_dates_unknown_and_old_json_shape_exactly() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("legacy.json");
    let legacy = json!({"fiis":[{
        "ticker":"TEST11", "name":"Fundo sintético", "sector":"Teste",
        "entries":[
            {"month":"2026-06","cotas_added":10,"price_per_cota":9.91,"dividend_per_cota":0.1,"dividend_total":null,"notes":"Sem data de compra no legado"},
            {"month":"2026-07","cotas_added":0,"price_per_cota":0,"dividend_per_cota":0.1,"dividend_total":0.97,"notes":"Só rendimento"}
        ]
    }]});
    let bytes = serde_json::to_vec_pretty(&legacy).unwrap();
    fs::write(&source, &bytes).unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    repo.import_legacy_file(&source).unwrap();
    let before = repo.snapshot().unwrap();
    assert!(before.funds[0]
        .entries
        .iter()
        .all(|entry| entry.purchase_date.is_none()));
    assert_eq!(
        serde_json::from_str::<Value>(&repo.export_legacy_json().unwrap()).unwrap(),
        legacy
    );
    let payload = json!({
        "fundId":before.funds[0].id, "month":"2026-08", "cotasAdded":"0",
        "pricePerCota":"0", "dividendPerCota":"0.1", "dividendTotal":null
    });
    let input: RecordInput = serde_json::from_value(payload).unwrap();
    assert_eq!(input.purchase_date, None);
    repo.save_monthly_record(input).unwrap();
    assert_eq!(repo.snapshot().unwrap().totals.cotas, "10");
    assert_eq!(repo.snapshot().unwrap().totals.dividends_received, "2.97");
    assert_eq!(fs::read(&source).unwrap(), bytes);
}

#[test]
fn invalid_purchase_date_in_backup_rolls_back_the_entire_import() {
    let dir = TempDir::new().unwrap();
    for (index, date) in [json!("2026-08-01"), json!(123), json!("2026-07-00")]
        .into_iter()
        .enumerate()
    {
        let source = dir.path().join(format!("invalid-{index}.json"));
        let legacy = json!({"fiis":[{
            "ticker":"TEST11", "name":"Fundo sintético", "sector":"Teste",
            "entries":[
                {"month":"2026-06","cotas_added":10,"price_per_cota":10,"dividend_per_cota":0.1},
                {"month":"2026-07","purchase_date":date,"cotas_added":2,"price_per_cota":10,"dividend_per_cota":0.1}
            ]
        }]});
        fs::write(&source, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let repo = Repository::open(dir.path().join(format!("invalid-{index}.sqlite"))).unwrap();
        assert!(repo.import_legacy_file(source).is_err());
        assert_eq!(repo.snapshot().unwrap().record_count, 0);
        assert!(repo.snapshot().unwrap().funds.is_empty());
        assert_eq!(repo.pending_change_count().unwrap(), 0);
    }
}

#[test]
fn schema_six_upgrade_adds_only_nullable_dates_without_touching_old_rows_or_outbox() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("portfolio.sqlite");
    let repo = Repository::open(&path).unwrap();
    let fund_id = fund(&repo);
    let mut input = record(&fund_id);
    input.purchase_date = None;
    repo.save_monthly_record(input).unwrap();
    let before = serde_json::to_value(repo.snapshot().unwrap()).unwrap();
    let export = repo.export_legacy_json().unwrap();
    let queue = outbox(&repo);
    drop(repo);
    let db = Connection::open(&path).unwrap();
    db.execute_batch("BEGIN IMMEDIATE; ALTER TABLE monthly_records DROP COLUMN purchase_date; PRAGMA user_version=6; COMMIT;").unwrap();
    let original: (String, String, i64, String) = db
        .query_row(
            "SELECT id, cotas_added, revision, updated_at FROM monthly_records",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    drop(db);
    let upgraded = Repository::open_existing(&path).unwrap();
    assert_eq!(
        serde_json::to_value(upgraded.snapshot().unwrap()).unwrap(),
        before
    );
    assert_eq!(upgraded.export_legacy_json().unwrap(), export);
    assert_eq!(outbox(&upgraded), queue);
    drop(upgraded);
    let db = Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
            .unwrap(),
        8
    );
    let actual: (String, String, i64, String) = db
        .query_row(
            "SELECT id, cotas_added, revision, updated_at FROM monthly_records",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(actual, original);
    assert_eq!(
        db.query_row("SELECT purchase_date FROM monthly_records", [], |row| {
            row.get::<_, Option<String>>(0)
        })
        .unwrap(),
        None
    );
    drop(db);
    let reopened = Repository::open_existing(&path).unwrap();
    assert_eq!(reopened.export_legacy_json().unwrap(), export);
    assert_eq!(outbox(&reopened), queue);
}
