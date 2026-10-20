use ivestfii_core::{
    FundInput, RecordInput, Repository, WalletCatalog, WalletInput, DEFAULT_WALLET_ID,
};
use std::fs;
use tempfile::TempDir;

fn wallet_input(name: &str, institution: &str) -> WalletInput {
    WalletInput {
        id: None,
        name: name.to_owned(),
        institution: institution.to_owned(),
    }
}

fn add_position(repository: &Repository, shares: &str, price: &str) -> String {
    repository
        .upsert_fund(FundInput {
            id: None,
            ticker: "TEST11".into(),
            name: "Fundo de teste".into(),
            sector: "Papel".into(),
        })
        .unwrap();
    let fund_id = repository.snapshot().unwrap().funds[0].id.clone();
    repository
        .save_monthly_record(RecordInput {
            id: None,
            fund_id: fund_id.clone(),
            month: "2026-01".into(),
            cotas_added: shares.into(),
            purchase_date: None,
            price_per_cota: price.into(),
            dividend_per_cota: "0.1".into(),
            dividend_total: None,
            notes: "Registro de teste".into(),
        })
        .unwrap();
    fund_id
}

#[test]
fn first_catalog_preserves_existing_default_ledger_and_source() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("original.json");
    let original = br#"{"fiis":[{"ticker":"TEST11","name":"Original","sector":"Papel","entries":[{"month":"2026-01","cotas_added":7,"price_per_cota":12.35,"dividend_per_cota":0.1,"dividend_total":0.69,"notes":"Conferido"}]}]}"#;
    fs::write(&source, original).unwrap();
    let repository = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
    let receipt = repository.import_legacy_file(&source).unwrap();
    let before = repository.export_legacy_json().unwrap();
    let pending = repository.pending_change_count().unwrap();
    drop(repository);

    let catalog = WalletCatalog::open(dir.path()).unwrap();
    assert_eq!(catalog.default_id(), DEFAULT_WALLET_ID);
    let wallets = catalog.list().unwrap();
    assert_eq!(wallets.len(), 1);
    assert_eq!(wallets[0].id, DEFAULT_WALLET_ID);
    assert!(wallets[0].is_default);
    assert_eq!(wallets[0].name, "Carteira principal");
    assert_eq!(wallets[0].institution, "");
    let repository = catalog.repository(None).unwrap();
    assert_eq!(repository.export_legacy_json().unwrap(), before);
    assert_eq!(repository.pending_change_count().unwrap(), pending);
    assert_eq!(fs::read(source).unwrap(), original);
    assert_eq!(fs::read(receipt.backup_path).unwrap(), original);
    drop(repository);
    drop(catalog);

    let reopened = WalletCatalog::open(dir.path()).unwrap();
    assert_eq!(reopened.list().unwrap(), wallets);
    assert_eq!(
        reopened
            .repository(Some(DEFAULT_WALLET_ID))
            .unwrap()
            .export_legacy_json()
            .unwrap(),
        before
    );
    assert!(!dir.path().join("wallets").exists());
}

#[test]
fn same_ticker_has_independent_holdings_and_records_in_each_wallet() {
    let dir = TempDir::new().unwrap();
    let catalog = WalletCatalog::open(dir.path()).unwrap();
    let main = catalog.repository(None).unwrap();
    let main_fund_id = add_position(&main, "3", "10");
    let nubank = catalog
        .create(wallet_input("FIIs no Nubank", "Nubank"))
        .unwrap();
    assert!(!nubank.is_default);
    assert_ne!(nubank.id, DEFAULT_WALLET_ID);
    let secondary = catalog.repository(Some(&nubank.id)).unwrap();
    assert_eq!(secondary.snapshot().unwrap().record_count, 0);
    let secondary_fund_id = add_position(&secondary, "8", "9.5");
    assert_ne!(main_fund_id, secondary_fund_id);
    assert_eq!(main.snapshot().unwrap().totals.invested, "30");
    assert_eq!(secondary.snapshot().unwrap().totals.invested, "76");
    assert_eq!(main.snapshot().unwrap().funds[0].total_cotas, "3");
    assert_eq!(secondary.snapshot().unwrap().funds[0].total_cotas, "8");
    assert!(dir
        .path()
        .join("wallets")
        .join(&nubank.id)
        .join("portfolio.sqlite")
        .is_file());
    assert!(secondary
        .save_monthly_record(RecordInput {
            id: None,
            fund_id: main_fund_id,
            month: "2026-02".into(),
            cotas_added: "1".into(),
            purchase_date: None,
            price_per_cota: "10".into(),
            dividend_per_cota: "0.1".into(),
            dividend_total: None,
            notes: String::new(),
        })
        .is_err());
    assert_eq!(main.snapshot().unwrap().record_count, 1);
    assert_eq!(secondary.snapshot().unwrap().record_count, 1);
}

