use chrono::Utc;
use ivestfii_core::{
    FundInput, InvestmentProfileInput, RecordInput, Repository, WalletCatalog, WalletInput,
};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::fs;
use tempfile::TempDir;

fn input() -> InvestmentProfileInput {
    InvestmentProfileInput {
        risk_tolerance: "moderate".into(),
        short_term_objective: "Reserva para uma viagem".into(),
        short_term_months: Some(18),
        long_term_objective: "Complementar renda na aposentadoria".into(),
        long_term_months: Some(240),
        liquidity_needs: "Parte do valor precisa estar acessível no mesmo dia".into(),
        emergency_reserve_status: "partial".into(),
        monthly_contribution: Some("1234.5600".into()),
        knowledge_level: Some("beginner".into()),
        constraints: "Sem alavancagem".into(),
        notes: "Dados sintéticos de teste".into(),
    }
}

fn add_history(repo: &Repository) {
    repo.upsert_fund(FundInput {
        id: None,
        ticker: "TEST11".into(),
        name: "Fundo sintético".into(),
        sector: "Teste".into(),
    })
    .unwrap();
    let id = repo.snapshot().unwrap().funds[0].id.clone();
    repo.save_monthly_record(RecordInput {
        id: None,
        fund_id: id,
        month: "2020-01".into(),
        cotas_added: "10".into(),
        purchase_date: None,
        price_per_cota: "9.91".into(),
        dividend_per_cota: "0.12".into(),
        dividend_total: Some("1.17".into()),
        notes: "Histórico não deve mudar".into(),
    })
    .unwrap();
}

fn outbox(repo: &Repository) -> Value {
    serde_json::to_value(repo.pending_changes().unwrap()).unwrap()
}

#[test]
fn holdings_never_create_or_infer_a_profile_and_read_is_financially_read_only() {
    let directory = TempDir::new().unwrap();
    let repo = Repository::open(directory.path().join("portfolio.sqlite")).unwrap();
    assert_eq!(repo.investment_profile().unwrap(), None);
    add_history(&repo);
    let before = repo.export_legacy_json().unwrap();
    let queue = outbox(&repo);
    assert_eq!(repo.investment_profile().unwrap(), None);
    assert_eq!(repo.export_legacy_json().unwrap(), before);
    assert_eq!(outbox(&repo), queue);
    assert!(serde_json::from_str::<Value>(&before)
        .unwrap()
        .get("investment_profile")
        .is_none());
}

#[test]
fn explicit_profile_is_normalized_saved_and_reopened_without_financial_or_outbox_changes() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("portfolio.sqlite");
    let repo = Repository::open(&path).unwrap();
    add_history(&repo);
    let before = serde_json::to_value(repo.snapshot().unwrap()).unwrap();
    let queue = outbox(&repo);
    let mut data = input();
    data.short_term_objective = "  Reserva para uma viagem  ".into();
    let saved = repo.save_investment_profile(data).unwrap();
    assert_eq!(saved.risk_tolerance, "moderate");
    assert_eq!(saved.short_term_objective, "Reserva para uma viagem");
    assert_eq!(saved.monthly_contribution.as_deref(), Some("1234.56"));
    assert_eq!(saved.knowledge_level.as_deref(), Some("beginner"));
    assert_eq!(saved.emergency_reserve_status, "partial");
    assert!(chrono::DateTime::parse_from_rfc3339(&saved.updated_at).is_ok());
    assert_eq!(repo.investment_profile().unwrap(), Some(saved.clone()));
    assert_eq!(
        serde_json::to_value(repo.snapshot().unwrap()).unwrap(),
        before
    );
    assert_eq!(outbox(&repo), queue);
    drop(repo);
    let reopened = Repository::open_existing(path).unwrap();
    assert_eq!(reopened.investment_profile().unwrap(), Some(saved));
    assert_eq!(outbox(&reopened), queue);
}

