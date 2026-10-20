//! Named portfolios share a catalog, while each portfolio keeps an independent
//! ledger. The original ledger remains at `portfolio.sqlite` permanently.

use super::{CoreError, Repository, Result};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;
use uuid::Uuid;

pub const DEFAULT_WALLET_ID: &str = "principal";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Wallet {
    pub id: String,
    pub name: String,
    pub institution: String,
    pub is_default: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletInput {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub institution: String,
}

pub struct WalletCatalog {
    connection: Mutex<Connection>,
    data_directory: PathBuf,
}

impl WalletCatalog {
    pub fn open(data_directory: impl AsRef<Path>) -> Result<Self> {
        fs::create_dir_all(data_directory.as_ref())?;
        let data_directory = fs::canonicalize(data_directory.as_ref())?;
        let catalog_path = data_directory.join("wallets.sqlite");
        if !catalog_path.exists() {
            let wallets_directory = data_directory.join("wallets");
            if wallets_directory.exists() {
                require_directory_child(&wallets_directory, &data_directory)?;
                if fs::read_dir(wallets_directory)?.next().is_some() {
                    return Err(invalid(
                        "Há carteiras locais sem o catálogo. Restaure wallets.sqlite antes de continuar.",
                    ));
                }
            }
        }
        if catalog_path.exists() {
            require_regular_child(&catalog_path, &data_directory)?;
        }
        let mut connection = Connection::open(catalog_path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA synchronous = FULL;",
        )?;
        let version: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > 1 {
            return Err(invalid(
                "O catálogo de carteiras foi criado por uma versão mais recente do aplicativo.",
            ));
        }
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(
            "CREATE TABLE IF NOT EXISTS wallets (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                institution TEXT NOT NULL,
                is_default INTEGER NOT NULL CHECK(is_default IN (0,1)),
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
             );
             CREATE UNIQUE INDEX IF NOT EXISTS wallets_one_default
                ON wallets(is_default) WHERE is_default = 1;",
        )?;
        let default_exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM wallets WHERE id = ?1 AND is_default = 1)",
            [DEFAULT_WALLET_ID],
            |row| row.get(0),
        )?;
        if !default_exists {
            let existing_wallets: i64 =
                transaction.query_row("SELECT COUNT(*) FROM wallets", [], |row| row.get(0))?;
            if existing_wallets != 0 || version != 0 {
                return Err(invalid(
                    "O catálogo está sem a carteira principal. Restaure o catálogo antes de continuar.",
                ));
            }
            let default_path = data_directory.join("portfolio.sqlite");
            let repository = if default_path.exists() {
                require_regular_child(&default_path, &data_directory)?;
                Repository::open_existing(default_path)?
            } else {
                Repository::open(default_path)?
            };
            let institution = repository
                .snapshot()?
                .broker_position
                .map(|position| position.institution)
                .unwrap_or_default();
            let now = Utc::now().to_rfc3339();
            transaction.execute(
                "INSERT INTO wallets(id,name,institution,is_default,created_at,updated_at)
                    VALUES (?1,?2,?3,1,?4,?4)",
                params![DEFAULT_WALLET_ID, "Carteira principal", institution, now],
            )?;
        }
        transaction.execute_batch("PRAGMA user_version = 1;")?;
        transaction.commit()?;
        Ok(Self {
            connection: Mutex::new(connection),
            data_directory,
        })
    }

    pub fn default_id(&self) -> &'static str {
        DEFAULT_WALLET_ID
    }

    pub fn list(&self) -> Result<Vec<Wallet>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id,name,institution,is_default,created_at,updated_at FROM wallets
                ORDER BY is_default DESC, created_at, id",
        )?;
        let rows = statement.query_map([], read_wallet)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn get(&self, wallet_id: Option<&str>) -> Result<Wallet> {
        let id = wallet_id.unwrap_or(DEFAULT_WALLET_ID);
        validate_id(id)?;
        self.connection()?
            .query_row(
                "SELECT id,name,institution,is_default,created_at,updated_at FROM wallets WHERE id=?1",
                [id],
                read_wallet,
            )
            .optional()?
            .ok_or_else(|| invalid("Carteira não encontrada."))
    }

    pub fn create(&self, input: WalletInput) -> Result<Wallet> {
        if input.id.is_some() {
            return Err(invalid(
                "Uma nova carteira não pode informar um identificador existente.",
            ));
        }
        let (name, institution) = validate_metadata(input.name, input.institution)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let wallet = Wallet {
            id,
            name,
            institution,
            is_default: false,
            created_at: now.clone(),
            updated_at: now,
        };
        let wallets_directory = self.wallets_directory()?;
        let wallet_directory = wallets_directory.join(&wallet.id);
        // create_dir refuses an existing directory, so an unrelated database is
        // never adopted even in the extraordinarily unlikely UUID collision.
        fs::create_dir(&wallet_directory)?;
        let create_result: Result<()> = (|| {
            require_directory_child(&wallet_directory, &wallets_directory)?;
            let repository = Repository::open(wallet_directory.join("portfolio.sqlite"))?;
            drop(repository);
            transaction.execute(
                "INSERT INTO wallets(id,name,institution,is_default,created_at,updated_at)
                    VALUES (?1,?2,?3,0,?4,?5)",
                params![
                    wallet.id,
                    wallet.name,
                    wallet.institution,
                    wallet.created_at,
                    wallet.updated_at
                ],
            )?;
            transaction.commit()?;
            Ok(())
        })();
        if let Err(error) = create_result {
            // Remove only known files in the UUID directory created above.
            // Never recurse into a path supplied by an application caller.
            // A failed disk commit can have an uncertain outcome: keep the
            // database unless the catalog confirms the wallet is absent.
            let catalog_has_wallet: rusqlite::Result<bool> = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM wallets WHERE id=?1)",
                [&wallet.id],
                |row| row.get(0),
            );
            if matches!(catalog_has_wallet, Ok(false))
                && require_directory_child(&wallet_directory, &wallets_directory).is_ok()
            {
                for name in [
                    "portfolio.sqlite",
                    "portfolio.sqlite-wal",
                    "portfolio.sqlite-shm",
                    "portfolio.sqlite-journal",
                ] {
                    let _ = fs::remove_file(wallet_directory.join(name));
                }
                let _ = fs::remove_dir(&wallet_directory);
            }
            return Err(error);
        }
        Ok(wallet)
    }

    pub fn update(&self, input: WalletInput) -> Result<Wallet> {
        let id = input
            .id
            .ok_or_else(|| invalid("Informe a carteira que deseja editar."))?;
        validate_id(&id)?;
        let (name, institution) = validate_metadata(input.name, input.institution)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let updated_at = Utc::now().to_rfc3339();
        let updated = transaction.execute(
            "UPDATE wallets SET name=?1,institution=?2,updated_at=?3 WHERE id=?4",
            params![name, institution, updated_at, id],
        )?;
        if updated != 1 {
            return Err(invalid("Carteira não encontrada."));
        }
        let wallet = transaction.query_row(
            "SELECT id,name,institution,is_default,created_at,updated_at FROM wallets WHERE id=?1",
            [&id],
            read_wallet,
        )?;
        transaction.commit()?;
        Ok(wallet)
    }

    pub fn repository(&self, wallet_id: Option<&str>) -> Result<Repository> {
        let wallet = self.get(wallet_id)?;
        let directory = if wallet.is_default {
            if wallet.id != DEFAULT_WALLET_ID {
                return Err(invalid("O identificador da carteira principal é inválido."));
            }
            self.data_directory.clone()
        } else {
            let wallets_directory = self.data_directory.join("wallets");
            require_directory_child(&wallets_directory, &self.data_directory)?;
            let directory = wallets_directory.join(wallet.id);
            require_directory_child(&directory, &wallets_directory)?;
            directory
        };
        let path = directory.join("portfolio.sqlite");
        require_regular_child(&path, &directory)?;
        Repository::open_existing(path)
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection.lock().map_err(|_| CoreError::LockPoisoned)
    }

    fn wallets_directory(&self) -> Result<PathBuf> {
        let directory = self.data_directory.join("wallets");
        match fs::create_dir(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        require_directory_child(&directory, &self.data_directory)?;
        Ok(directory)
    }
}

