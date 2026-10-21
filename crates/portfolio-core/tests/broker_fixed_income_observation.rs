use ivestfii_core::Repository;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

// Synthetic amounts and product names; no personal positions or source images.
fn position() -> Value {
    json!({
        "referenceDate":"2025-06-30", "institution":"Corretora de teste",
        "importedAt":"2025-06-30T12:00:00Z", "sourcePath":"synthetic-position.json",
        "sourceHash":"b".repeat(64),
        "overview":{"updatedTime":"09:00","capturedTime":"09:01", "totalWithCash":"160",
            "invested":"150", "cash":"10", "listedFunds":"100", "fixedIncome":"50", "pendingIncome":"0"},
        "listedFunds":{"capturedTime":"09:02","marketValue":"100","costBasis":"100",
            "unrealizedResult":"0","dailyResult":"1","holdings":[
                {"ticker":"TEST11","shares":"10","averagePrice":"10","price":"10",
                 "marketValue":"100","unrealizedResult":"0","costBasis":"100"}]},
        "fixedIncome":{"name":"Produto anterior...","nameTruncated":true,"balance":"50",
            "netBalance":"49","reportedEarnings":"5","return12Months":"10",
            "redemptionConversion":"D+0","redemptionSettlement":"D+1"},
        "warnings":["Observação original preservada."]
    })
}

fn supplement() -> Value {
    json!({
        "type":"broker_fixed_income_observation", "institution":"Corretora de teste",
        "expectedPositionSourceHash":"b".repeat(64),
        "observation":{
            "referenceDate":"2025-07-05","name":"Fundo sintético pós-fixado",
            "category":"Renda fixa","riskLevel":"Baixo 1", "balance":"234.00",
            "invested":"200.00","totalEarnings":"34.00","earnings12Months":"20.00",
            "return12Months":"12.34","returnReferenceDate":"2025-07-01",
            "contributions":[
                {"applicationDate":"2024-01-10","invested":"100","currentValue":"110","earnings":"10","returnPercent":"10"},
                {"applicationDate":"2024-06-10","invested":"80","currentValue":"100","earnings":"20","returnPercent":"25"},
                {"applicationDate":"2025-01-10","invested":"20","currentValue":"24","earnings":"4","returnPercent":"20"}
            ],
            "sources":[{"fileName":"synthetic-observation.png","sha256":"a".repeat(64)}]
        }
    })
}

fn write_json(root: &Path, name: &str, value: &Value) -> PathBuf {
    let path = root.join(name);
    fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    path
}

fn setup() -> (TempDir, Repository) {
    let root = TempDir::new().unwrap();
    let repo = Repository::open(root.path().join("portfolio.sqlite")).unwrap();
    let ledger = json!({"fiis":[{"ticker":"TEST11","name":"Teste","sector":"Teste","entries":[
        {"month":"2024-01","cotas_added":1,"price_per_cota":10,"dividend_per_cota":0,"dividend_total":1,"notes":"Teste"}
    ]}]});
    repo.import_legacy_file(write_json(root.path(), "ledger.json", &ledger))
        .unwrap();
    repo.import_broker_position_file(write_json(root.path(), "position.json", &position()))
        .unwrap();
    (root, repo)
}

#[test]
fn supplement_only_changes_its_dated_observation_and_preserves_exact_source_bytes() {
    let (root, repo) = setup();
    let before = serde_json::to_value(repo.snapshot().unwrap()).unwrap();
    let pending = repo.pending_change_count().unwrap();
    let path = write_json(root.path(), "supplement.json", &supplement());
    let original = fs::read(&path).unwrap();
    let hash = format!("{:x}", Sha256::digest(&original));
    let saved = repo.import_broker_position_file(&path).unwrap();
    let observation = saved.fixed_income.latest_observation.as_ref().unwrap();
    assert_eq!(observation.reference_date, "2025-07-05");
    assert_eq!(observation.return_reference_date, "2025-07-01");
    assert_eq!(observation.balance, "234");
    assert_eq!(observation.invested, "200");
    assert_eq!(observation.total_earnings, "34");
    assert_eq!(observation.contributions.len(), 3);
    assert_eq!(observation.source_hash, hash);
    assert_eq!(
        observation.source_path,
        fs::canonicalize(&path).unwrap().to_string_lossy()
    );
    assert!(chrono::DateTime::parse_from_rfc3339(&observation.imported_at).is_ok());
    let backup = root
        .path()
        .join("backups")
        .join(format!("broker-position-{hash}.json"));
    assert_eq!(fs::read(&backup).unwrap(), original);
    assert_eq!(fs::read(&path).unwrap(), original);
    let mut after = serde_json::to_value(repo.snapshot().unwrap()).unwrap();
    after["brokerPosition"]["fixedIncome"]
        .as_object_mut()
        .unwrap()
        .remove("latestObservation");
    assert_eq!(after, before);
    assert_eq!(repo.pending_change_count().unwrap(), pending);
}

