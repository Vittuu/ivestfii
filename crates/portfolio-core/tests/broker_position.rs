use ivestfii_core::Repository;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

// Entirely synthetic values: overview and detailed listing intentionally have
// distinct capture totals, as they are independent observations.
fn position_fixture() -> Value {
    json!({
        "versao_formato":1,"moeda":"BRL","instituicao":"Corretora exemplo","data_referencia":"2025-06-30",
        "panorama_carteira":{
            "data_atualizacao_exibida":"2025-06-30","horario_atualizacao_exibido":"09:00","horario_captura_exibido":"09:01",
            "total_investido_mais_conta_investimento":"160.00","total_investido_em_classes":"150.00","saldo_conta_investimento_nacional":"10.00",
            "quantidade_classes":3,"classes":[
                {"nome_exibido":"Fundos Listados","valor":"99.00"},
                {"nome_exibido":"Pós-Fixado","valor":"50.00"},
                {"nome_exibido":"Proventos","valor":"1.00"}
            ],"observacoes":["Proventos do panorama não são créditos recebidos."]
        },
        "posicao_fundos_listados":{
            "horario_captura_exibido":"09:02","data_atualizacao_explicita_na_tela":null,
            "quantidade_ativos":1,"valor_aplicado_exibido":"110.00","total_em_bolsa_exibido":"101.00",
            "resultado_total_exibido":"-9.00","resultado_do_dia_exibido":"2.00",
            "ativos":[{"ticker":"TEST11","quantidade_cotas":10,"preco_medio_exibido":"11.00","preco_atual_exibido":"10.10",
                "posicao_exibida":"101.00","resultado_total_exibido":"-9.00","custo_total_inferido":"110.00"}]
        },
        "posicao_pos_fixado":{
            "data_atualizacao_exibida":"2025-06-30","quantidade_ativos":1,"total_investido_exibido":"50.00",
            "ativo":{"nome_exibido":"Fundo exemplo...","nome_exibido_esta_truncado":true,"saldo":"50.00","saldo_liquido":"49.00",
                "rendimento_exibido":"5.00","rentabilidade_12_meses_percentual":"12.50",
                "cotizacao_de_resgate":"D+0","liquidacao_de_resgate":"D+1"},
            "observacoes":["Período do rendimento não informado."]
        },
        "divergencias_e_limites":[{"tratamento":"Preservar custos históricos."}]
    })
}

fn write_json(dir: &Path, name: &str, value: &Value) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    path
}

fn ledger_fixture() -> Value {
    json!({"fiis":[{"ticker":"TEST11","name":"Teste","sector":"Papel","entries":[
        {"month":"2024-12","cotas_added":10,"price_per_cota":10,"dividend_per_cota":0.1,"dividend_total":1,"notes":"Histórico independente"}
    ]}]})
}

#[test]
fn imports_separate_totals_without_changing_the_ledger_and_backs_up_exact_bytes() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    repo.import_legacy_file(write_json(dir.path(), "ledger.json", &ledger_fixture()))
        .unwrap();
    let before = serde_json::to_value(repo.snapshot().unwrap()).unwrap();
    let pending = repo.pending_change_count().unwrap();
    let path = write_json(dir.path(), "position.json", &position_fixture());
    let original = fs::read(&path).unwrap();
    let position = repo.import_broker_position_file(&path).unwrap();
    assert_eq!(position.overview.total_with_cash, "160");
    assert_eq!(position.overview.listed_funds, "99");
    assert_eq!(position.listed_funds.market_value, "101");
    assert_eq!(position.fixed_income.reported_earnings, "5");
    assert_eq!(position.fixed_income.return12_months, "12.5");
    assert!(position.fixed_income.name_truncated);
    assert!(position
        .warnings
        .iter()
        .any(|w| w.contains("horários diferentes")));
    let hash = format!("{:x}", Sha256::digest(&original));
    assert_eq!(position.source_hash, hash);
    assert_eq!(
        fs::read(
            dir.path()
                .join("backups")
                .join(format!("broker-position-{hash}.json"))
        )
        .unwrap(),
        original
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    let after = serde_json::to_value(repo.snapshot().unwrap()).unwrap();
    assert_eq!(before["funds"], after["funds"]);
    assert_eq!(before["totals"], after["totals"]);
    assert_eq!(before["recordCount"], after["recordCount"]);
    assert_eq!(repo.pending_change_count().unwrap(), pending);
    assert_eq!(
        after["brokerPosition"]["fixedIncome"]["return12Months"],
        "12.5"
    );
}