#[test]
fn unknown_defaults_are_explicit_and_zero_contribution_is_distinct_from_unknown() {
    let directory = TempDir::new().unwrap();
    let repo = Repository::open(directory.path().join("portfolio.sqlite")).unwrap();
    let defaults: InvestmentProfileInput = serde_json::from_value(json!({})).unwrap();
    assert_eq!(defaults, InvestmentProfileInput::default());
    let saved = repo.save_investment_profile(defaults).unwrap();
    assert_eq!(saved.risk_tolerance, "unknown");
    assert_eq!(saved.emergency_reserve_status, "unknown");
    assert_eq!(saved.knowledge_level, None);
    assert_eq!(saved.monthly_contribution, None);
    assert_eq!(saved.short_term_months, None);
    assert!(saved.long_term_objective.is_empty());
    let zero = InvestmentProfileInput {
        monthly_contribution: Some("0.00".into()),
        ..InvestmentProfileInput::default()
    };
    assert_eq!(
        repo.save_investment_profile(zero)
            .unwrap()
            .monthly_contribution
            .as_deref(),
        Some("0")
    );
    assert_eq!(repo.pending_change_count().unwrap(), 0);
}

#[test]
fn profiles_are_isolated_between_wallets_and_unknown_wallets_cannot_be_saved() {
    let directory = TempDir::new().unwrap();
    let catalog = WalletCatalog::open(directory.path()).unwrap();
    let secondary = catalog
        .create(WalletInput {
            id: None,
            name: "Outra carteira".into(),
            institution: "Instituição de teste".into(),
        })
        .unwrap();
    let main = catalog.save_investment_profile(None, input()).unwrap();
    assert_eq!(
        catalog.investment_profile(Some(&secondary.id)).unwrap(),
        None
    );
    let mut other = input();
    other.risk_tolerance = "conservative".into();
    other.notes = "Preferência apenas desta carteira".into();
    let other = catalog
        .save_investment_profile(Some(&secondary.id), other)
        .unwrap();
    assert_eq!(
        catalog.investment_profile(None).unwrap(),
        Some(main.clone())
    );
    assert_eq!(
        catalog.investment_profile(Some(&secondary.id)).unwrap(),
        Some(other)
    );
    for invalid in [
        "../../portfolio.sqlite",
        "not-a-wallet",
        "00000000-0000-0000-0000-000000000000",
    ] {
        assert!(catalog
            .save_investment_profile(Some(invalid), input())
            .is_err());
        assert!(catalog.investment_profile(Some(invalid)).is_err());
    }
    assert_eq!(catalog.investment_profile(None).unwrap(), Some(main));
    assert_eq!(
        catalog
            .repository(None)
            .unwrap()
            .pending_change_count()
            .unwrap(),
        0
    );
    assert_eq!(
        catalog
            .repository(Some(&secondary.id))
            .unwrap()
            .pending_change_count()
            .unwrap(),
        0
    );
}

#[test]
fn invalid_profiles_are_rejected_atomically_and_do_not_touch_existing_records() {
    let directory = TempDir::new().unwrap();
    let repo = Repository::open(directory.path().join("portfolio.sqlite")).unwrap();
    add_history(&repo);
    let saved = repo.save_investment_profile(input()).unwrap();
    let before = repo.export_legacy_json().unwrap();
    let queue = outbox(&repo);
    let mut cases = Vec::new();
    let mut bad = input();
    bad.risk_tolerance = "guaranteed".into();
    cases.push(bad);
    let mut bad = input();
    bad.emergency_reserve_status = "yes".into();
    cases.push(bad);
    let mut bad = input();
    bad.knowledge_level = Some("unknown".into());
    cases.push(bad);
    for months in [0, 1201, u32::MAX] {
        let mut bad = input();
        bad.short_term_months = Some(months);
        cases.push(bad);
        let mut bad = input();
        bad.long_term_months = Some(months);
        cases.push(bad);
    }
    for money in [
        "-0.01",
        "0.001",
        "NaN",
        "Infinity",
        "",
        "79228162514264337593543950336",
        "1e-29",
    ] {
        let mut bad = input();
        bad.monthly_contribution = Some(money.into());
        cases.push(bad);
    }
    let mut bad = input();
    bad.short_term_objective = "á".repeat(501);
    cases.push(bad);
    let mut bad = input();
    bad.long_term_objective = "x".repeat(501);
    cases.push(bad);
    let mut bad = input();
    bad.liquidity_needs = "x".repeat(1001);
    cases.push(bad);
    let mut bad = input();
    bad.constraints = "x".repeat(1001);
    cases.push(bad);
    let mut bad = input();
    bad.notes = "x".repeat(4001);
    cases.push(bad);
    let mut bad = input();
    bad.notes = "texto\0inválido".into();
    cases.push(bad);
    for bad in cases {
        assert!(repo.save_investment_profile(bad).is_err());
        assert_eq!(repo.investment_profile().unwrap(), Some(saved.clone()));
        assert_eq!(repo.export_legacy_json().unwrap(), before);
        assert_eq!(outbox(&repo), queue);
    }
}