#[test]
fn repeated_files_never_revert_newer_observations_and_survive_restart() {
    let (root, repo) = setup();
    let first_path = write_json(root.path(), "first.json", &supplement());
    let first = repo.import_broker_position_file(&first_path).unwrap();
    assert_eq!(
        repo.import_broker_position_file(&first_path).unwrap(),
        first
    );
    let mut newer = supplement();
    newer["observation"]["referenceDate"] = json!("2025-07-06");
    let second = repo
        .import_broker_position_file(write_json(root.path(), "second.json", &newer))
        .unwrap();
    assert_eq!(
        repo.import_broker_position_file(&first_path).unwrap(),
        second
    );
    assert_eq!(
        repo.import_broker_position_file(root.path().join("position.json"))
            .unwrap(),
        second
    );
    let mut old_new_file = supplement();
    old_new_file["observation"]["name"] = json!("Outra descrição antiga");
    assert!(repo
        .import_broker_position_file(write_json(root.path(), "older.json", &old_new_file))
        .is_err());
    assert_eq!(
        repo.snapshot().unwrap().broker_position,
        Some(second.clone())
    );
    drop(repo);
    let reopened = Repository::open(root.path().join("portfolio.sqlite")).unwrap();
    assert_eq!(reopened.snapshot().unwrap().broker_position, Some(second));
}

#[test]
fn metadata_is_derived_from_the_imported_file_and_same_day_corrections_are_supported() {
    let (root, repo) = setup();
    let first = repo
        .import_broker_position_file(write_json(root.path(), "first.json", &supplement()))
        .unwrap();
    let mut correction = supplement();
    correction["observation"]["name"] = json!("Descrição corrigida do produto sintético");
    correction["observation"]["importedAt"] = json!("2999-01-01T00:00:00Z");
    correction["observation"]["sourcePath"] = json!("forged-path");
    correction["observation"]["sourceHash"] = json!("forged-hash");
    let path = write_json(root.path(), "correction.json", &correction);
    let corrected = repo.import_broker_position_file(&path).unwrap();
    let observation = corrected.fixed_income.latest_observation.unwrap();
    assert_eq!(observation.name, "Descrição corrigida do produto sintético");
    assert_ne!(observation.imported_at, "2999-01-01T00:00:00Z");
    assert_eq!(
        observation.source_path,
        fs::canonicalize(path).unwrap().to_string_lossy()
    );
    assert_eq!(observation.source_hash.len(), 64);
    assert_ne!(
        observation.imported_at,
        first.fixed_income.latest_observation.unwrap().imported_at
    );
}

#[test]
fn backup_round_trip_preserves_observation_and_old_backups_need_no_new_field() {
    let (root, repo) = setup();
    assert!(repo
        .snapshot()
        .unwrap()
        .broker_position
        .unwrap()
        .fixed_income
        .latest_observation
        .is_none());
    let saved = repo
        .import_broker_position_file(write_json(root.path(), "supplement.json", &supplement()))
        .unwrap();
    let exported = repo.export_legacy_json().unwrap();
    let backup = root.path().join("full-backup.json");
    fs::write(&backup, &exported).unwrap();
    let restored = Repository::open(root.path().join("restored.sqlite")).unwrap();
    restored.import_legacy_file(&backup).unwrap();
    assert_eq!(restored.snapshot().unwrap().broker_position, Some(saved));
    assert_eq!(restored.export_legacy_json().unwrap(), exported);
}

#[test]
fn incompatible_target_or_missing_complete_position_cannot_receive_a_supplement() {
    let (root, repo) = setup();
    let before = repo.export_legacy_json().unwrap();
    for (index, (pointer, value)) in [
        ("/institution", json!("Outra corretora")),
        ("/expectedPositionSourceHash", json!("c".repeat(64))),
        ("/expectedPositionSourceHash", json!("invalid")),
    ]
    .into_iter()
    .enumerate()
    {
        let mut invalid = supplement();
        *invalid.pointer_mut(pointer).unwrap() = value;
        assert!(repo
            .import_broker_position_file(write_json(
                root.path(),
                &format!("bad-target-{index}.json"),
                &invalid
            ))
            .is_err());
        assert_eq!(repo.export_legacy_json().unwrap(), before);
    }
    let empty = Repository::open(root.path().join("empty.sqlite")).unwrap();
    assert!(empty
        .import_broker_position_file(write_json(root.path(), "no-position.json", &supplement()))
        .is_err());
    assert!(empty.snapshot().unwrap().broker_position.is_none());
}