#[test]
fn rename_metadata_keeps_default_identity_and_all_positions() {
    let dir = TempDir::new().unwrap();
    let catalog = WalletCatalog::open(dir.path()).unwrap();
    let repository = catalog.repository(None).unwrap();
    add_position(&repository, "5", "7");
    let before = repository.export_legacy_json().unwrap();
    let created_at = catalog.get(None).unwrap().created_at;
    let renamed = catalog
        .update(WalletInput {
            id: Some(DEFAULT_WALLET_ID.to_owned()),
            name: "  FIIs de longo prazo  ".into(),
            institution: "  Nubank  ".into(),
        })
        .unwrap();
    assert_eq!(renamed.name, "FIIs de longo prazo");
    assert_eq!(renamed.institution, "Nubank");
    assert_eq!(renamed.id, DEFAULT_WALLET_ID);
    assert_eq!(renamed.created_at, created_at);
    assert!(renamed.is_default);
    assert_eq!(
        catalog
            .repository(None)
            .unwrap()
            .export_legacy_json()
            .unwrap(),
        before
    );
    drop(catalog);
    let reopened = WalletCatalog::open(dir.path()).unwrap();
    assert_eq!(reopened.get(None).unwrap(), renamed);
}

#[test]
fn missing_known_database_is_reported_without_creating_an_empty_replacement() {
    let dir = TempDir::new().unwrap();
    let catalog = WalletCatalog::open(dir.path()).unwrap();
    let wallet = catalog
        .create(wallet_input("Outra carteira", "Nubank"))
        .unwrap();
    let path = dir
        .path()
        .join("wallets")
        .join(&wallet.id)
        .join("portfolio.sqlite");
    fs::remove_file(&path).unwrap();
    assert!(catalog.repository(Some(&wallet.id)).is_err());
    assert!(!path.exists());
    assert_eq!(catalog.list().unwrap().len(), 2);
    let default_path = dir.path().join("portfolio.sqlite");
    fs::remove_file(&default_path).unwrap();
    drop(catalog);

    let reopened = WalletCatalog::open(dir.path()).unwrap();
    assert_eq!(reopened.list().unwrap().len(), 2);
    assert!(reopened.repository(None).is_err());
    assert!(!default_path.exists());
}

#[test]
fn invalid_or_unknown_identifiers_never_create_paths_or_adopt_databases() {
    let dir = TempDir::new().unwrap();
    let catalog = WalletCatalog::open(dir.path()).unwrap();
    for id in [
        "../outside",
        "..\\outside",
        "",
        "principal/../other",
        "C:\\outside",
    ] {
        assert!(catalog.repository(Some(id)).is_err());
        assert!(catalog.get(Some(id)).is_err());
    }
    let unknown = "64d79c50-f285-4f1a-a07f-11e9b6434614";
    assert!(catalog.repository(Some(unknown)).is_err());
    assert!(!dir.path().join("wallets").exists());
    assert!(catalog
        .update(WalletInput {
            id: Some(unknown.into()),
            name: "Unknown".into(),
            institution: String::new(),
        })
        .is_err());
    assert_eq!(catalog.list().unwrap().len(), 1);
}