#[test]
fn backup_preserves_profile_timestamp_and_finances_losslessly_without_profile_sync() {
    let directory = TempDir::new().unwrap();
    let repo = Repository::open(directory.path().join("source.sqlite")).unwrap();
    add_history(&repo);
    let saved = repo.save_investment_profile(input()).unwrap();
    let exported = repo.export_legacy_json().unwrap();
    let json: Value = serde_json::from_str(&exported).unwrap();
    assert_eq!(json["investment_profile"]["riskTolerance"], "moderate");
    assert_eq!(json["investment_profile"]["monthlyContribution"], "1234.56");
    assert_eq!(json["investment_profile"]["updatedAt"], saved.updated_at);
    let backup = directory.path().join("backup.json");
    fs::write(&backup, &exported).unwrap();
    let restored = Repository::open(directory.path().join("restored.sqlite")).unwrap();
    let receipt = restored.import_legacy_file(&backup).unwrap();
    assert_eq!(restored.investment_profile().unwrap(), Some(saved));
    assert_eq!(restored.export_legacy_json().unwrap(), exported);
    assert_eq!(fs::read_to_string(&backup).unwrap(), exported);
    assert_eq!(fs::read_to_string(receipt.backup_path).unwrap(), exported);
    assert_eq!(restored.pending_change_count().unwrap(), 2);
    assert!(restored
        .pending_changes()
        .unwrap()
        .iter()
        .all(|change| change.entity_type != "investmentProfile"));
    assert!(
        restored
            .import_legacy_file(&backup)
            .unwrap()
            .already_imported
    );
    assert_eq!(restored.pending_change_count().unwrap(), 2);
}

#[test]
fn legacy_import_has_no_inferred_profile_and_preserves_a_previously_entered_local_profile() {
    let directory = TempDir::new().unwrap();
    let backup = directory.path().join("legacy.json");
    let bytes=br#"{"fiis":[{"ticker":"TEST11","name":"Fundo de teste","entries":[{"month":"2020-01","cotas_added":10,"price_per_cota":9.91,"dividend_per_cota":0.12}]}]}"#;
    fs::write(&backup, bytes).unwrap();
    for has_profile in [false, true] {
        let repo = Repository::open(
            directory
                .path()
                .join(format!("target-{has_profile}.sqlite")),
        )
        .unwrap();
        let before = has_profile.then(|| repo.save_investment_profile(input()).unwrap());
        repo.import_legacy_file(&backup).unwrap();
        assert_eq!(repo.investment_profile().unwrap(), before);
        assert_eq!(repo.snapshot().unwrap().funds.len(), 1);
        assert_eq!(repo.pending_change_count().unwrap(), 2);
    }
    assert_eq!(fs::read(backup).unwrap(), bytes);
}