#[test]
fn import_is_idempotent_survives_restart_and_never_resets_a_newer_import() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("portfolio.sqlite");
    let path = write_json(dir.path(), "first.json", &position_fixture());
    let repo = Repository::open(&database).unwrap();
    let first = repo.import_broker_position_file(&path).unwrap();
    assert_eq!(repo.import_broker_position_file(&path).unwrap(), first);
    assert_eq!(fs::read_dir(dir.path().join("backups")).unwrap().count(), 1);
    drop(repo);
    let repo = Repository::open(database).unwrap();
    assert_eq!(
        repo.snapshot().unwrap().broker_position,
        Some(first.clone())
    );
    let mut next = position_fixture();
    next["posicao_fundos_listados"]["total_em_bolsa_exibido"] = json!("102");
    next["posicao_fundos_listados"]["resultado_total_exibido"] = json!("-8");
    next["posicao_fundos_listados"]["ativos"][0]["preco_atual_exibido"] = json!("10.2");
    next["posicao_fundos_listados"]["ativos"][0]["posicao_exibida"] = json!("102");
    next["posicao_fundos_listados"]["ativos"][0]["resultado_total_exibido"] = json!("-8");
    let second = repo
        .import_broker_position_file(write_json(dir.path(), "second.json", &next))
        .unwrap();
    assert_ne!(first.source_hash, second.source_hash);
    assert_eq!(repo.import_broker_position_file(path).unwrap(), second);
    assert_eq!(repo.snapshot().unwrap().broker_position, Some(second));
}

#[test]
fn combined_export_round_trips_position_and_history_atomically() {
    let dir = TempDir::new().unwrap();
    let source = Repository::open(dir.path().join("first.sqlite")).unwrap();
    source
        .import_legacy_file(write_json(dir.path(), "ledger.json", &ledger_fixture()))
        .unwrap();
    let position = source
        .import_broker_position_file(write_json(dir.path(), "position.json", &position_fixture()))
        .unwrap();
    let export = source.export_legacy_json().unwrap();
    let value: Value = serde_json::from_str(&export).unwrap();
    assert_eq!(value["broker_position"]["sourceHash"], position.source_hash);
    let backup = dir.path().join("combined.json");
    fs::write(&backup, &export).unwrap();
    let target = Repository::open(dir.path().join("second.sqlite")).unwrap();
    target.import_legacy_file(&backup).unwrap();
    let snapshot = target.snapshot().unwrap();
    assert_eq!(snapshot.broker_position, Some(position.clone()));
    assert_eq!(snapshot.record_count, 1);
    assert_eq!(snapshot.totals.invested, "100");
    assert_eq!(snapshot.totals.dividends_received, "1");
    assert_eq!(target.export_legacy_json().unwrap(), export);
    assert!(target.import_legacy_file(&backup).unwrap().already_imported);
    let position_only = Repository::open(dir.path().join("position-only.sqlite")).unwrap();
    assert_eq!(
        position_only.import_broker_position_file(&backup).unwrap(),
        position
    );
    assert!(position_only.snapshot().unwrap().funds.is_empty());
}

#[test]
fn invalid_position_fields_cannot_corrupt_a_previous_position_or_history() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    repo.import_legacy_file(write_json(dir.path(), "ledger.json", &ledger_fixture()))
        .unwrap();
    repo.import_broker_position_file(write_json(dir.path(), "position.json", &position_fixture()))
        .unwrap();
    let before = repo.export_legacy_json().unwrap();
    for (index, (pointer, value)) in [
        ("/data_referencia", json!("2025-02-30")),
        (
            "/panorama_carteira/data_atualizacao_exibida",
            json!("2025-06-29"),
        ),
        ("/panorama_carteira/horario_captura_exibido", json!("25:00")),
        (
            "/panorama_carteira/total_investido_mais_conta_investimento",
            json!("161"),
        ),
        ("/panorama_carteira/classes/0/valor", json!("98")),
        (
            "/posicao_fundos_listados/total_em_bolsa_exibido",
            json!("160"),
        ),
        (
            "/posicao_fundos_listados/ativos/0/quantidade_cotas",
            json!(-10),
        ),
        (
            "/posicao_fundos_listados/ativos/0/preco_atual_exibido",
            json!("10.11"),
        ),
        (
            "/posicao_fundos_listados/ativos/0/preco_medio_exibido",
            json!("20"),
        ),
        ("/posicao_pos_fixado/ativo/saldo_liquido", json!("51")),
        (
            "/posicao_pos_fixado/ativo/saldo",
            json!("0.1234567890123456789012345678901"),
        ),
        (
            "/posicao_pos_fixado/ativo/nome_exibido_esta_truncado",
            json!("yes"),
        ),
        ("/posicao_pos_fixado/quantidade_ativos", json!(2)),
    ]
    .into_iter()
    .enumerate()
    {
        let mut invalid = position_fixture();
        *invalid.pointer_mut(pointer).unwrap() = value;
        assert!(
            repo.import_broker_position_file(write_json(
                dir.path(),
                &format!("invalid-{index}.json"),
                &invalid
            ))
            .is_err(),
            "accepted {pointer}"
        );
        assert_eq!(repo.export_legacy_json().unwrap(), before);
    }
    let mut duplicate = position_fixture();
    let holding = duplicate["posicao_fundos_listados"]["ativos"][0].clone();
    duplicate["posicao_fundos_listados"]["ativos"]
        .as_array_mut()
        .unwrap()
        .push(holding);
    duplicate["posicao_fundos_listados"]["quantidade_ativos"] = json!(2);
    assert!(repo
        .import_broker_position_file(write_json(dir.path(), "duplicate.json", &duplicate))
        .is_err());
    assert_eq!(repo.export_legacy_json().unwrap(), before);
}