#[test]
fn invalid_metadata_and_create_with_id_leave_no_new_wallet_database() {
    let dir = TempDir::new().unwrap();
    let catalog = WalletCatalog::open(dir.path()).unwrap();
    for (name, institution) in [
        (" ", "Nubank"),
        ("Nome\nquebrado", "Nubank"),
        ("Carteira", "Inst\tinválida"),
    ] {
        assert!(catalog.create(wallet_input(name, institution)).is_err());
    }
    assert!(catalog.create(wallet_input(&"x".repeat(121), "")).is_err());
    assert!(catalog
        .create(WalletInput {
            id: Some(DEFAULT_WALLET_ID.to_owned()),
            name: "Existing".into(),
            institution: String::new(),
        })
        .is_err());
    assert_eq!(catalog.list().unwrap().len(), 1);
    assert!(!dir.path().join("wallets").exists());
}

#[test]
fn future_catalog_version_is_rejected_without_schema_downgrade() {
    let dir = TempDir::new().unwrap();
    let catalog = WalletCatalog::open(dir.path()).unwrap();
    drop(catalog);
    let connection = rusqlite::Connection::open(dir.path().join("wallets.sqlite")).unwrap();
    connection.pragma_update(None, "user_version", 99).unwrap();
    drop(connection);
    assert!(WalletCatalog::open(dir.path()).is_err());
    let connection = rusqlite::Connection::open(dir.path().join("wallets.sqlite")).unwrap();
    let version: u32 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 99);
}

#[test]
fn failed_catalog_insert_cleans_only_new_wallet_files() {
    let dir = TempDir::new().unwrap();
    let catalog = WalletCatalog::open(dir.path()).unwrap();
    let main = catalog.repository(None).unwrap();
    add_position(&main, "2", "10");
    let before = main.export_legacy_json().unwrap();
    let connection = rusqlite::Connection::open(dir.path().join("wallets.sqlite")).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER fail_wallet_insert BEFORE INSERT ON wallets
                 BEGIN SELECT RAISE(ABORT, 'fixture rejection'); END;",
        )
        .unwrap();
    assert!(catalog
        .create(wallet_input("Falha simulada", "Instituição"))
        .is_err());
    assert_eq!(catalog.list().unwrap().len(), 1);
    assert_eq!(fs::read_dir(dir.path().join("wallets")).unwrap().count(), 0);
    assert_eq!(main.export_legacy_json().unwrap(), before);
}

#[test]
fn independent_connections_create_wallets_without_lost_catalog_rows() {
    let dir = TempDir::new().unwrap();
    let catalog = WalletCatalog::open(dir.path()).unwrap();
    let directory = dir.path().to_owned();
    let handles: Vec<_> = (0..4)
        .map(|index| {
            let directory = directory.clone();
            std::thread::spawn(move || {
                let catalog = WalletCatalog::open(directory).unwrap();
                catalog
                    .create(wallet_input(&format!("Carteira {index}"), "Instituição"))
                    .unwrap()
            })
        })
        .collect();
    let created: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    let wallets = catalog.list().unwrap();
    assert_eq!(wallets.len(), 5);
    assert_eq!(wallets.iter().filter(|wallet| wallet.is_default).count(), 1);
    for wallet in created {
        assert!(wallets.contains(&wallet));
        assert!(catalog
            .repository(Some(&wallet.id))
            .unwrap()
            .snapshot()
            .unwrap()
            .funds
            .is_empty());
    }
}

#[test]
fn missing_catalog_is_not_replaced_when_named_wallet_data_still_exists() {
    let dir = TempDir::new().unwrap();
    let catalog = WalletCatalog::open(dir.path()).unwrap();
    let wallet = catalog
        .create(wallet_input("Não perder", "Nubank"))
        .unwrap();
    let repository = catalog.repository(Some(&wallet.id)).unwrap();
    add_position(&repository, "9", "10");
    let before = repository.export_legacy_json().unwrap();
    drop(repository);
    drop(catalog);
    fs::remove_file(dir.path().join("wallets.sqlite")).unwrap();
    assert!(WalletCatalog::open(dir.path()).is_err());
    assert!(!dir.path().join("wallets.sqlite").exists());
    let repository = Repository::open_existing(
        dir.path()
            .join("wallets")
            .join(wallet.id)
            .join("portfolio.sqlite"),
    )
    .unwrap();
    assert_eq!(repository.export_legacy_json().unwrap(), before);
}