#[test]
fn invalid_dates_amounts_percentages_and_sources_fail_atomically() {
    let (root, repo) = setup();
    repo.import_broker_position_file(write_json(root.path(), "valid.json", &supplement()))
        .unwrap();
    let before = repo.export_legacy_json().unwrap();
    for (index, (pointer, value)) in [
        ("/observation/referenceDate", json!("2999-01-01")),
        ("/observation/referenceDate", json!("2025-02-30")),
        ("/observation/referenceDate", json!("2025-06-29")),
        ("/observation/returnReferenceDate", json!("2025-07-06")),
        (
            "/observation/contributions/0/applicationDate",
            json!("2025-07-06"),
        ),
        ("/observation/balance", json!("235")),
        ("/observation/invested", json!("201")),
        ("/observation/totalEarnings", json!("35")),
        ("/observation/earnings12Months", json!("1.001")),
        ("/observation/contributions/0/currentValue", json!("111")),
        ("/observation/contributions/0/invested", json!("0")),
        ("/observation/contributions/0/returnPercent", json!("11")),
        (
            "/observation/contributions/0/returnPercent",
            json!("10.001"),
        ),
        ("/observation/contributions", json!([])),
        ("/observation/sources", json!([])),
        ("/observation/sources/0/fileName", json!("../source.png")),
        ("/observation/sources/0/fileName", json!("C:\\source.png")),
        ("/observation/sources/0/sha256", json!("invalid")),
        ("/observation/name", json!(" ")),
    ]
    .into_iter()
    .enumerate()
    {
        let mut invalid = supplement();
        *invalid.pointer_mut(pointer).unwrap() = value;
        assert!(
            repo.import_broker_position_file(write_json(
                root.path(),
                &format!("invalid-{index}.json"),
                &invalid
            ))
            .is_err(),
            "accepted {pointer}"
        );
        assert_eq!(
            repo.export_legacy_json().unwrap(),
            before,
            "changed state after {pointer}"
        );
    }
}

#[test]
fn rounded_contribution_percentages_and_losses_are_valid_but_financial_totals_stay_exact() {
    let (root, repo) = setup();
    let mut input = supplement();
    input["observation"]["invested"] = json!("3");
    input["observation"]["balance"] = json!("2");
    input["observation"]["totalEarnings"] = json!("-1");
    input["observation"]["earnings12Months"] = json!("-0.50");
    input["observation"]["return12Months"] = json!("-20");
    input["observation"]["contributions"] = json!([
        {"applicationDate":"2024-01-10","invested":"3","currentValue":"2","earnings":"-1","returnPercent":"-33.33"}
    ]);
    let saved = repo
        .import_broker_position_file(write_json(root.path(), "loss.json", &input))
        .unwrap();
    let observation = saved.fixed_income.latest_observation.unwrap();
    assert_eq!(observation.total_earnings, "-1");
    assert_eq!(observation.contributions[0].return_percent, "-33.33");
    assert_eq!(repo.snapshot().unwrap().totals.dividends_received, "1");
}

#[test]
fn tampered_source_backup_and_tampered_embedded_observation_are_rejected() {
    let (root, repo) = setup();
    let path = write_json(root.path(), "supplement.json", &supplement());
    let saved = repo.import_broker_position_file(&path).unwrap();
    let before = repo.export_legacy_json().unwrap();
    let hash = &saved
        .fixed_income
        .latest_observation
        .as_ref()
        .unwrap()
        .source_hash;
    fs::write(
        root.path()
            .join("backups")
            .join(format!("broker-position-{hash}.json")),
        b"tampered",
    )
    .unwrap();
    assert!(repo.import_broker_position_file(&path).is_err());
    assert_eq!(repo.export_legacy_json().unwrap(), before);
    let mut forged = serde_json::to_value(saved).unwrap();
    forged["fixedIncome"]["latestObservation"]["totalEarnings"] = json!("999");
    assert!(repo
        .import_broker_position_file(write_json(root.path(), "forged.json", &forged))
        .is_err());
    assert_eq!(repo.export_legacy_json().unwrap(), before);
}