#[test]
fn a_backup_does_not_silently_overwrite_an_existing_local_profile() {
    let directory = TempDir::new().unwrap();
    let source = Repository::open(directory.path().join("source.sqlite")).unwrap();
    source.save_investment_profile(input()).unwrap();
    let exported = source.export_legacy_json().unwrap();
    let path = directory.path().join("backup.json");
    fs::write(&path, &exported).unwrap();
    let target = Repository::open(directory.path().join("target.sqlite")).unwrap();
    let saved = target
        .save_investment_profile(InvestmentProfileInput::default())
        .unwrap();
    let before = target.export_legacy_json().unwrap();
    assert!(target.import_legacy_file(&path).is_err());
    assert_eq!(target.investment_profile().unwrap(), Some(saved));
    assert_eq!(target.export_legacy_json().unwrap(), before);
    assert_eq!(target.pending_change_count().unwrap(), 0);
    assert_eq!(fs::read_to_string(path).unwrap(), exported);
}

#[test]
fn invalid_profile_in_a_backup_cannot_partially_import_finances_or_modify_the_source() {
    let directory = TempDir::new().unwrap();
    let source = Repository::open(directory.path().join("source.sqlite")).unwrap();
    add_history(&source);
    source.save_investment_profile(input()).unwrap();
    let valid: Value = serde_json::from_str(&source.export_legacy_json().unwrap()).unwrap();
    for (index, (field, value)) in [
        ("riskTolerance", json!("other")),
        ("monthlyContribution", json!("-1")),
        ("updatedAt", json!("bad date")),
        (
            "updatedAt",
            json!((Utc::now() + chrono::Duration::days(1)).to_rfc3339()),
        ),
        ("shortTermMonths", json!(0)),
        ("schemaVersion", json!(2)),
    ]
    .into_iter()
    .enumerate()
    {
        let mut invalid = valid.clone();
        invalid["investment_profile"][field] = value;
        let path = directory.path().join(format!("invalid-{index}.json"));
        let bytes = serde_json::to_vec(&invalid).unwrap();
        fs::write(&path, &bytes).unwrap();
        let target =
            Repository::open(directory.path().join(format!("target-{index}.sqlite"))).unwrap();
        assert!(target.import_legacy_file(&path).is_err());
        assert!(target.snapshot().unwrap().funds.is_empty());
        assert_eq!(target.investment_profile().unwrap(), None);
        assert_eq!(target.pending_change_count().unwrap(), 0);
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn schema_five_upgrade_is_additive_without_inventing_a_profile_or_rewriting_financial_history() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("portfolio.sqlite");
    let repo = Repository::open(&path).unwrap();
    add_history(&repo);
    let cash=serde_json::from_value(json!({"name":"Reserva sintética","institution":"Teste","balance":"100.01","cdiPercent":"120","totalDeposits":"90","totalWithdrawals":"0","balanceDate":"2020-01-10"})).unwrap();
    repo.upsert_cash_pocket(cash).unwrap();
    let export = repo.export_legacy_json().unwrap();
    let snapshot = serde_json::to_value(repo.snapshot().unwrap()).unwrap();
    let queue = outbox(&repo);
    drop(repo);
    let old = Connection::open(&path).unwrap();
    old.execute_batch(
        "BEGIN IMMEDIATE; DROP TABLE investment_profile; PRAGMA user_version=5; COMMIT;",
    )
    .unwrap();
    drop(old);
    let upgraded = Repository::open_existing(&path).unwrap();
    assert_eq!(upgraded.investment_profile().unwrap(), None);
    assert_eq!(upgraded.export_legacy_json().unwrap(), export);
    assert_eq!(
        serde_json::to_value(upgraded.snapshot().unwrap()).unwrap(),
        snapshot
    );
    assert_eq!(outbox(&upgraded), queue);
    drop(upgraded);
    let db = Connection::open(&path).unwrap();
    let version: u32 = db
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 8);
    drop(db);
    let reopened = Repository::open_existing(path).unwrap();
    assert_eq!(reopened.export_legacy_json().unwrap(), export);
    assert_eq!(outbox(&reopened), queue);
}
