use chrono::{Duration, Utc};
use ivestfii_core::{CashPocketInput, CashSimulationInput, FundInput, RecordInput, Repository};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::fs;
use tempfile::TempDir;

const CDI_SOURCE: &str =
    "https://api.bcb.gov.br/dados/serie/bcdata.sgs.4389/dados/ultimos/1?formato=json";
const FETCHED_AT: &str = "2020-01-03T12:00:00Z";

fn reference() -> CashPocketInput {
    CashPocketInput {
        id: None,
        name: "Reserva sintética".into(),
        institution: "Instituição de teste".into(),
        balance: "1234.56".into(),
        total_deposits: None,
        total_withdrawals: None,
        balance_date: None,
        clear_reconciliation: false,
        cdi_percent: "120".into(),
        annual_cdi_rate: Some("8.2".into()),
        rate_date: Some("2020-01-02".into()),
        rate_source: Some("bcb-sgs".into()),
        rate_source_url: Some(CDI_SOURCE.into()),
        rate_fetched_at: Some(FETCHED_AT.into()),
        notes: "Valores fictícios para teste offline".into(),
    }
}

fn outbox(repo: &Repository) -> Value {
    serde_json::to_value(repo.pending_changes().unwrap()).unwrap()
}

#[test]
fn bcb_reference_survives_reopen_export_import_and_read_only_simulation() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("source.sqlite");
    let repo = Repository::open(&path).unwrap();
    let saved = repo.upsert_cash_pocket(reference()).unwrap();
    assert_eq!(saved.rate_source, "bcb-sgs");
    assert_eq!(saved.rate_source_url.as_deref(), Some(CDI_SOURCE));
    assert_eq!(saved.rate_fetched_at.as_deref(), Some(FETCHED_AT));
    let queue = outbox(&repo);
    assert_eq!(queue[0]["payload"]["rateSource"], "bcb-sgs");
    assert_eq!(queue[0]["payload"]["rateSourceUrl"], CDI_SOURCE);
    assert_eq!(queue[0]["payload"]["rateFetchedAt"], FETCHED_AT);
    let before = repo.export_legacy_json().unwrap();
    let simulation = repo
        .simulate_cash_pocket(CashSimulationInput {
            pocket_id: saved.id.clone(),
            business_days: 21,
        })
        .unwrap();
    assert_eq!(simulation.rate_source, "bcb-sgs");
    assert_eq!(simulation.rate_source_url.as_deref(), Some(CDI_SOURCE));
    assert_eq!(simulation.rate_fetched_at.as_deref(), Some(FETCHED_AT));
    assert_eq!(repo.export_legacy_json().unwrap(), before);
    assert_eq!(outbox(&repo), queue);
    drop(repo);

    let reopened = Repository::open_existing(&path).unwrap();
    assert_eq!(reopened.cash_pockets().unwrap(), vec![saved.clone()]);
    assert_eq!(reopened.export_legacy_json().unwrap(), before);
    assert_eq!(outbox(&reopened), queue);
    let exported: Value = serde_json::from_str(&before).unwrap();
    assert_eq!(exported["cash_pockets"][0]["rateSource"], "bcb-sgs");
    assert_eq!(exported["cash_pockets"][0]["rateSourceUrl"], CDI_SOURCE);
    assert_eq!(exported["cash_pockets"][0]["rateFetchedAt"], FETCHED_AT);

    let backup = directory.path().join("backup.json");
    fs::write(&backup, before.as_bytes()).unwrap();
    let restored = Repository::open(directory.path().join("restored.sqlite")).unwrap();
    restored.import_legacy_file(&backup).unwrap();
    assert_eq!(restored.cash_pockets().unwrap(), vec![saved]);
    assert_eq!(restored.export_legacy_json().unwrap(), before);
    assert_eq!(restored.pending_change_count().unwrap(), 1);
    assert_eq!(
        restored.pending_changes().unwrap()[0].payload["rateSourceUrl"],
        CDI_SOURCE
    );
    assert!(
        restored
            .import_legacy_file(&backup)
            .unwrap()
            .already_imported
    );
    assert_eq!(restored.pending_change_count().unwrap(), 1);
    assert_eq!(fs::read_to_string(backup).unwrap(), before);
}

