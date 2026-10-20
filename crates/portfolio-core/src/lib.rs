//! Local-first portfolio core. All monetary arithmetic uses decimal values.
//!
//! The original JSON is only ever read. An exact copy is saved before migration,
//! and import, receipt, and outbox rows commit in one SQLite transaction.

use chrono::{Datelike, Local, NaiveDate, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;
use thiserror::Error;
use uuid::Uuid;

mod broker;
mod cash;
pub mod performance;
pub mod portfolio_performance;
mod profile;
mod wallets;
mod wealth;
pub use broker::{
    BrokerFixedIncome, BrokerHolding, BrokerListedFunds, BrokerOverview, BrokerPosition,
};
pub use cash::{CashPocket, CashPocketInput, CashSimulation, CashSimulationInput};
pub use profile::{InvestmentProfile, InvestmentProfileInput};
pub use wallets::{Wallet, WalletCatalog, WalletInput, DEFAULT_WALLET_ID};
pub use wealth::{WalletWealth, WealthOverview, WealthTotals};

pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("Erro no banco local: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("Erro ao acessar arquivo: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON inválido: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Validation(String),
    #[error(
        "A carteira já contém dados. A importação de outro arquivo exige uma mesclagem explícita."
    )]
    ImportRequiresEmptyPortfolio,
    #[error("Banco local indisponível: uma operação anterior foi interrompida.")]
    LockPoisoned,
    #[error("Valor excede a precisão decimal suportada.")]
    ArithmeticOverflow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonthlyEntry {
    pub id: String,
    pub month: String,
    pub cotas_added: String,
    pub price_per_cota: String,
    pub dividend_per_cota: String,
    /// None means computed from all holdings through this month, inclusive.
    /// An explicit value is never silently recalculated.
    pub dividend_total: Option<String>,
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundView {
    pub id: String,
    pub ticker: String,
    pub name: String,
    pub sector: String,
    pub entries: Vec<MonthlyEntry>,
    pub total_cotas: String,
    pub total_invested: String,
    pub average_price: String,
    pub dividends_received: String,
    /// Current holdings multiplied by the mean of the last six positive
    /// per-share dividend records. This is an estimate, not a price quote.
    pub estimated_monthly_income: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioTotals {
    pub invested: String,
    pub cotas: String,
    pub dividends_received: String,
    pub estimated_monthly_income: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioSnapshot {
    pub funds: Vec<FundView>,
    pub totals: PortfolioTotals,
    pub record_count: usize,
    #[serde(default)]
    pub broker_position: Option<BrokerPosition>,
    #[serde(default)]
    pub cash_pockets: Vec<CashPocket>,
    #[serde(default = "zero_string")]
    pub cash_balance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundInput {
    #[serde(default)]
    pub id: Option<String>,
    pub ticker: String,
    pub name: String,
    #[serde(default)]
    pub sector: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordInput {
    #[serde(default)]
    pub id: Option<String>,
    pub fund_id: String,
    pub month: String,
    pub cotas_added: String,
    pub price_per_cota: String,
    pub dividend_per_cota: String,
    #[serde(default)]
    pub dividend_total: Option<String>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionInput {
    #[serde(default)]
    pub fund_id: Option<String>,
    #[serde(default = "default_months")]
    pub months: u32,
    /// Fixed monthly shares per selected fund; not a currency contribution.
    #[serde(default = "zero_string")]
    pub monthly_cotas: String,
    #[serde(default = "default_window")]
    pub window: Option<usize>,
    #[serde(default)]
    pub reinvest: bool,
}

impl Default for ProjectionInput {
    fn default() -> Self {
        Self {
            fund_id: None,
            months: default_months(),
            monthly_cotas: zero_string(),
            window: default_window(),
            reinvest: false,
        }
    }
}

fn default_months() -> u32 {
    12
}
fn default_window() -> Option<usize> {
    Some(6)
}
fn zero_string() -> String {
    "0".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionPoint {
    pub month: String,
    pub projected_cotas: String,
    pub projected_income: String,
    pub cumulative_income: String,
    pub reinvested_cotas: String,
    pub combined_cotas: String,
    pub combined_income: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReceipt {
    pub sha256: String,
    pub source_path: String,
    pub backup_path: String,
    pub fund_count: usize,
    pub record_count: usize,
    pub imported_at: String,
    pub already_imported: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingChange {
    pub id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub operation: String,
    pub payload: Value,
    pub created_at: String,
}

pub struct Repository {
    connection: Mutex<Connection>,
    database_path: PathBuf,
}

impl Repository {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_internal(path.as_ref(), true)
    }

    /// Opening a registered wallet must never recreate a missing database and
    /// present the resulting empty portfolio as if its data were intact.
    pub fn open_existing(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_internal(path.as_ref(), false)
    }

    fn open_internal(path: &Path, create: bool) -> Result<Self> {
        let path = path.to_path_buf();
        if let Some(parent) = path
            .parent()
            .filter(|p| create && !p.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
            | if create {
                rusqlite::OpenFlags::SQLITE_OPEN_CREATE
            } else {
                rusqlite::OpenFlags::empty()
            };
        let connection = Connection::open_with_flags(&path, flags)?;
        if !create {
            let has_ledger: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='funds')",
                [],
                |row| row.get(0),
            )?;
            if !has_ledger {
                return Err(CoreError::Validation("O arquivo da carteira está vazio ou não contém um banco válido do IvestFII. Restaure um backup antes de continuar.".into()));
            }
        }
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA synchronous = FULL;",
        )?;
        let version: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > 6 {
            return Err(CoreError::Validation(
                "Este banco foi criado por uma versão mais recente do aplicativo.".into(),
            ));
        }
        connection.execute_batch(
            "BEGIN IMMEDIATE;
             CREATE TABLE IF NOT EXISTS funds (
                 id TEXT PRIMARY KEY,
                 ticker TEXT NOT NULL UNIQUE,
                 name TEXT NOT NULL,
                 sector TEXT NOT NULL,
                 revision INTEGER NOT NULL DEFAULT 1,
                 updated_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS monthly_records (
                 id TEXT PRIMARY KEY,
                 fund_id TEXT NOT NULL REFERENCES funds(id),
                 month TEXT NOT NULL,
                 cotas_added TEXT NOT NULL,
                 price_per_cota TEXT NOT NULL,
                 dividend_per_cota TEXT NOT NULL,
                 dividend_total TEXT,
                 notes TEXT NOT NULL,
                 sequence INTEGER NOT NULL,
                 revision INTEGER NOT NULL DEFAULT 1,
                 updated_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS records_chronological ON monthly_records(fund_id, month, sequence);
             CREATE TABLE IF NOT EXISTS legacy_imports (
                 sha256 TEXT PRIMARY KEY,
                 source_path TEXT NOT NULL,
                 backup_path TEXT NOT NULL,
                 fund_count INTEGER NOT NULL,
                 record_count INTEGER NOT NULL,
                 imported_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS sync_outbox (
                 id TEXT PRIMARY KEY,
                 entity_type TEXT NOT NULL,
                 entity_id TEXT NOT NULL,
                 operation TEXT NOT NULL,
                 payload TEXT NOT NULL,
                 created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS broker_position (
                 singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                 payload TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS broker_position_imports (
                 sha256 TEXT PRIMARY KEY,
                 source_path TEXT NOT NULL,
                 backup_path TEXT NOT NULL,
                 imported_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS investment_profile (
                 singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                 payload TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS cash_pockets (
                 id TEXT PRIMARY KEY,
                 name TEXT NOT NULL,
                 institution TEXT NOT NULL,
                 balance TEXT NOT NULL,
                 cdi_percent TEXT NOT NULL,
                 annual_cdi_rate TEXT,
                 rate_date TEXT,
                 notes TEXT NOT NULL,
                 revision INTEGER NOT NULL DEFAULT 1,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             );
             "
        )?;
        // Recheck after obtaining the migration lock; another process may have
        // upgraded this database while this connection waited for BEGIN.
        let migration = (|| -> Result<()> {
            let locked_version: u32 =
                connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
            if locked_version > 6 {
                return Err(CoreError::Validation(
                    "Este banco foi criado por uma versão mais recente do aplicativo.".into(),
                ));
            }
            let columns = connection
                .prepare("PRAGMA table_info(cash_pockets)")?
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<std::result::Result<HashSet<_>, _>>()?;
            for (name, definition) in [
                ("rate_source", "TEXT NOT NULL DEFAULT 'manual'"),
                ("rate_source_url", "TEXT"),
                ("rate_fetched_at", "TEXT"),
                ("total_deposits", "TEXT"),
                ("total_withdrawals", "TEXT"),
                ("balance_date", "TEXT"),
            ] {
                if !columns.contains(name) {
                    connection.execute_batch(&format!(
                        "ALTER TABLE cash_pockets ADD COLUMN {name} {definition};"
                    ))?;
                }
            }
            connection.execute_batch("PRAGMA user_version = 6; COMMIT;")?;
            Ok(())
        })();
        if let Err(error) = migration {
            let _ = connection.execute_batch("ROLLBACK;");
            return Err(error);
        }
        Ok(Self {
            connection: Mutex::new(connection),
            database_path: path,
        })
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection.lock().map_err(|_| CoreError::LockPoisoned)
    }

    pub fn import_legacy_file(&self, path: impl AsRef<Path>) -> Result<ImportReceipt> {
        let source_path = fs::canonicalize(path.as_ref())?;
        let bytes = fs::read(&source_path)?;
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        let backup_dir = self
            .database_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("backups");
        fs::create_dir_all(&backup_dir)?;
        let backup_path = backup_dir.join(format!("legacy-{sha256}.json"));
        // Never overwrite an existing backup. Verify it rather than trusting its name.
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&backup_path)
        {
            Ok(mut file) => {
                file.write_all(&bytes)?;
                file.sync_all()?;
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                if fs::read(&backup_path)? != bytes {
                    return Err(CoreError::Validation("O backup existente não corresponde ao arquivo de origem; a importação foi cancelada.".into()));
                }
            }
            Err(err) => return Err(err.into()),
        }
        // Validate every field and all calculations before touching portfolio rows.
        let imported = parse_legacy(&bytes)?;
        let embedded_position = broker::parse_embedded(&bytes)?;
        let embedded_cash = cash::parse_embedded(&bytes)?;
        let embedded_profile = profile::parse_embedded(&bytes)?;
        for fund in &imported {
            calculate_fund(
                fund.id.clone(),
                fund.ticker.clone(),
                fund.name.clone(),
                fund.sector.clone(),
                fund.entries.clone(),
            )?;
        }
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let previous: Option<ImportReceipt> = tx.query_row(
            "SELECT sha256, source_path, backup_path, fund_count, record_count, imported_at FROM legacy_imports WHERE sha256 = ?1",
            [&sha256],
            |row| Ok(ImportReceipt {
                sha256: row.get(0)?, source_path: row.get(1)?, backup_path: row.get(2)?,
                fund_count: read_count(row, 3)?, record_count: read_count(row, 4)?, imported_at: row.get(5)?, already_imported: true,
            }),
        ).optional()?;
        if let Some(receipt) = previous {
            return Ok(receipt);
        }
        let fund_count: i64 = tx.query_row(
            "SELECT (SELECT COUNT(*) FROM funds)+(SELECT COUNT(*) FROM cash_pockets)",
            [],
            |row| row.get(0),
        )?;
        if fund_count > 0 {
            return Err(CoreError::ImportRequiresEmptyPortfolio);
        }
        if embedded_profile.is_some() && profile::load(&tx)?.is_some() {
            return Err(CoreError::Validation("Esta carteira já possui um perfil de investimento. A importação não sobrescreve o perfil existente.".into()));
        }
        let now = Utc::now().to_rfc3339();
        let record_count = imported.iter().map(|fund| fund.entries.len()).sum();
        for fund in &imported {
            tx.execute("INSERT INTO funds(id, ticker, name, sector, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![fund.id, fund.ticker, fund.name, fund.sector, now])?;
            enqueue(
                &tx,
                "fund",
                &fund.id,
                &json!({"id":fund.id,"ticker":fund.ticker,"name":fund.name,"sector":fund.sector,"revision":1}),
                &now,
            )?;
            for (index, entry) in fund.entries.iter().enumerate() {
                insert_entry(&tx, &fund.id, entry, index as i64, &now)?;
                enqueue(
                    &tx,
                    "monthlyRecord",
                    &entry.id,
                    &entry_payload(&fund.id, entry, 1),
                    &now,
                )?;
            }
        }
        if let Some(position) = embedded_position {
            broker::store_position(&tx, &position)?;
        }
        cash::store_imported(&tx, &embedded_cash, &now)?;
        if let Some(profile) = embedded_profile {
            profile::store_imported(&tx, &profile)?;
        }
        snapshot_connection(&tx)?;
        let receipt = ImportReceipt {
            sha256,
            source_path: source_path.to_string_lossy().into_owned(),
            backup_path: fs::canonicalize(backup_path)?
                .to_string_lossy()
                .into_owned(),
            fund_count: imported.len(),
            record_count,
            imported_at: now,
            already_imported: false,
        };
        tx.execute("INSERT INTO legacy_imports(sha256, source_path, backup_path, fund_count, record_count, imported_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![receipt.sha256, receipt.source_path, receipt.backup_path, receipt.fund_count as i64, receipt.record_count as i64, receipt.imported_at])?;
        tx.commit()?;
        Ok(receipt)
    }

    pub fn snapshot(&self) -> Result<PortfolioSnapshot> {
        let mut connection = self.connection()?;
        // Desktop and MCP may open the same WAL database in different
        // processes. Keep all fund/record reads on one committed revision.
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)?;
        let snapshot = snapshot_connection(&tx)?;
        tx.commit()?;
        Ok(snapshot)
    }

    pub fn upsert_fund(&self, input: FundInput) -> Result<()> {
        let ticker = normalize_ticker(&input.ticker)?;
        let name = input.name.trim();
        let sector = input.sector.trim();
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let ticker_id: Option<String> = tx
            .query_row("SELECT id FROM funds WHERE ticker = ?1", [&ticker], |r| {
                r.get(0)
            })
            .optional()?;
        let id = match input.id.filter(|value| !value.trim().is_empty()) {
            Some(id) => {
                if ticker_id.as_ref().is_some_and(|existing| *existing != id) {
                    return Err(CoreError::Validation(
                        "Este ticker já pertence a outro fundo.".into(),
                    ));
                }
                id
            }
            None => {
                if ticker_id.is_some() {
                    return Err(CoreError::Validation("Este fundo já está cadastrado. Use a edição para alterar os dados existentes.".into()));
                }
                Uuid::new_v4().to_string()
            }
        };
        let now = Utc::now().to_rfc3339();
        tx.execute(
            "INSERT INTO funds(id, ticker, name, sector, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET ticker=excluded.ticker, name=excluded.name, sector=excluded.sector, revision=funds.revision+1, updated_at=excluded.updated_at",
            params![id, ticker, name, sector, now],
        )?;
        let revision: i64 =
            tx.query_row("SELECT revision FROM funds WHERE id = ?1", [&id], |r| {
                r.get(0)
            })?;
        enqueue(
            &tx,
            "fund",
            &id,
            &json!({"id":id,"ticker":ticker,"name":name,"sector":sector,"revision":revision}),
            &now,
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn save_monthly_record(&self, input: RecordInput) -> Result<()> {
        let month = normalize_month(&input.month)?;
        let entry = MonthlyEntry {
            id: input
                .id
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            month,
            cotas_added: decimal_string(parse_nonnegative(
                &input.cotas_added,
                "Quantidade de cotas",
            )?),
            price_per_cota: decimal_string(parse_nonnegative(
                &input.price_per_cota,
                "Preço por cota",
            )?),
            dividend_per_cota: decimal_string(parse_nonnegative(
                &input.dividend_per_cota,
                "Rendimento por cota",
            )?),
            dividend_total: input
                .dividend_total
                .map(|value| parse_nonnegative(&value, "Rendimento total").map(decimal_string))
                .transpose()?,
            notes: input.notes,
        };
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM funds WHERE id = ?1)",
            [&input.fund_id],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(CoreError::Validation("Fundo não encontrado.".into()));
        }
        let current: Option<(String, i64)> = tx
            .query_row(
                "SELECT fund_id, sequence FROM monthly_records WHERE id = ?1",
                [&entry.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let sequence = match current {
            Some((fund_id, sequence)) if fund_id == input.fund_id => sequence,
            Some(_) => {
                return Err(CoreError::Validation(
                    "Este lançamento pertence a outro fundo.".into(),
                ))
            }
            None => tx.query_row(
                "SELECT COALESCE(MAX(sequence), -1) + 1 FROM monthly_records WHERE fund_id = ?1",
                [&input.fund_id],
                |r| r.get(0),
            )?,
        };
        let now = Utc::now().to_rfc3339();
        insert_entry(&tx, &input.fund_id, &entry, sequence, &now)?;
        // This also prevents persisting a value that would make aggregate
        // arithmetic overflow. All changes (including outbox) roll back on error.
        snapshot_connection(&tx)?;
        let revision: i64 = tx.query_row(
            "SELECT revision FROM monthly_records WHERE id = ?1",
            [&entry.id],
            |r| r.get(0),
        )?;
        enqueue(
            &tx,
            "monthlyRecord",
            &entry.id,
            &entry_payload(&input.fund_id, &entry, revision),
            &now,
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Export the legacy schema using JSON numbers, not floating-point conversion.
    /// Decimal values and explicit dividend totals survive export/import exactly.
    pub fn export_legacy_json(&self) -> Result<String> {
        let (snapshot, profile) = {
            let mut connection = self.connection()?;
            let tx =
                connection.transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)?;
            let snapshot = snapshot_connection(&tx)?;
            let profile = profile::load(&tx)?;
            tx.commit()?;
            (snapshot, profile)
        };
        let mut funds = Vec::with_capacity(snapshot.funds.len());
        for fund in snapshot.funds {
            let mut entries = Vec::with_capacity(fund.entries.len());
            for entry in fund.entries {
                entries.push(json!({
                    "month": entry.month,
                    "cotas_added": json_decimal(&entry.cotas_added)?,
                    "price_per_cota": json_decimal(&entry.price_per_cota)?,
                    "dividend_per_cota": json_decimal(&entry.dividend_per_cota)?,
                    "dividend_total": entry.dividend_total.map(|v| json_decimal(&v)).transpose()?,
                    "notes": entry.notes,
                }));
            }
            funds.push(json!({"ticker":fund.ticker,"name":fund.name,"sector":fund.sector,"entries":entries}));
        }
        let mut value = json!({"fiis": funds});
        if let Some(position) = snapshot.broker_position {
            value["broker_position"] = serde_json::to_value(position)?;
        }
        if !snapshot.cash_pockets.is_empty() {
            value["cash_pockets"] = serde_json::to_value(snapshot.cash_pockets)?;
        }
        if let Some(profile) = profile {
            value["investment_profile"] = serde_json::to_value(profile)?;
        }
        Ok(serde_json::to_string_pretty(&value)?)
    }

    pub fn project(&self, input: ProjectionInput) -> Result<Vec<ProjectionPoint>> {
        if input.months == 0 || input.months > 1200 {
            return Err(CoreError::Validation(
                "A projeção deve ter entre 1 e 1200 meses.".into(),
            ));
        }
        if input.window == Some(0) {
            return Err(CoreError::Validation(
                "A janela de rendimentos deve ser maior que zero.".into(),
            ));
        }
        let monthly_cotas = parse_nonnegative(&input.monthly_cotas, "Aporte mensal em cotas")?;
        let snapshot = self.snapshot()?;
        let selected: Vec<FundView> = snapshot
            .funds
            .into_iter()
            .filter(|fund| input.fund_id.as_ref().is_none_or(|id| fund.id == *id))
            .collect();
        if input.fund_id.is_some() && selected.is_empty() {
            return Err(CoreError::Validation("Fundo não encontrado.".into()));
        }
        if selected.is_empty() {
            return Ok(Vec::new());
        }
        let reference_month = selected
            .iter()
            .flat_map(|fund| fund.entries.iter().map(|entry| entry.month.as_str()))
            .max()
            .map(str::to_owned)
            .unwrap_or_else(|| Local::now().format("%Y-%m").to_string());
        let mut states = selected
            .iter()
            .map(|fund| {
                Ok(ProjectionState {
                    cotas: parse_decimal(&fund.total_cotas)?,
                    price: parse_decimal(&fund.average_price)?,
                    dividend: average_dividend(&fund.entries, input.window)?,
                    cash: Decimal::ZERO,
                    reinvested: Decimal::ZERO,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut points = Vec::with_capacity(input.months as usize);
        let mut cumulative_income = Decimal::ZERO;
        for offset in 1..=input.months {
            let mut projected_cotas = Decimal::ZERO;
            let mut projected_income = Decimal::ZERO;
            let mut reinvested_cotas = Decimal::ZERO;
            let mut combined_cotas = Decimal::ZERO;
            let mut combined_income = Decimal::ZERO;
            for state in &mut states {
                state.cotas = add(state.cotas, monthly_cotas)?;
                projected_cotas = add(projected_cotas, state.cotas)?;
                let income = money(mul(state.cotas, state.dividend)?);
                projected_income = add(projected_income, income)?;
                if input.reinvest && state.price > Decimal::ZERO {
                    state.cash = add(state.cash, income)?;
                    let extra = div(state.cash, state.price)?.floor();
                    state.cotas = add(state.cotas, extra)?;
                    state.reinvested = add(state.reinvested, extra)?;
                    state.cash = state
                        .cash
                        .checked_sub(mul(extra, state.price)?)
                        .ok_or(CoreError::ArithmeticOverflow)?;
                }
                reinvested_cotas = add(reinvested_cotas, state.reinvested)?;
                combined_cotas = add(combined_cotas, state.cotas)?;
                combined_income = add(combined_income, money(mul(state.cotas, state.dividend)?))?;
            }
            cumulative_income = add(cumulative_income, projected_income)?;
            points.push(ProjectionPoint {
                month: month_after(&reference_month, offset)?,
                projected_cotas: decimal_string(projected_cotas),
                projected_income: decimal_string(projected_income),
                cumulative_income: decimal_string(cumulative_income),
                reinvested_cotas: decimal_string(reinvested_cotas),
                combined_cotas: decimal_string(combined_cotas),
                combined_income: decimal_string(combined_income),
            });
        }
        Ok(points)
    }

    /// Durable local queue only. No entry is marked delivered before a future
    /// authenticated sync adapter receives an acknowledgement from the server.
    pub fn pending_change_count(&self) -> Result<usize> {
        Ok(self
            .connection()?
            .query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| read_count(r, 0))?)
    }

    pub fn pending_changes(&self) -> Result<Vec<PendingChange>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare("SELECT id, entity_type, entity_id, operation, payload, created_at FROM sync_outbox ORDER BY rowid")?;
        let rows = statement.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        let mut changes = Vec::new();
        for row in rows {
            let (id, entity_type, entity_id, operation, payload, created_at) = row?;
            changes.push(PendingChange {
                id,
                entity_type,
                entity_id,
                operation,
                payload: serde_json::from_str(&payload)?,
                created_at,
            });
        }
        Ok(changes)
    }
}

struct ProjectionState {
    cotas: Decimal,
    price: Decimal,
    dividend: Decimal,
    cash: Decimal,
    reinvested: Decimal,
}

struct ImportedFund {
    id: String,
    ticker: String,
    name: String,
    sector: String,
    entries: Vec<MonthlyEntry>,
}

fn snapshot_connection(connection: &Connection) -> Result<PortfolioSnapshot> {
    let mut statement =
        connection.prepare("SELECT id, ticker, name, sector FROM funds ORDER BY ticker")?;
    let funds = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut result = Vec::with_capacity(funds.len());
    let mut total_invested = Decimal::ZERO;
    let mut total_cotas = Decimal::ZERO;
    let mut dividends_received = Decimal::ZERO;
    let mut estimated_monthly_income = Decimal::ZERO;
    let mut record_count = 0;
    for (id, ticker, name, sector) in funds {
        let mut records = connection.prepare("SELECT id, month, cotas_added, price_per_cota, dividend_per_cota, dividend_total, notes FROM monthly_records WHERE fund_id = ?1 ORDER BY month, sequence, id")?;
        let entries = records
            .query_map([&id], |r| {
                Ok(MonthlyEntry {
                    id: r.get(0)?,
                    month: r.get(1)?,
                    cotas_added: r.get(2)?,
                    price_per_cota: r.get(3)?,
                    dividend_per_cota: r.get(4)?,
                    dividend_total: r.get(5)?,
                    notes: r.get(6)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        record_count += entries.len();
        let fund = calculate_fund(id, ticker, name, sector, entries)?;
        total_invested = add(total_invested, parse_decimal(&fund.total_invested)?)?;
        total_cotas = add(total_cotas, parse_decimal(&fund.total_cotas)?)?;
        dividends_received = add(dividends_received, parse_decimal(&fund.dividends_received)?)?;
        estimated_monthly_income = add(
            estimated_monthly_income,
            parse_decimal(&fund.estimated_monthly_income)?,
        )?;
        result.push(fund);
    }
    let cash_pockets = cash::load_cash_pockets(connection)?;
    let cash_balance = cash_pockets.iter().try_fold(Decimal::ZERO, |sum, pocket| {
        add(sum, parse_decimal(&pocket.balance)?)
    })?;
    // Validate new cash accounting aggregates within the saving/importing
    // transaction, before either a financial row or its outbox is committed.
    let cash_performance = portfolio_performance::calculate_cash_performance(&cash_pockets)?;
    add(
        total_invested,
        parse_decimal(&cash_performance.known_invested)?,
    )?;
    add(
        dividends_received,
        parse_decimal(&cash_performance.known_income)?,
    )?;
    Ok(PortfolioSnapshot {
        funds: result,
        totals: PortfolioTotals {
            invested: decimal_string(total_invested),
            cotas: decimal_string(total_cotas),
            dividends_received: decimal_string(dividends_received),
            estimated_monthly_income: decimal_string(estimated_monthly_income),
        },
        record_count,
        broker_position: broker::load_position(connection)?,
        cash_pockets,
        cash_balance: decimal_string(cash_balance),
    })
}

fn calculate_fund(
    id: String,
    ticker: String,
    name: String,
    sector: String,
    mut entries: Vec<MonthlyEntry>,
) -> Result<FundView> {
    // Stable sort retains distinct source ordering within the same month.
    entries.sort_by(|a, b| a.month.cmp(&b.month));
    let mut holdings_by_month = BTreeMap::new();
    let mut total_cotas = Decimal::ZERO;
    let mut total_invested = Decimal::ZERO;
    for entry in &entries {
        let cotas = parse_decimal(&entry.cotas_added)?;
        total_cotas = add(total_cotas, cotas)?;
        total_invested = add(
            total_invested,
            mul(cotas, parse_decimal(&entry.price_per_cota)?)?,
        )?;
        holdings_by_month.insert(entry.month.clone(), total_cotas);
    }
    let mut dividends_received = Decimal::ZERO;
    for entry in &entries {
        let dividend = match &entry.dividend_total {
            Some(explicit) => parse_decimal(explicit)?,
            None => money(mul(
                *holdings_by_month
                    .get(&entry.month)
                    .unwrap_or(&Decimal::ZERO),
                parse_decimal(&entry.dividend_per_cota)?,
            )?),
        };
        dividends_received = add(dividends_received, dividend)?;
    }
    let average_price = if total_cotas.is_zero() {
        Decimal::ZERO
    } else {
        div(total_invested, total_cotas)?
    };
    let estimated_monthly_income = money(mul(total_cotas, average_dividend(&entries, Some(6))?)?);
    Ok(FundView {
        id,
        ticker,
        name,
        sector,
        entries,
        total_cotas: decimal_string(total_cotas),
        total_invested: decimal_string(total_invested),
        average_price: decimal_string(average_price),
        dividends_received: decimal_string(dividends_received),
        estimated_monthly_income: decimal_string(estimated_monthly_income),
    })
}

fn parse_legacy(bytes: &[u8]) -> Result<Vec<ImportedFund>> {
    let root: Value =
        serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes))?;
    let raw_funds = root.get("fiis").and_then(Value::as_array).ok_or_else(|| {
        CoreError::Validation(
            "O arquivo precisa conter a lista 'fiis'. Nenhum dado foi importado.".into(),
        )
    })?;
    let mut tickers = HashSet::new();
    let mut result = Vec::with_capacity(raw_funds.len());
    for raw in raw_funds {
        let ticker = normalize_ticker(required_string(raw, "ticker")?)?;
        if !tickers.insert(ticker.clone()) {
            return Err(CoreError::Validation(format!(
                "Ticker duplicado no arquivo: {ticker}."
            )));
        }
        let id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("ivestfii/fund/{ticker}").as_bytes(),
        )
        .to_string();
        let raw_entries = raw
            .get("entries")
            .map(|value| {
                value.as_array().ok_or_else(|| {
                    CoreError::Validation(format!("Histórico inválido para {ticker}."))
                })
            })
            .transpose()?;
        let mut entries = Vec::new();
        let mut occurrences = HashMap::<String, usize>::new();
        for raw_entry in raw_entries.into_iter().flatten() {
            let mut entry = MonthlyEntry {
                id: String::new(),
                month: normalize_month(required_string(raw_entry, "month")?)?,
                cotas_added: legacy_decimal(raw_entry, "cotas_added", false)?
                    .unwrap_or_else(zero_string),
                price_per_cota: legacy_decimal(raw_entry, "price_per_cota", false)?
                    .unwrap_or_else(zero_string),
                dividend_per_cota: legacy_decimal(raw_entry, "dividend_per_cota", false)?
                    .unwrap_or_else(zero_string),
                dividend_total: legacy_decimal(raw_entry, "dividend_total", true)?,
                notes: optional_string(raw_entry, "notes")?,
            };
            let fingerprint = format!("{:x}", Sha256::digest(serde_json::to_vec(&entry)?));
            let occurrence = occurrences.entry(fingerprint.clone()).or_default();
            entry.id = Uuid::new_v5(
                &Uuid::NAMESPACE_URL,
                format!("ivestfii/entry/{id}/{fingerprint}/{occurrence}").as_bytes(),
            )
            .to_string();
            *occurrence += 1;
            entries.push(entry);
        }
        result.push(ImportedFund {
            id,
            ticker,
            name: optional_string(raw, "name")?,
            sector: optional_string(raw, "sector")?,
            entries,
        });
    }
    Ok(result)
}

fn insert_entry(
    tx: &Transaction<'_>,
    fund_id: &str,
    entry: &MonthlyEntry,
    sequence: i64,
    now: &str,
) -> Result<()> {
    tx.execute(
        "INSERT INTO monthly_records(id, fund_id, month, cotas_added, price_per_cota, dividend_per_cota, dividend_total, notes, sequence, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(id) DO UPDATE SET month=excluded.month, cotas_added=excluded.cotas_added, price_per_cota=excluded.price_per_cota,
         dividend_per_cota=excluded.dividend_per_cota, dividend_total=excluded.dividend_total, notes=excluded.notes,
         revision=monthly_records.revision+1, updated_at=excluded.updated_at",
        params![entry.id, fund_id, entry.month, entry.cotas_added, entry.price_per_cota, entry.dividend_per_cota, entry.dividend_total, entry.notes, sequence, now],
    )?;
    Ok(())
}

fn entry_payload(fund_id: &str, entry: &MonthlyEntry, revision: i64) -> Value {
    let mut value =
        serde_json::to_value(entry).expect("serializing a strings-only entry cannot fail");
    value["fundId"] = json!(fund_id);
    value["revision"] = json!(revision);
    value
}

fn enqueue(
    tx: &Transaction<'_>,
    entity_type: &str,
    entity_id: &str,
    payload: &Value,
    now: &str,
) -> Result<()> {
    tx.execute("INSERT INTO sync_outbox(id, entity_type, entity_id, operation, payload, created_at) VALUES (?1, ?2, ?3, 'upsert', ?4, ?5)",
        params![Uuid::new_v4().to_string(), entity_type, entity_id, serde_json::to_string(payload)?, now])?;
    Ok(())
}

fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value.get(field).and_then(Value::as_str).ok_or_else(|| {
        CoreError::Validation(format!("Campo '{field}' ausente ou inválido no arquivo."))
    })
}

fn optional_string(value: &Value, field: &str) -> Result<String> {
    match value.get(field) {
        None => Ok(String::new()),
        Some(Value::String(value)) => Ok(value.clone()),
        _ => Err(CoreError::Validation(format!(
            "Campo '{field}' precisa ser texto."
        ))),
    }
}

fn legacy_decimal(value: &Value, field: &str, nullable: bool) -> Result<Option<String>> {
    let decimal = match value.get(field) {
        None => return Ok(None),
        Some(Value::Null) if nullable => return Ok(None),
        Some(Value::Number(value)) => value.to_string(),
        Some(Value::String(value)) => value.clone(),
        _ => {
            return Err(CoreError::Validation(format!(
                "Campo '{field}' precisa ser um número decimal."
            )))
        }
    };
    Ok(Some(decimal_string(parse_nonnegative(&decimal, field)?)))
}

fn json_decimal(value: &str) -> Result<Value> {
    Ok(Value::Number(serde_json::Number::from_str(value)?))
}

fn normalize_ticker(value: &str) -> Result<String> {
    let ticker = value.trim().to_uppercase();
    if ticker.is_empty() || ticker.len() > 32 || !ticker.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return Err(CoreError::Validation(
            "Informe um ticker com letras e números (até 32 caracteres).".into(),
        ));
    }
    Ok(ticker)
}

fn normalize_month(value: &str) -> Result<String> {
    let mut value = value.trim().replace('/', "-");
    if value.len() == 6 && value.bytes().all(|b| b.is_ascii_digit()) {
        value.insert(4, '-');
    }
    if value.len() != 7 || value.as_bytes().get(4) != Some(&b'-') {
        return Err(CoreError::Validation(
            "Informe o mês no formato AAAA-MM.".into(),
        ));
    }
    let date = NaiveDate::parse_from_str(&format!("{value}-01"), "%Y-%m-%d")
        .map_err(|_| CoreError::Validation("Informe um mês válido no formato AAAA-MM.".into()))?;
    if date.year() < 1 {
        return Err(CoreError::Validation(
            "O ano precisa ser maior que zero.".into(),
        ));
    }
    Ok(date.format("%Y-%m").to_string())
}

fn month_after(reference: &str, offset: u32) -> Result<String> {
    let date = NaiveDate::parse_from_str(&format!("{reference}-01"), "%Y-%m-%d")
        .map_err(|_| CoreError::Validation("Mês de referência inválido.".into()))?;
    let index = date.year() * 12 + date.month0() as i32 + offset as i32;
    let year = index / 12;
    if year > 9999 {
        return Err(CoreError::Validation(
            "A projeção excede o ano 9999.".into(),
        ));
    }
    Ok(format!("{:04}-{:02}", year, index % 12 + 1))
}

fn parse_decimal(value: &str) -> Result<Decimal> {
    let value = value.trim();
    let invalid = || {
        CoreError::Validation(format!(
            "Número decimal inválido ou fora da precisão suportada: '{value}'."
        ))
    };
    if let Some(index) = value.find(['e', 'E']) {
        // rust_decimal::from_scientific parses the mantissa with a rounding
        // parser. Import must reject unrepresentable source values instead.
        let mut base = Decimal::from_str_exact(&value[..index])
            .map_err(|_| invalid())?
            .normalize();
        let exponent: i32 = value[index + 1..].parse().map_err(|_| invalid())?;
        if !(-28..=28).contains(&exponent) {
            return Err(invalid());
        }
        let target_scale = base.scale() as i32 - exponent;
        if target_scale > 28 {
            return Err(invalid());
        }
        if target_scale >= 0 {
            base.set_scale(target_scale as u32).map_err(|_| invalid())?;
        } else {
            base.set_scale(0).map_err(|_| invalid())?;
            for _ in 0..-target_scale {
                base = base.checked_mul(Decimal::TEN).ok_or_else(invalid)?;
            }
        }
        Ok(base)
    } else {
        Decimal::from_str_exact(value).map_err(|_| invalid())
    }
}

fn read_count(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<usize> {
    let count = row.get::<_, i64>(index)?;
    usize::try_from(count).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, count))
}

fn parse_nonnegative(value: &str, label: &str) -> Result<Decimal> {
    let parsed = parse_decimal(value)?;
    if parsed < Decimal::ZERO {
        return Err(CoreError::Validation(format!(
            "{label} não pode ser negativo."
        )));
    }
    Ok(parsed)
}

fn decimal_string(value: Decimal) -> String {
    value.normalize().to_string()
}
fn money(value: Decimal) -> Decimal {
    value.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
}
fn add(a: Decimal, b: Decimal) -> Result<Decimal> {
    a.checked_add(b).ok_or(CoreError::ArithmeticOverflow)
}
fn mul(a: Decimal, b: Decimal) -> Result<Decimal> {
    a.checked_mul(b).ok_or(CoreError::ArithmeticOverflow)
}
fn div(a: Decimal, b: Decimal) -> Result<Decimal> {
    a.checked_div(b).ok_or(CoreError::ArithmeticOverflow)
}

fn average_dividend(entries: &[MonthlyEntry], window: Option<usize>) -> Result<Decimal> {
    let mut values = Vec::new();
    for entry in entries {
        let value = parse_decimal(&entry.dividend_per_cota)?;
        if value > Decimal::ZERO {
            values.push(value);
        }
    }
    if let Some(window) = window {
        if values.len() > window {
            values.drain(..values.len() - window);
        }
    }
    if values.is_empty() {
        return Ok(Decimal::ZERO);
    }
    let sum = values
        .iter()
        .try_fold(Decimal::ZERO, |acc, v| add(acc, *v))?;
    div(sum, Decimal::from(values.len() as u64))
}