fn read_wallet(row: &rusqlite::Row<'_>) -> rusqlite::Result<Wallet> {
    Ok(Wallet {
        id: row.get(0)?,
        name: row.get(1)?,
        institution: row.get(2)?,
        is_default: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

fn validate_id(id: &str) -> Result<()> {
    if id == DEFAULT_WALLET_ID {
        return Ok(());
    }
    let parsed = Uuid::parse_str(id).map_err(|_| invalid("Identificador de carteira inválido."))?;
    if parsed.to_string() != id {
        return Err(invalid("Identificador de carteira inválido."));
    }
    Ok(())
}

fn validate_metadata(name: String, institution: String) -> Result<(String, String)> {
    let name = name.trim().to_owned();
    let institution = institution.trim().to_owned();
    if name.is_empty() || name.chars().count() > 120 || institution.chars().count() > 120 {
        return Err(invalid(
            "Informe um nome de carteira e uma instituição com até 120 caracteres.",
        ));
    }
    if name
        .chars()
        .chain(institution.chars())
        .any(char::is_control)
    {
        return Err(invalid(
            "O nome da carteira e a instituição não podem conter caracteres de controle.",
        ));
    }
    Ok((name, institution))
}

fn require_directory_child(path: &Path, parent: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            invalid(
                "A pasta desta carteira não foi encontrada. Restaure os dados antes de continuar.",
            )
        } else {
            error.into()
        }
    })?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(invalid(
            "O caminho da carteira não é uma pasta local válida.",
        ));
    }
    let resolved = fs::canonicalize(path)?;
    let parent = fs::canonicalize(parent)?;
    if resolved.parent() != Some(parent.as_path()) {
        return Err(invalid(
            "A pasta da carteira está fora do diretório de dados.",
        ));
    }
    Ok(())
}

fn require_regular_child(path: &Path, parent: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            invalid(
                "O banco desta carteira não foi encontrado. Restaure os dados antes de continuar.",
            )
        } else {
            error.into()
        }
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(invalid(
            "O banco da carteira não é um arquivo local válido.",
        ));
    }
    let resolved = fs::canonicalize(path)?;
    let parent = fs::canonicalize(parent)?;
    if resolved.parent() != Some(parent.as_path()) {
        return Err(invalid(
            "O banco da carteira está fora do diretório de dados.",
        ));
    }
    Ok(())
}

fn invalid(message: &str) -> CoreError {
    CoreError::Validation(message.to_owned())
}