#[test]
fn explicit_manual_edit_removes_bcb_metadata_without_changing_balance_or_identity() {
    let directory = TempDir::new().unwrap();
    let repo = Repository::open(directory.path().join("portfolio.sqlite")).unwrap();
    let original = repo.upsert_cash_pocket(reference()).unwrap();
    let mut edited = reference();
    edited.id = Some(original.id.clone());
    edited.annual_cdi_rate = Some("7.25".into());
    edited.rate_date = Some("2020-01-06".into());
    edited.rate_source = Some("manual".into());
    edited.rate_source_url = None;
    edited.rate_fetched_at = None;
    let saved = repo.upsert_cash_pocket(edited).unwrap();
    assert_eq!(saved.id, original.id);
    assert_eq!(saved.created_at, original.created_at);
    assert_eq!(saved.balance, original.balance);
    assert_eq!(saved.cdi_percent, original.cdi_percent);
    assert_eq!(saved.annual_cdi_rate.as_deref(), Some("7.25"));
    assert_eq!(saved.rate_date.as_deref(), Some("2020-01-06"));
    assert_eq!(saved.rate_source, "manual");
    assert_eq!(saved.rate_source_url, None);
    assert_eq!(saved.rate_fetched_at, None);
    let changes = repo.pending_changes().unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[1].payload["revision"], 2);
    assert_eq!(changes[1].payload["rateSource"], "manual");
    assert_eq!(changes[1].payload["rateSourceUrl"], Value::Null);
    assert_eq!(changes[1].payload["rateFetchedAt"], Value::Null);
}

#[test]
fn invalid_provenance_and_incomplete_references_do_not_mutate_a_wallet_or_its_queue() {
    let directory = TempDir::new().unwrap();
    let repo = Repository::open(directory.path().join("portfolio.sqlite")).unwrap();
    let saved = repo.upsert_cash_pocket(reference()).unwrap();
    let before = repo.export_legacy_json().unwrap();
    let queue = outbox(&repo);
    let mut valid_edit = reference();
    valid_edit.id = Some(saved.id);
    valid_edit.balance = "9999.99".into();
    let mut cases = Vec::new();
    for (label, url) in [
        ("foreign origin", "https://example.com/cdi"),
        (
            "different BCB series",
            "https://api.bcb.gov.br/dados/serie/bcdata.sgs.432/dados/ultimos/1?formato=json",
        ),
    ] {
        let mut input = valid_edit.clone();
        input.rate_source_url = Some(url.into());
        cases.push((label, input));
    }
    let mut input = valid_edit.clone();
    input.rate_source_url = None;
    cases.push(("missing BCB source URL", input));
    let mut input = valid_edit.clone();
    input.rate_fetched_at = None;
    cases.push(("missing fetched timestamp", input));
    let mut input = valid_edit.clone();
    input.rate_fetched_at = Some("invalid timestamp".into());
    cases.push(("invalid timestamp", input));
    let mut input = valid_edit.clone();
    input.rate_fetched_at = Some((Utc::now() + Duration::days(1)).to_rfc3339());
    cases.push(("future fetched timestamp", input));
    let mut input = valid_edit.clone();
    input.rate_fetched_at = Some("2020-01-01T12:00:00Z".into());
    cases.push(("fetch preceding reference date", input));
    let mut input = valid_edit.clone();
    input.rate_source = Some("unrecognized-provider".into());
    cases.push(("unknown source", input));
    let mut input = valid_edit.clone();
    input.rate_source = Some("manual".into());
    input.rate_fetched_at = None;
    cases.push(("manual source with URL", input));
    let mut input = valid_edit.clone();
    input.rate_source = None;
    input.rate_source_url = None;
    cases.push(("implicit manual source with fetched timestamp", input));
    let mut input = valid_edit.clone();
    input.rate_date = None;
    cases.push(("rate without date", input));
    let mut input = valid_edit.clone();
    input.annual_cdi_rate = None;
    cases.push(("date without rate", input));
    let mut input = valid_edit;
    input.rate_date = None;
    input.annual_cdi_rate = None;
    cases.push(("BCB metadata without any reference", input));

    for (label, input) in cases {
        assert!(repo.upsert_cash_pocket(input).is_err(), "{label}");
        assert_eq!(repo.export_legacy_json().unwrap(), before, "{label}");
        assert_eq!(outbox(&repo), queue, "{label}");
    }
}

#[test]
fn invalid_bcb_metadata_in_backup_cannot_be_imported_partially() {
    let directory = TempDir::new().unwrap();
    let original = Repository::open(directory.path().join("original.sqlite")).unwrap();
    original.upsert_cash_pocket(reference()).unwrap();
    let valid: Value = serde_json::from_str(&original.export_legacy_json().unwrap()).unwrap();
    for (index, (field, value)) in [
        ("rateSourceUrl", json!("https://example.com/cdi")),
        ("rateFetchedAt", Value::Null),
        ("rateSource", json!("manual")),
        ("rateDate", Value::Null),
    ]
    .into_iter()
    .enumerate()
    {
        let mut invalid = valid.clone();
        invalid["cash_pockets"][0][field] = value;
        let backup = directory.path().join(format!("invalid-{index}.json"));
        let bytes = serde_json::to_vec_pretty(&invalid).unwrap();
        fs::write(&backup, &bytes).unwrap();
        let restored =
            Repository::open(directory.path().join(format!("restored-{index}.sqlite"))).unwrap();
        assert!(restored.import_legacy_file(&backup).is_err(), "{field}");
        assert!(restored.cash_pockets().unwrap().is_empty(), "{field}");
        assert_eq!(restored.pending_change_count().unwrap(), 0, "{field}");
        assert_eq!(fs::read(backup).unwrap(), bytes, "{field}");
    }
}