#[test]
fn invalid_embedded_position_prevents_partial_history_import() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    let previous = repo
        .import_broker_position_file(write_json(dir.path(), "position.json", &position_fixture()))
        .unwrap();
    let mut combined = ledger_fixture();
    combined["broker_position"] = serde_json::to_value(&previous).unwrap();
    combined["broker_position"]["overview"]["totalWithCash"] = json!("999");
    assert!(repo
        .import_legacy_file(write_json(dir.path(), "invalid-combined.json", &combined))
        .is_err());
    let snapshot = repo.snapshot().unwrap();
    assert_eq!(snapshot.broker_position, Some(previous));
    assert!(snapshot.funds.is_empty());
    assert_eq!(repo.pending_change_count().unwrap(), 0);
}

#[test]
fn sqlite_failure_rolls_back_both_history_and_embedded_position() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("portfolio.sqlite");
    let repo = Repository::open(&database).unwrap();
    let mut combined = ledger_fixture();
    let position = repo
        .import_broker_position_file(write_json(dir.path(), "position.json", &position_fixture()))
        .unwrap();
    combined["broker_position"] = serde_json::to_value(position.clone()).unwrap();
    combined["broker_position"]["institution"] = json!("Outra corretora exemplo");
    // Fail after the funds, records, outbox, and position have been written.
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_receipt BEFORE INSERT ON legacy_imports BEGIN SELECT RAISE(ABORT, 'test failure'); END;").unwrap();
    assert!(repo
        .import_legacy_file(write_json(dir.path(), "combined.json", &combined))
        .is_err());
    assert_eq!(repo.snapshot().unwrap().broker_position, Some(position));
    assert!(repo.snapshot().unwrap().funds.is_empty());
    assert_eq!(repo.pending_change_count().unwrap(), 0);
}

#[test]
fn a_tampered_existing_backup_blocks_import_without_modifying_the_position() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    let source = write_json(dir.path(), "position.json", &position_fixture());
    let position = repo.import_broker_position_file(&source).unwrap();
    let backup = dir
        .path()
        .join("backups")
        .join(format!("broker-position-{}.json", position.source_hash));
    fs::write(backup, b"tampered").unwrap();
    assert!(repo.import_broker_position_file(source).is_err());
    assert_eq!(repo.snapshot().unwrap().broker_position, Some(position));
}

#[test]
fn old_schema_one_database_and_legacy_files_work_without_a_position() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("portfolio.sqlite");
    let repo = Repository::open(&database).unwrap();
    repo.import_legacy_file(write_json(dir.path(), "ledger.json", &ledger_fixture()))
        .unwrap();
    let before = repo.export_legacy_json().unwrap();
    let pending = repo.pending_change_count().unwrap();
    drop(repo);
    // Reproduce an actual schema-one database, including its existing history.
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch("DROP TABLE broker_position; DROP TABLE broker_position_imports;")
        .unwrap();
    connection.pragma_update(None, "user_version", 1).unwrap();
    drop(connection);
    let repo = Repository::open(database).unwrap();
    assert_eq!(repo.export_legacy_json().unwrap(), before);
    assert_eq!(repo.pending_change_count().unwrap(), pending);
    let snapshot = serde_json::to_value(repo.snapshot().unwrap()).unwrap();
    assert!(snapshot["brokerPosition"].is_null());
    let export: Value = serde_json::from_str(&repo.export_legacy_json().unwrap()).unwrap();
    assert!(export.get("broker_position").is_none());
}