#[test]
fn legacy_backup_without_provenance_defaults_to_manual() {
    let directory = TempDir::new().unwrap();
    let original = Repository::open(directory.path().join("source.sqlite")).unwrap();
    let saved = original.upsert_cash_pocket(reference()).unwrap();
    let mut legacy: Value = serde_json::from_str(&original.export_legacy_json().unwrap()).unwrap();
    let pocket = legacy["cash_pockets"][0].as_object_mut().unwrap();
    for key in ["rateSource", "rateSourceUrl", "rateFetchedAt"] {
        pocket.remove(key);
    }
    let backup = directory.path().join("legacy.json");
    fs::write(&backup, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();
    let restored = Repository::open(directory.path().join("restored.sqlite")).unwrap();
    restored.import_legacy_file(backup).unwrap();
    let pocket = &restored.cash_pockets().unwrap()[0];
    assert_eq!(pocket.id, saved.id);
    assert_eq!(pocket.balance, saved.balance);
    assert_eq!(pocket.annual_cdi_rate, saved.annual_cdi_rate);
    assert_eq!(pocket.rate_date, saved.rate_date);
    assert_eq!(pocket.rate_source, "manual");
    assert_eq!(pocket.rate_source_url, None);
    assert_eq!(pocket.rate_fetched_at, None);
}

#[test]
fn schema_three_migration_preserves_cash_history_and_unchanged_outbox() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("portfolio.sqlite");
    let repo = Repository::open(&path).unwrap();
    let mut manual = reference();
    manual.rate_source = None;
    manual.rate_source_url = None;
    manual.rate_fetched_at = None;
    let pocket = repo.upsert_cash_pocket(manual).unwrap();
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
        purchase_date: None,
        price_per_cota: "9.91".into(),
        dividend_per_cota: "0.12".into(),
        dividend_total: Some("1.17".into()),
        notes: "Histórico preservado pela migração".into(),
    })
    .unwrap();
    let snapshot = serde_json::to_value(repo.snapshot().unwrap()).unwrap();
    let export = repo.export_legacy_json().unwrap();
    let mut expected_queue = repo.pending_changes().unwrap();
    for change in &mut expected_queue {
        if change.entity_type == "cashPocket" {
            let payload = change.payload.as_object_mut().unwrap();
            for key in [
                "rateSource",
                "rateSourceUrl",
                "rateFetchedAt",
                "totalDeposits",
                "totalWithdrawals",
                "balanceDate",
            ] {
                payload.remove(key);
            }
        }
    }
    drop(repo);

    // Recreate the exact older table shape only in this disposable fixture.
    let old = Connection::open(&path).unwrap();
    old.execute_batch(
        "BEGIN IMMEDIATE;
         ALTER TABLE cash_pockets DROP COLUMN rate_source;
         ALTER TABLE cash_pockets DROP COLUMN rate_source_url;
         ALTER TABLE cash_pockets DROP COLUMN rate_fetched_at;
         ALTER TABLE cash_pockets DROP COLUMN total_deposits;
         ALTER TABLE cash_pockets DROP COLUMN total_withdrawals;
         ALTER TABLE cash_pockets DROP COLUMN balance_date;
         PRAGMA user_version = 3;",
    )
    .unwrap();
    for change in &expected_queue {
        old.execute(
            "UPDATE sync_outbox SET payload=?1 WHERE id=?2",
            params![serde_json::to_string(&change.payload).unwrap(), change.id],
        )
        .unwrap();
    }
    old.execute_batch("COMMIT;").unwrap();
    assert_eq!(
        old.query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
            .unwrap(),
        3
    );
    drop(old);

    let migrated = Repository::open_existing(&path).unwrap();
    assert_eq!(
        serde_json::to_value(migrated.snapshot().unwrap()).unwrap(),
        snapshot
    );
    assert_eq!(migrated.export_legacy_json().unwrap(), export);
    assert_eq!(migrated.cash_pockets().unwrap(), vec![pocket]);
    let expected_queue = serde_json::to_value(expected_queue).unwrap();
    assert_eq!(outbox(&migrated), expected_queue);
    assert_eq!(migrated.pending_change_count().unwrap(), 3);
    drop(migrated);

    let upgraded = Connection::open(&path).unwrap();
    assert_eq!(
        upgraded
            .query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
            .unwrap(),
        7
    );
    let provenance: (String, Option<String>, Option<String>) = upgraded
        .query_row(
            "SELECT rate_source,rate_source_url,rate_fetched_at FROM cash_pockets",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(provenance, ("manual".into(), None, None));
    drop(upgraded);
    let reopened = Repository::open_existing(path).unwrap();
    assert_eq!(outbox(&reopened), expected_queue);
    assert_eq!(reopened.export_legacy_json().unwrap(), export);
}
