//! Official CVM monthly FII and FIAGRO reports. These are dated accounting observations,
//! never market quotes. Only public archives are downloaded; symbols and CNPJs
//! are matched locally and no portfolio data is sent to CVM.
//!
//! Source schemas verified against the CVM dictionaries and live 2025/2026
//! archives: https://dados.cvm.gov.br/dataset/fii-doc-inf_mensal and
//! https://dados.cvm.gov.br/dataset/fi-cad . CSVs are semicolon-delimited Latin-1.
//! CVM's register has no ticker; the caller must supply an explicit CNPJ link.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex, OnceLock, Weak},
    time::Duration,
};

use crate::refresh_policy::{RefreshGate, RefreshMode, RefreshOutcome};
use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, SecondsFormat, Utc};
use reqwest::{redirect::Policy, Client};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

mod fiagro;

const REGISTER_URL: &str = "https://dados.cvm.gov.br/dados/FI/CAD/DADOS/registro_fundo_classe.zip";
const SOURCE: &str = "CVM · Informe Mensal Estruturado de FII";
const CACHE_VERSION: u32 = 2;
const MAX_DOWNLOAD: usize = 16 * 1024 * 1024;
const MAX_MEMBER: u64 = 40 * 1024 * 1024;
const MAX_EXPANDED: u64 = 96 * 1024 * 1024;
const MAX_CACHE: u64 = 16 * 1024 * 1024;
const MAX_ROWS: usize = 150_000;
const REFRESH_HOURS: i64 = 6;
static REFRESH_GATES: OnceLock<StdMutex<BTreeMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CvmFundRequest {
    pub symbol: String,
    pub cnpj: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CvmPanel {
    pub items: Vec<CvmFundItem>,
    pub fetched_at: Option<String>,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CvmFundItem {
    pub symbol: String,
    pub cnpj: Option<String>,
    /// mapping_required, invalid_mapping, not_found, unsupported, available,
    /// cached, unavailable.
    pub status: String,
    pub report: Option<CvmFundReport>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CvmFundReport {
    #[serde(default = "default_fii")]
    pub fund_type: String,
    pub cnpj: String,
    pub fund_name: String,
    /// CVM represents a monthly reference as YYYY-MM-01, not a trading date.
    pub reference_date: String,
    pub filed_at: Option<String>,
    pub net_assets: Option<String>,
    pub equity_per_share: Option<String>,
    pub shareholders: Option<u64>,
    pub issued_shares: Option<String>,
    pub source: String,
    pub source_url: String,
    pub fetched_at: String,
}

fn default_fii() -> String {
    "FII".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredReport {
    version: u32,
    report: CvmFundReport,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AnnualReports {
    fetched_at: String,
    reports: BTreeMap<String, StoredReport>,
    /// An ambiguous highest revision is isolated to its class, never selected.
    #[serde(default)]
    rejected: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Register {
    fetched_at: String,
    /// Only official FII fund/class entries. FIAGRO/FIDC/etc. are excluded.
    funds: BTreeMap<String, String>,
    /// Current active FIAGRO fund/class entries; never historical canceled FIIs.
    #[serde(default, rename = "fiagroFunds")]
    fiagro_funds: BTreeMap<String, String>,
    /// Exact register types for non-FII CNPJs; no name-based classification.
    #[serde(default)]
    other_funds: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Cache {
    version: u32,
    register: Option<Register>,
    years: BTreeMap<i32, AnnualReports>,
    /// Separate monthly FIAGRO archives, compatible with the existing v2 cache.
    #[serde(default, rename = "fiagroMonths")]
    fiagro_months: BTreeMap<String, AnnualReports>,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            version: CACHE_VERSION,
            register: None,
            years: BTreeMap::new(),
            fiagro_months: BTreeMap::new(),
        }
    }
}

/// Clones share a refresh lock. A short file lock protects cache reads/merges
/// across processes; HTTP never runs while that file lock is held. Successful
/// archives are reused for six hours, independently of wallet or CNPJ selection.
#[derive(Clone)]
pub struct CvmService {
    client: Client,
    cache_path: PathBuf,
    refresh_lock: Arc<Mutex<()>>,
}

impl CvmService {
    pub fn new(cache_path: PathBuf) -> Result<Self, String> {
        if cache_path.file_name().is_none() {
            return Err("Informe um arquivo para o cache da CVM.".into());
        }
        let cache_path = std::path::absolute(cache_path)
            .map_err(|_| "Caminho do cache CVM inválido.".to_string())?;
        let refresh_lock = refresh_gate(&cache_path)?;
        let client = Client::builder()
            .https_only(true)
            // CVM currently serves the canonical HTTPS archive without redirects.
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(60))
            .user_agent("IvestFII/0.1 (CVM public monthly FII and FIAGRO reports)")
            .build()
            .map_err(|_| "Não foi possível preparar a conexão HTTPS com a CVM.".to_string())?;
        Ok(Self {
            client,
            cache_path,
            refresh_lock,
        })
    }

    pub fn read_cache(&self, funds: &[CvmFundRequest]) -> Result<CvmPanel, String> {
        validate_requests(funds)?;
        // Atomic replacement and bounded validation make a read lock unnecessary.
        // Opening the app must not create folders, locks, or repair invalid data.
        let cache = read_cache_file(&self.cache_path)?.unwrap_or_default();
        let mut panel = build_panel(&cache, funds, &[], false);
        for item in &mut panel.items {
            if item.report.is_some() {
                item.status = "cached".into();
            }
        }
        Ok(panel)
    }

    pub async fn refresh(&self, funds: &[CvmFundRequest]) -> Result<CvmPanel, String> {
        self.refresh_with_mode(funds, RefreshMode::Manual).await
    }

    pub async fn refresh_if_due(&self, funds: &[CvmFundRequest]) -> RefreshOutcome<CvmPanel> {
        if funds.is_empty() {
            return RefreshOutcome::cached(CvmPanel::default());
        }
        match self.refresh_with_mode(funds, RefreshMode::Automatic).await {
            Ok(data)
                if !data.errors.is_empty()
                    || data.items.iter().any(|item| item.error.is_some()) =>
            {
                let message = data
                    .errors
                    .iter()
                    .cloned()
                    .chain(data.items.iter().filter_map(|item| item.error.clone()))
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(" ");
                let status = if data.items.iter().any(|item| item.report.is_some()) {
                    "cached"
                } else {
                    "unavailable"
                };
                RefreshOutcome {
                    data: Some(data),
                    status: status.into(),
                    message: Some(message),
                }
            }
            Ok(data) if data.items.iter().any(|item| item.status == "available") => {
                RefreshOutcome::updated(data)
            }
            Ok(data) => RefreshOutcome::cached(data),
            Err(error) => RefreshOutcome::failed(self.read_cache(funds).ok(), error),
        }
    }

    async fn refresh_with_mode(
        &self,
        funds: &[CvmFundRequest],
        mode: RefreshMode,
    ) -> Result<CvmPanel, String> {
        validate_requests(funds)?;
        let _refresh = self.refresh_lock.lock().await;
        // Other desktop/MCP processes also re-check the dated cache after this
        // lock. Waiting uses try_lock and an async timer, never a blocking lock.
        let _network = network_lock(&self.cache_path).await?;
        let (mut cache, cache_error) = {
            let _lock = lock_cache(&self.cache_path)?;
            match read_cache_file(&self.cache_path) {
                Ok(cache) => (cache.unwrap_or_default(), None),
                Err(error) => (Cache::default(), Some(error)),
            }
        };
        // Missing or invalid mappings never cause a network request.
        let cnps: BTreeSet<_> = funds
            .iter()
            .filter_map(|fund| {
                fund.cnpj
                    .as_deref()
                    .and_then(|raw| normalize_cnpj(raw).ok())
            })
            .collect();
        if cnps.is_empty() {
            return Ok(build_panel(
                &cache,
                funds,
                &cache_error.into_iter().collect::<Vec<_>>(),
                false,
            ));
        }
        let now = Utc::now();
        let reuse = |value: &str| match mode {
            RefreshMode::Manual => fresh(value, now),
            RefreshMode::Automatic => crate::refresh_policy::fresh(Some(value), 24 * 3600, now),
        };
        let year = brasilia_date(now).year();
        let mut errors = Vec::new();
        let mut changed = false;
        if cache.register.as_ref().is_none_or(|register| {
            !reuse(&register.fetched_at)
                || (register.other_funds.is_empty() && register.fiagro_funds.is_empty())
                || cnps.iter().any(|cnpj| {
                    !register.fiagro_funds.contains_key(cnpj)
                        && register
                            .other_funds
                            .get(cnpj)
                            .is_some_and(|kind| fiagro::is_type(kind))
                })
        }) {
            match self
                .download(REGISTER_URL)
                .await
                .and_then(|bytes| parse_register(&bytes, now))
            {
                Ok(register) => {
                    cache.register = Some(register);
                    changed = true;
                }
                Err(error) => errors.push(format!("Cadastro CVM: {error}")),
            }
        }
        // Each registered type uses its own official archive and parser.
        let requires_reports = cache
            .register
            .as_ref()
            .is_some_and(|register| cnps.iter().any(|cnpj| register.funds.contains_key(cnpj)));
        if requires_reports {
            if cache
                .years
                .get(&year)
                .is_none_or(|annual| !reuse(&annual.fetched_at))
            {
                match self
                    .download_year(
                        year,
                        now,
                        cache.register.as_ref().expect("validated register"),
                    )
                    .await
                {
                    Ok(annual) => {
                        cache.years.insert(year, annual);
                        changed = true;
                    }
                    Err(error) => errors.push(format!("Informe CVM {year}: {error}")),
                }
            }
            // January and funds whose latest report predates the current year
            // still need the previous archive. The same cache is reused later.
            let needs_previous = cnps.iter().any(|cnpj| {
                cache
                    .register
                    .as_ref()
                    .is_some_and(|r| r.funds.contains_key(cnpj))
                    && !cache
                        .years
                        .get(&year)
                        .is_some_and(|a| a.reports.contains_key(cnpj))
            });
            if needs_previous
                && cache
                    .years
                    .get(&(year - 1))
                    .is_none_or(|annual| !reuse(&annual.fetched_at))
            {
                match self
                    .download_year(
                        year - 1,
                        now,
                        cache.register.as_ref().expect("validated register"),
                    )
                    .await
                {
                    Ok(annual) => {
                        cache.years.insert(year - 1, annual);
                        changed = true;
                    }
                    Err(error) => errors.push(format!("Informe CVM {}: {error}", year - 1)),
                }
            }
        }
        let fiagro_cnps: BTreeSet<_> = cache
            .register
            .as_ref()
            .map(|register| {
                cnps.iter()
                    .filter(|cnpj| register.fiagro_funds.contains_key(*cnpj))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if !fiagro_cnps.is_empty() {
            // Start at the last complete calendar month. If publication is late
            // or a requested class is missing, look back within the twelve
            // official months, stopping as soon as every requested class exists.
            let mut unresolved = fiagro_cnps;
            for month in fiagro::recent_months(now, 12) {
                if cache
                    .fiagro_months
                    .get(&month)
                    .is_none_or(|monthly| !reuse(&monthly.fetched_at))
                {
                    match self
                        .download(&fiagro::url(&month))
                        .await
                        .and_then(|bytes| fiagro::parse(&bytes, &month, now))
                    {
                        Ok(mut monthly) => {
                            fiagro::resolve_names(
                                &mut monthly,
                                cache.register.as_ref().expect("validated register"),
                            );
                            cache.fiagro_months.insert(month.clone(), monthly);
                            changed = true;
                        }
                        Err(error) => errors.push(format!("Informe FIAGRO {month}: {error}")),
                    }
                }
                if let Some(monthly) = cache.fiagro_months.get(&month) {
                    for cnpj in &unresolved {
                        if monthly.rejected.contains_key(cnpj) {
                            errors.push(format!("Informe FIAGRO {month}: a classe {cnpj} possui registros conflitantes na mesma versão e data de entrega; esta competência foi desconsiderada."));
                        }
                    }
                    unresolved.retain(|cnpj| !monthly.reports.contains_key(cnpj));
                }
                if unresolved.is_empty() {
                    break;
                }
            }
        }
        cache
            .years
            .retain(|key, _| *key >= year - 1 && *key <= year);
        if changed {
            let _lock = lock_cache(&self.cache_path)?;
            // Keep newer archives another process may have written meanwhile.
            if let Ok(Some(previous)) = read_cache_file(&self.cache_path) {
                merge_newer(&mut cache, previous);
            }
            cache
                .years
                .retain(|key, _| *key >= year - 1 && *key <= year);
            let retained_months = fiagro::recent_months(now, 12);
            cache
                .fiagro_months
                .retain(|month, _| retained_months.contains(month));
            write_cache(&self.cache_path, &cache)?;
        } else if let Some(error) = cache_error {
            // Preserve a corrupt file when there was no validated replacement.
            errors.push(error);
        }
        let mut panel = build_panel(&cache, funds, &errors, !errors.is_empty());
        if !changed {
            for item in &mut panel.items {
                if item.status == "available" {
                    item.status = "cached".into();
                }
            }
        }
        Ok(panel)
    }

    async fn download_year(
        &self,
        year: i32,
        now: DateTime<Utc>,
        register: &Register,
    ) -> Result<AnnualReports, String> {
        let bytes = self.download(&annual_url(year)).await?;
        let mut annual = parse_annual(&bytes, year, now)?;
        resolve_names(&mut annual, register);
        Ok(annual)
    }

    async fn download(&self, url: &str) -> Result<Vec<u8>, String> {
        if url != REGISTER_URL && !valid_annual_url(url) && !fiagro::valid_url(url) {
            return Err("Origem CVM não permitida.".into());
        }
        let mut policy = RefreshGate::acquire(&self.cache_path).await?;
        let key = url
            .rsplit('/')
            .next()
            .expect("fixed CVM URL")
            .replace('.', "_");
        if let Some(message) = policy.reserve(&key, None, 0, Utc::now(), None)? {
            return Err(message);
        }
        let mut response = self.client.get(url).send().await.map_err(|error| {
            if error.is_timeout() {
                "A CVM não respondeu no prazo. Tente novamente depois."
            } else {
                "Não foi possível consultar os dados públicos da CVM."
            }
            .to_string()
        })?;
        if !response.status().is_success() {
            return Err(format!(
                "Arquivo público indisponível (HTTP {}).",
                response.status().as_u16()
            ));
        }
        if response
            .content_length()
            .is_some_and(|size| size > MAX_DOWNLOAD as u64)
        {
            return Err("Arquivo CVM excede o limite de download.".into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "Download CVM interrompido.".to_string())?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_DOWNLOAD {
                return Err("Arquivo CVM excede o limite de download.".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

/// Canonical unformatted CNPJ with check digits, accepting numeric CNPJs and
/// the official alphanumeric scheme (ASCII character value minus 48).
pub fn normalize_cnpj(raw: &str) -> Result<String, String> {
    let mut value = String::new();
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            value.push(ch.to_ascii_uppercase());
        } else if !matches!(ch, '.' | '/' | '-' | ' ' | '\t') {
            return Err("CNPJ do fundo contém caracteres inválidos.".into());
        }
    }
    let bytes = value.as_bytes();
    if bytes.len() != 14
        || !bytes[12..].iter().all(u8::is_ascii_digit)
        || bytes.iter().all(|ch| *ch == bytes[0])
    {
        return Err("Informe um CNPJ válido do fundo ou da classe.".into());
    }
    for (end, weights) in [
        (12, &[5, 4, 3, 2, 9, 8, 7, 6, 5, 4, 3, 2][..]),
        (13, &[6, 5, 4, 3, 2, 9, 8, 7, 6, 5, 4, 3, 2][..]),
    ] {
        let sum: u32 = bytes[..end]
            .iter()
            .zip(weights)
            .map(|(ch, weight)| u32::from(*ch - b'0') * weight)
            .sum();
        let remainder = sum % 11;
        let digit = if remainder < 2 { 0 } else { 11 - remainder };
        if bytes[end] - b'0' != digit as u8 {
            return Err("CNPJ do fundo possui dígitos verificadores inválidos.".into());
        }
    }
    Ok(value)
}

fn validate_requests(funds: &[CvmFundRequest]) -> Result<(), String> {
    if funds.len() > 200 {
        return Err("Consulte no máximo 200 fundos por vez na CVM.".into());
    }
    let mut seen = BTreeSet::new();
    for fund in funds {
        if fund.symbol.is_empty()
            || fund.symbol.len() > 20
            || !fund
                .symbol
                .chars()
                .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
            || !seen.insert(&fund.symbol)
            || fund.cnpj.as_ref().is_some_and(|cnpj| cnpj.len() > 40)
        {
            return Err("Lista de fundos CVM inválida ou com símbolos repetidos.".into());
        }
    }
    Ok(())
}

fn annual_url(year: i32) -> String {
    format!("https://dados.cvm.gov.br/dados/FII/DOC/INF_MENSAL/DADOS/inf_mensal_fii_{year}.zip")
}

fn valid_annual_url(url: &str) -> bool {
    (2016..=2100).any(|year| url == annual_url(year))
}

fn brasilia_date(at: DateTime<Utc>) -> NaiveDate {
    at.with_timezone(&FixedOffset::west_opt(3 * 3600).expect("fixed UTC-3"))
        .date_naive()
}

fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn fresh(value: &str, now: DateTime<Utc>) -> bool {
    DateTime::parse_from_rfc3339(value).is_ok_and(|date| {
        date <= now && now.signed_duration_since(date).num_hours() < REFRESH_HOURS
    })
}

fn decode_csv(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.trim_start_matches('\u{feff}').to_string(),
        Err(_) => bytes.iter().map(|byte| char::from(*byte)).collect(),
    }
}

fn zip_members(bytes: &[u8], required: &[String]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    if bytes.len() > MAX_DOWNLOAD {
        return Err("Arquivo CVM excede o limite de download.".into());
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| "Arquivo CVM não é um ZIP válido.".to_string())?;
    if archive.len() > 32 {
        return Err("Arquivo CVM contém entradas em excesso.".into());
    }
    let mut expanded = 0_u64;
    let mut result = BTreeMap::new();
    for index in 0..archive.len() {
        let mut member = archive
            .by_index(index)
            .map_err(|_| "Não foi possível abrir uma entrada do ZIP CVM.".to_string())?;
        expanded = expanded.saturating_add(member.size());
        if member.size() > MAX_MEMBER || expanded > MAX_EXPANDED || member.encrypted() {
            return Err("ZIP CVM excede os limites seguros de leitura.".into());
        }
        let name = member.name().to_string();
        // Nothing is extracted to disk; only exact canonical filenames are read.
        if required.contains(&name) {
            if result.contains_key(&name) {
                return Err("ZIP CVM contém uma tabela duplicada.".into());
            }
            let mut data = Vec::new();
            (&mut member)
                .take(MAX_MEMBER + 1)
                .read_to_end(&mut data)
                .map_err(|_| "Tabela CVM comprimida está corrompida.".to_string())?;
            if data.len() as u64 > MAX_MEMBER || data.len() as u64 != member.size() {
                return Err("Tabela CVM excede o tamanho declarado.".into());
            }
            result.insert(name, data);
        }
    }
    if required.iter().any(|name| !result.contains_key(name)) {
        return Err(
            "ZIP CVM não contém as tabelas esperadas; verifique atualização do formato.".into(),
        );
    }
    Ok(result)
}

struct Table {
    names: BTreeMap<String, usize>,
    rows: Vec<csv::StringRecord>,
}

impl Table {
    fn read(bytes: &[u8], required: &[&str]) -> Result<Self, String> {
        let text = decode_csv(bytes);
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(b';')
            .from_reader(text.as_bytes());
        let headers = reader
            .headers()
            .map_err(|_| "Cabeçalho CSV da CVM inválido.".to_string())?;
        if headers.len() > 256 || headers.iter().any(|name| name.len() > 256) {
            return Err("Cabeçalho CSV da CVM excede os limites de leitura.".into());
        }
        let names: BTreeMap<_, _> = headers
            .iter()
            .enumerate()
            .map(|(index, name)| (name.to_string(), index))
            .collect();
        if names.len() != headers.len() || required.iter().any(|name| !names.contains_key(*name)) {
            return Err(
                "Colunas obrigatórias da CVM ausentes ou duplicadas; formato não reconhecido."
                    .into(),
            );
        }
        let mut rows = Vec::new();
        for row in reader.records() {
            let row = row.map_err(|_| "CSV da CVM possui uma linha malformada.".to_string())?;
            if rows.len() >= MAX_ROWS || row.iter().any(|field| field.len() > 8192) {
                return Err("CSV da CVM excede os limites de leitura.".into());
            }
            rows.push(row);
        }
        Ok(Self { names, rows })
    }

    fn get<'a>(&self, row: &'a csv::StringRecord, name: &str) -> &'a str {
        self.names
            .get(name)
            .and_then(|index| row.get(*index))
            .unwrap_or("")
            .trim()
    }
}

fn parse_register(bytes: &[u8], at: DateTime<Utc>) -> Result<Register, String> {
    let names = vec![
        "registro_fundo.csv".to_string(),
        "registro_classe.csv".to_string(),
    ];
    let members = zip_members(bytes, &names)?;
    struct Candidate {
        rank: (NaiveDate, bool, u8),
        name: String,
        fund_type: String,
        is_fii: bool,
        is_fiagro: bool,
    }
    let mut candidates: BTreeMap<String, Candidate> = BTreeMap::new();
    for (filename, cnpj_column, type_column, allowed, priority) in [
        ("registro_fundo.csv", "CNPJ_Fundo", "Tipo_Fundo", "FII", 1),
        (
            "registro_classe.csv",
            "CNPJ_Classe",
            "Tipo_Classe",
            "Classes de Cotas de Fundos FII",
            2,
        ),
    ] {
        let table = Table::read(
            &members[filename],
            &[
                cnpj_column,
                type_column,
                "Denominacao_Social",
                "Situacao",
                "Data_Inicio_Situacao",
            ],
        )?;
        for row in &table.rows {
            if let Ok(cnpj) = normalize_cnpj(table.get(row, cnpj_column)) {
                let fund_type = table.get(row, type_column);
                let name = table.get(row, "Denominacao_Social");
                if fund_type.is_empty()
                    || fund_type.len() > 256
                    || name.is_empty()
                    || name.len() > 1024
                {
                    continue;
                }
                let status_date = table.get(row, "Data_Inicio_Situacao");
                let status_date = if status_date.is_empty() {
                    NaiveDate::MIN
                } else {
                    NaiveDate::parse_from_str(status_date, "%Y-%m-%d")
                        .map_err(|_| "Data de situação inválida no cadastro CVM.".to_string())?
                };
                let active = !table
                    .get(row, "Situacao")
                    .to_ascii_lowercase()
                    .contains("cancel");
                let candidate = Candidate {
                    rank: (status_date, active, priority),
                    name: name.into(),
                    fund_type: fund_type.into(),
                    is_fii: fund_type == allowed,
                    is_fiagro: active && fiagro::is_type(fund_type),
                };
                // The same CNPJ may have a historical canceled FII row and a
                // current FIAGRO row. Use dated registration status, with the
                // adapted class taking priority over its old fund record.
                if candidates
                    .get(&cnpj)
                    .is_none_or(|previous| candidate.rank > previous.rank)
                {
                    candidates.insert(cnpj, candidate);
                }
            }
        }
    }
    let mut funds = BTreeMap::new();
    let mut fiagro_funds = BTreeMap::new();
    let mut other_funds = BTreeMap::new();
    for (cnpj, candidate) in candidates {
        if candidate.is_fii {
            funds.insert(cnpj, candidate.name);
        } else if candidate.is_fiagro {
            fiagro_funds.insert(cnpj, candidate.name);
        } else {
            other_funds.insert(cnpj, candidate.fund_type);
        }
    }
    if funds.is_empty() && fiagro_funds.is_empty() {
        return Err("Cadastro CVM recebido sem fundos FII/FIAGRO válidos.".into());
    }
    Ok(Register {
        fetched_at: stamp(at),
        funds,
        fiagro_funds,
        other_funds,
    })
}

fn date(value: &str, at: DateTime<Utc>) -> Result<String, String> {
    let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| "Data inválida no informe mensal CVM.".to_string())?;
    if parsed.format("%Y-%m-%d").to_string() != value || parsed > brasilia_date(at) {
        return Err("Informe CVM contém uma data futura ou inválida.".into());
    }
    Ok(value.to_string())
}

fn decimal(value: &str, allow_negative: bool) -> Result<Option<String>, String> {
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > 96 || value.contains(',') || value.chars().any(char::is_whitespace) {
        return Err("Valor decimal inválido no informe mensal CVM.".into());
    }
    let parsed = if value.contains(['e', 'E']) {
        Decimal::from_scientific(value)
    } else {
        Decimal::from_str_exact(value)
    }
    .map_err(|_| "Valor decimal inválido no informe mensal CVM.".to_string())?;
    if !allow_negative && parsed.is_sign_negative() {
        return Err("Quantidade negativa no informe mensal CVM.".into());
    }
    Ok(Some(parsed.normalize().to_string()))
}

fn integer(value: &str) -> Result<Option<u64>, String> {
    if value.is_empty() {
        return Ok(None);
    }
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("Número de cotistas inválido no informe CVM.".into());
    }
    value
        .parse()
        .map(Some)
        .map_err(|_| "Número de cotistas excede o limite de leitura.".into())
}

fn row_key(
    table: &Table,
    row: &csv::StringRecord,
    year: i32,
    at: DateTime<Utc>,
) -> Result<(String, String, u32), String> {
    let cnpj = normalize_cnpj(table.get(row, "CNPJ_Fundo_Classe"))?;
    let reference = date(table.get(row, "Data_Referencia"), at)?;
    let parsed = NaiveDate::parse_from_str(&reference, "%Y-%m-%d").expect("validated date");
    if parsed.year() != year || parsed.day() != 1 {
        return Err("Competência CVM não corresponde ao arquivo anual.".into());
    }
    let version: u32 = table
        .get(row, "Versao")
        .parse()
        .map_err(|_| "Versão inválida no informe CVM.".to_string())?;
    if version == 0 || version > 255 {
        return Err("Versão inválida no informe CVM.".into());
    }
    Ok((cnpj, reference, version))
}

fn parse_annual(bytes: &[u8], year: i32, at: DateTime<Utc>) -> Result<AnnualReports, String> {
    let general_name = format!("inf_mensal_fii_geral_{year}.csv");
    let extra_name = format!("inf_mensal_fii_complemento_{year}.csv");
    let members = zip_members(bytes, &[general_name.clone(), extra_name.clone()])?;
    let general = Table::read(
        &members[&general_name],
        &[
            "CNPJ_Fundo_Classe",
            "Data_Referencia",
            "Versao",
            "Data_Entrega",
            "Nome_Fundo_Classe",
            "Quantidade_Cotas_Emitidas",
        ],
    )?;
    let extra = Table::read(
        &members[&extra_name],
        &[
            "CNPJ_Fundo_Classe",
            "Data_Referencia",
            "Versao",
            "Patrimonio_Liquido",
            "Valor_Patrimonial_Cotas",
            "Total_Numero_Cotistas",
            "Cotas_Emitidas",
        ],
    )?;
    let mut reports: BTreeMap<String, StoredReport> = BTreeMap::new();
    for row in &general.rows {
        let (cnpj, reference, version) = row_key(&general, row, year, at)?;
        let name = general.get(row, "Nome_Fundo_Classe");
        if name.len() > 1024 {
            return Err("Nome de fundo inválido no informe CVM.".into());
        }
        let filed = general.get(row, "Data_Entrega");
        let filed_at = if filed.is_empty() {
            None
        } else {
            Some(date(filed, at)?)
        };
        if filed_at.as_ref().is_some_and(|filed| filed < &reference) {
            return Err("Data de entrega anterior à competência no informe CVM.".into());
        }
        let candidate = StoredReport {
            version,
            report: CvmFundReport {
                fund_type: default_fii(),
                cnpj: cnpj.clone(),
                fund_name: name.into(),
                reference_date: reference,
                filed_at,
                net_assets: None,
                equity_per_share: None,
                shareholders: None,
                issued_shares: decimal(general.get(row, "Quantidade_Cotas_Emitidas"), false)?,
                source: SOURCE.into(),
                source_url: annual_url(year),
                fetched_at: stamp(at),
            },
        };
        match reports.get(&cnpj) {
            Some(previous) if report_order(previous) > report_order(&candidate) => {}
            Some(previous) if report_order(previous) == report_order(&candidate) => {
                // Ambiguous duplicate metadata must not pick an arbitrary name.
                if previous.report != candidate.report {
                    return Err("Informe CVM contém versões duplicadas conflitantes.".into());
                }
            }
            _ => {
                reports.insert(cnpj, candidate);
            }
        }
    }
    let mut enriched = BTreeSet::new();
    for row in &extra.rows {
        let (cnpj, reference, version) = row_key(&extra, row, year, at)?;
        if let Some(stored) = reports.get_mut(&cnpj) {
            // Never mix one version's metadata with another version's finances.
            if stored.report.reference_date == reference && stored.version == version {
                if !enriched.insert(cnpj) {
                    return Err(
                        "Informe CVM contém complemento duplicado para a mesma versão.".into(),
                    );
                }
                stored.report.net_assets = decimal(extra.get(row, "Patrimonio_Liquido"), true)?;
                stored.report.equity_per_share =
                    decimal(extra.get(row, "Valor_Patrimonial_Cotas"), true)?;
                stored.report.shareholders = integer(extra.get(row, "Total_Numero_Cotistas"))?;
                if let Some(shares) = decimal(extra.get(row, "Cotas_Emitidas"), false)? {
                    stored.report.issued_shares = Some(shares);
                }
            }
        }
    }
    if reports.is_empty() {
        return Err("Arquivo anual CVM não contém informes válidos.".into());
    }
    Ok(AnnualReports {
        fetched_at: stamp(at),
        reports,
        rejected: BTreeMap::new(),
    })
}

fn report_order(value: &StoredReport) -> (&str, u32, Option<&str>) {
    (
        &value.report.reference_date,
        value.version,
        value.report.filed_at.as_deref(),
    )
}

fn resolve_names(annual: &mut AnnualReports, register: &Register) {
    annual.reports.retain(|cnpj, stored| {
        let Some(official_name) = register.funds.get(cnpj) else {
            return false;
        };
        // A real CVM 2026 row has an empty name. Its CNPJ remains the join key;
        // never infer identity from a similar name or from the trading ticker.
        if stored.report.fund_name.is_empty() {
            stored.report.fund_name = official_name.clone();
        }
        true
    });
}

fn build_panel(
    cache: &Cache,
    funds: &[CvmFundRequest],
    errors: &[String],
    failed: bool,
) -> CvmPanel {
    let mut panel = CvmPanel {
        items: Vec::new(),
        fetched_at: None,
        errors: errors.to_vec(),
    };
    for fund in funds {
        let mut item = CvmFundItem {
            symbol: fund.symbol.clone(),
            cnpj: None,
            status: "mapping_required".into(),
            report: None,
            error: Some("Vincule o ticker ao CNPJ oficial do fundo nas configurações.".into()),
        };
        let Some(raw) = fund.cnpj.as_deref().filter(|raw| !raw.trim().is_empty()) else {
            panel.items.push(item);
            continue;
        };
        let cnpj = match normalize_cnpj(raw) {
            Ok(cnpj) => cnpj,
            Err(error) => {
                item.status = "invalid_mapping".into();
                item.error = Some(error);
                panel.items.push(item);
                continue;
            }
        };
        item.cnpj = Some(cnpj.clone());
        let is_fiagro = cache
            .register
            .as_ref()
            .is_some_and(|register| register.fiagro_funds.contains_key(&cnpj));
        if let Some(fund_type) = cache
            .register
            .as_ref()
            .and_then(|register| register.other_funds.get(&cnpj))
        {
            item.status = "unsupported".into();
            item.error = Some(format!("Tipo não atendido pelo cadastro deste cache: {fund_type}. Atualize os informes para verificar classes FII/FIAGRO; outros tipos não são suportados."));
            panel.items.push(item);
            continue;
        }
        if cache
            .register
            .as_ref()
            .is_some_and(|register| !register.funds.contains_key(&cnpj) && !is_fiagro)
        {
            item.status = "not_found".into();
            item.error = Some("CNPJ não consta como FII ou FIAGRO ativo no cadastro oficial da CVM. Confira o vínculo.".into());
            panel.items.push(item);
            continue;
        }
        let latest = cache
            .years
            .values()
            .filter(|_| !is_fiagro)
            .chain(cache.fiagro_months.values().filter(|_| is_fiagro))
            .filter_map(|annual| annual.reports.get(&cnpj))
            .filter(|stored| stored.report.fund_type == if is_fiagro { "FIAGRO" } else { "FII" })
            .max_by(|a, b| report_order(a).cmp(&report_order(b)));
        let ambiguous_reference = cache
            .fiagro_months
            .iter()
            .filter(|(_, monthly)| is_fiagro && monthly.rejected.contains_key(&cnpj))
            .map(|(month, _)| format!("{}-{}-01", &month[..4], &month[4..]))
            .max();
        let ambiguous_latest = ambiguous_reference.as_ref().is_some_and(|reference| {
            latest.is_none_or(|stored| reference >= &stored.report.reference_date)
        });
        if let Some(stored) = latest.filter(|_| cache.register.is_some()) {
            item.status = if failed || ambiguous_latest {
                "cached"
            } else {
                "available"
            }
            .into();
            item.report = Some(stored.report.clone());
            item.error = if ambiguous_latest {
                Some("A competência FIAGRO mais recente possui registros conflitantes; exibindo o último informe validado, com sua competência e data de consulta.".into())
            } else if failed {
                Some("A atualização falhou; exibindo o último informe validado em cache, com sua competência e data de consulta.".into())
            } else {
                None
            };
            if panel
                .fetched_at
                .as_ref()
                .is_none_or(|at| at < &stored.report.fetched_at)
            {
                panel.fetched_at = Some(stored.report.fetched_at.clone());
            }
        } else {
            item.status = if failed || ambiguous_latest {
                "unavailable"
            } else {
                "not_found"
            }
            .into();
            item.error = Some(
                if cache.register.is_none() {
                    "Cadastro CVM ainda não disponível. Atualize os dados para validar este CNPJ."
                } else if ambiguous_latest {
                    "A classe FIAGRO possui registros conflitantes na mesma versão e data de entrega; nenhum informe válido está disponível em cache."
                } else if is_fiagro {
                    "Nenhum informe FIAGRO encontrado nas até doze competências anteriores consultadas."
                } else {
                    "Nenhum informe mensal encontrado para este CNPJ no ano atual ou anterior."
                }
                .into(),
            );
        }
        panel.items.push(item);
    }
    panel
}

fn merge_newer(next: &mut Cache, previous: Cache) {
    if let Some(register) = previous.register {
        if next
            .register
            .as_ref()
            .is_none_or(|value| value.fetched_at < register.fetched_at)
        {
            next.register = Some(register);
        }
    }
    for (year, annual) in previous.years {
        if next
            .years
            .get(&year)
            .is_none_or(|value| value.fetched_at < annual.fetched_at)
        {
            next.years.insert(year, annual);
        }
    }
    for (month, monthly) in previous.fiagro_months {
        if next
            .fiagro_months
            .get(&month)
            .is_none_or(|value| value.fetched_at < monthly.fetched_at)
        {
            next.fiagro_months.insert(month, monthly);
        }
    }
}

fn refresh_gate(path: &Path) -> Result<Arc<Mutex<()>>, String> {
    let mut gates = REFRESH_GATES
        .get_or_init(|| StdMutex::new(BTreeMap::new()))
        .lock()
        .map_err(|_| "Controle de consultas CVM indisponível.".to_string())?;
    gates.retain(|_, gate| gate.strong_count() > 0);
    if let Some(gate) = gates.get(path).and_then(Weak::upgrade) {
        return Ok(gate);
    }
    let gate = Arc::new(Mutex::new(()));
    gates.insert(path.to_path_buf(), Arc::downgrade(&gate));
    Ok(gate)
}

async fn network_lock(path: &Path) -> Result<File, String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|_| "Não foi possível criar a pasta de cache CVM.".to_string())?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("refresh.lock"))
        .map_err(|_| "Não foi possível abrir o controle de consultas CVM.".to_string())?;
    for _ in 0..300 {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) => {
                tokio::time::sleep(Duration::from_millis(100)).await
            }
            Err(_) => {
                return Err("Não foi possível bloquear as consultas CVM com segurança.".into())
            }
        }
    }
    Err("Outra atualização da CVM está em andamento. Aguarde e tente novamente.".into())
}

fn lock_cache(path: &Path) -> Result<File, String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|_| "Não foi possível criar a pasta de cache CVM.".to_string())?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("lock"))
        .map_err(|_| "Não foi possível abrir o controle do cache CVM.".to_string())?;
    file.lock()
        .map_err(|_| "Não foi possível bloquear o cache CVM.".to_string())?;
    Ok(file)
}

fn read_cache_file(path: &Path) -> Result<Option<Cache>, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Não foi possível ler o cache CVM; arquivo preservado.".into()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_CACHE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Não foi possível ler o cache CVM; arquivo preservado.".to_string())?;
    if bytes.len() as u64 > MAX_CACHE {
        return Err("Cache CVM excede o limite de tamanho; arquivo preservado.".into());
    }
    let cache: Cache = serde_json::from_slice(&bytes)
        .map_err(|_| "Cache CVM inválido; arquivo preservado.".to_string())?;
    validate_cache(&cache)?;
    Ok(Some(cache))
}

fn validate_cache(cache: &Cache) -> Result<(), String> {
    let now = Utc::now();
    if cache.version != CACHE_VERSION || cache.years.len() > 3 || cache.fiagro_months.len() > 12 {
        return Err("Versão ou conteúdo do cache CVM não suportado.".into());
    }
    let valid_stamp = |value: &str| {
        DateTime::parse_from_rfc3339(value).is_ok_and(|at| at <= now + chrono::Duration::minutes(5))
    };
    if let Some(register) = &cache.register {
        if !valid_stamp(&register.fetched_at)
            || register.funds.len() > 30_000
            || (register.funds.is_empty() && register.fiagro_funds.is_empty())
            || register.fiagro_funds.len() > 30_000
            || register.other_funds.len() > MAX_ROWS
        {
            return Err("Cadastro CVM em cache inválido.".into());
        }
        for (cnpj, name) in &register.funds {
            if normalize_cnpj(cnpj).as_deref() != Ok(cnpj.as_str())
                || name.is_empty()
                || name.len() > 1024
            {
                return Err("Cadastro CVM em cache contém identificação inválida.".into());
            }
        }
        for (cnpj, fund_type) in &register.other_funds {
            if normalize_cnpj(cnpj).as_deref() != Ok(cnpj.as_str())
                || register.funds.contains_key(cnpj)
                || register.fiagro_funds.contains_key(cnpj)
                || fund_type.is_empty()
                || fund_type.len() > 256
            {
                return Err("Tipo de fundo no cadastro CVM em cache inválido.".into());
            }
        }
        for (cnpj, name) in &register.fiagro_funds {
            if normalize_cnpj(cnpj).as_deref() != Ok(cnpj.as_str())
                || register.funds.contains_key(cnpj)
                || name.is_empty()
                || name.len() > 1024
            {
                return Err("Cadastro FIAGRO em cache contém identificação inválida.".into());
            }
        }
    }
    for (year, annual) in &cache.years {
        if !(2016..=brasilia_date(now).year()).contains(year)
            || !valid_stamp(&annual.fetched_at)
            || annual.reports.len() > 30_000
            || !annual.rejected.is_empty()
        {
            return Err("Arquivo anual CVM em cache inválido.".into());
        }
        for (cnpj, stored) in &annual.reports {
            let report = &stored.report;
            let reference = date(&report.reference_date, now)?;
            let reference =
                NaiveDate::parse_from_str(&reference, "%Y-%m-%d").expect("validated date");
            let fetched = DateTime::parse_from_rfc3339(&report.fetched_at)
                .map_err(|_| "Data de consulta do informe CVM em cache inválida.".to_string())?
                .with_timezone(&Utc);
            if normalize_cnpj(cnpj).as_deref() != Ok(cnpj.as_str())
                || report.cnpj != *cnpj
                || reference.year() != *year
                || reference.day() != 1
                || report.fund_name.is_empty()
                || report.fund_name.len() > 1024
                || stored.version == 0
                || stored.version > 255
                || report.fund_type != "FII"
                || report.source != SOURCE
                || report.source_url != annual_url(*year)
                || report.fetched_at != annual.fetched_at
                || !valid_stamp(&report.fetched_at)
                || reference > brasilia_date(fetched)
            {
                return Err("Informe CVM em cache contém origem ou identificação inválida.".into());
            }
            if let Some(filed) = &report.filed_at {
                date(filed, fetched)?;
                if filed < &report.reference_date {
                    return Err("Data de entrega CVM em cache inválida.".into());
                }
            }
            for value in [&report.net_assets, &report.equity_per_share]
                .into_iter()
                .flatten()
            {
                if decimal(value, true)?.as_ref() != Some(value) {
                    return Err("Valor financeiro CVM em cache inválido.".into());
                }
            }
            if let Some(value) = &report.issued_shares {
                if decimal(value, false)?.as_ref() != Some(value) {
                    return Err("Quantidade de cotas CVM em cache inválida.".into());
                }
            }
        }
    }
    for (month, monthly) in &cache.fiagro_months {
        fiagro::validate_monthly(month, monthly, now)?;
    }
    Ok(())
}

fn write_cache(path: &Path, cache: &Cache) -> Result<(), String> {
    validate_cache(cache)?;
    let bytes = serde_json::to_vec(cache)
        .map_err(|_| "Não foi possível serializar o cache CVM.".to_string())?;
    if bytes.len() as u64 > MAX_CACHE {
        return Err("Cache CVM excede o limite de tamanho.".into());
    }
    let temporary = path.with_extension(format!("{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|_| "Não foi possível criar o cache CVM temporário.".to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "Não foi possível gravar o cache CVM temporário.".to_string())?;
        drop(file);
        fs::rename(&temporary, path).map_err(|_| {
            "Não foi possível substituir o cache CVM; arquivo anterior preservado.".to_string()
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;

    const CNPJ: &str = "00332266000131";
    fn at() -> DateTime<Utc> {
        "2026-10-19T12:00:00Z".parse().unwrap()
    }

    fn archive(files: &[(&str, &str)]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, content) in files {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn fixture(general: &str, extra: &str) -> Vec<u8> {
        let g = format!("CNPJ_Fundo_Classe;Data_Referencia;Versao;Data_Entrega;Nome_Fundo_Classe;Quantidade_Cotas_Emitidas\n{general}");
        let e = format!("CNPJ_Fundo_Classe;Data_Referencia;Versao;Patrimonio_Liquido;Valor_Patrimonial_Cotas;Total_Numero_Cotistas;Cotas_Emitidas\n{extra}");
        archive(&[
            ("inf_mensal_fii_geral_2026.csv", &g),
            ("inf_mensal_fii_complemento_2026.csv", &e),
        ])
    }

    fn annual() -> AnnualReports {
        parse_annual(
            &fixture(
                "00.332.266/0001-31;2026-09-01;1;2026-10-10;Fundo Oficial;100\n",
                "00.332.266/0001-31;2026-09-01;1;1234.5600;12.3456;7;100\n",
            ),
            2026,
            at(),
        )
        .unwrap()
    }

    fn cache() -> Cache {
        Cache {
            version: CACHE_VERSION,
            register: Some(Register {
                fetched_at: stamp(at()),
                funds: BTreeMap::from([(CNPJ.into(), "Fundo Oficial".into())]),
                fiagro_funds: BTreeMap::new(),
                other_funds: BTreeMap::from([("26502794000185".into(), "FIAGRO".into())]),
            }),
            years: BTreeMap::from([(2026, annual())]),
            fiagro_months: BTreeMap::new(),
        }
    }

    fn request(cnpj: Option<&str>) -> Vec<CvmFundRequest> {
        vec![CvmFundRequest {
            symbol: "FVPQ11".into(),
            cnpj: cnpj.map(str::to_string),
        }]
    }

    const AGRO_CNPJ: &str = "26502794000185";

    fn fiagro_fixture(rows: &str, subclasses: &str) -> Vec<u8> {
        let main = format!("CNPJ_Classe;Nome_Classe;Data_Referencia;Data_Entrega;Versao;Classe_Unica;Numero_Cotistas;Patrimonio_Liquido;Cotas_Emitidas;Valor_Patrimonial_Cotas\n{rows}");
        let sub = format!("CNPJ_Classe;Nome_Classe;Data_Referencia;Nome_Subclasse;Numero_Cotas;Valor_Patrimonial_Cota\n{subclasses}");
        archive(&[
            ("inf_mensal_fiagro_202609.csv", &main),
            ("inf_mensal_fiagro_subclasse_202609.csv", &sub),
        ])
    }

    fn monthly() -> AnnualReports {
        fiagro::parse(&fiagro_fixture(
            "26502794000185;Classe Agro Sintetica;2026-09-01;2026-10-15;1;N;7;1234.5600;100;12.3456\n",
            "26502794000185;Classe Agro Sintetica;2026-09-01;Subclasse A;100;12.3456\n",
        ), "202609", at()).unwrap()
    }

    fn fiagro_cache() -> Cache {
        let mut value = cache();
        let register = value.register.as_mut().unwrap();
        register.other_funds.remove(AGRO_CNPJ);
        register
            .fiagro_funds
            .insert(AGRO_CNPJ.into(), "Classe Agro Sintetica".into());
        value.fiagro_months.insert("202609".into(), monthly());
        value
    }

    #[test]
    fn fiagro_class_report_preserves_published_values_type_source_and_dates() {
        let monthly = monthly();
        let report = &monthly.reports[AGRO_CNPJ].report;
        assert_eq!(report.fund_type, "FIAGRO");
        assert_eq!(report.reference_date, "2026-09-01");
        assert_eq!(report.filed_at.as_deref(), Some("2026-10-15"));
        assert_eq!(report.net_assets.as_deref(), Some("1234.56"));
        assert_eq!(report.equity_per_share.as_deref(), Some("12.3456"));
        assert_eq!(report.issued_shares.as_deref(), Some("100"));
        assert_eq!(report.shareholders, Some(7));
        assert_eq!(report.source, fiagro::SOURCE);
        assert_eq!(report.source_url, fiagro::url("202609"));
        assert_eq!(report.fetched_at, stamp(at()));
    }

    #[test]
    fn fiagro_revision_and_latest_filing_are_selected_without_crossing_class_identity() {
        let archive = fiagro_fixture(
            "26502794000185;Original;2026-09-01;2026-10-10;1;N;2;999;100;9.99\n26502794000185;Revisto;2026-09-01;2026-10-12;2;N;7;1234.56;100;12.3456\n26502794000185;Nova entrega;2026-09-01;2026-10-15;2;N;8;1234.56;100;12.3456\n",
            "26502794000185;Classe;2026-09-01;Subclasse A;100;12.3456\n",
        );
        let result = fiagro::parse(&archive, "202609", at()).unwrap();
        let stored = &result.reports[AGRO_CNPJ];
        assert_eq!(stored.version, 2);
        assert_eq!(stored.report.fund_name, "Nova entrega");
        assert_eq!(stored.report.shareholders, Some(8));
        let mut missing = result;
        fiagro::resolve_names(&mut missing, cache().register.as_ref().unwrap());
        assert!(missing.reports.is_empty());
    }

    #[test]
    fn fiagro_subclasses_never_supply_or_choose_an_unmapped_share_value() {
        let general = "26502794000185;Classe;2026-09-01;2026-10-15;1;N;7;1234.56;100;12.3456\n";
        let multiple = "26502794000185;Classe;2026-09-01;Subclasse A;40;9\n26502794000185;Classe;2026-09-01;Subclasse B;60;14\n";
        let result = fiagro::parse(&fiagro_fixture(general, multiple), "202609", at()).unwrap();
        assert!(result.reports[AGRO_CNPJ].report.equity_per_share.is_none());
        assert_eq!(
            result.reports[AGRO_CNPJ].report.net_assets.as_deref(),
            Some("1234.56")
        );
        let unrelated = "00332266000131;Outra classe;2026-09-01;Subclasse A;100;9999\n";
        let result = fiagro::parse(&fiagro_fixture(general, unrelated), "202609", at()).unwrap();
        assert!(result.reports[AGRO_CNPJ].report.equity_per_share.is_none());
        let absent = "26502794000185;Classe;2026-09-01;2026-10-15;1;N;7;;100;\n";
        let one = "26502794000185;Classe;2026-09-01;Subclasse A;100;12.3456\n";
        let result = fiagro::parse(&fiagro_fixture(absent, one), "202609", at()).unwrap();
        assert!(result.reports[AGRO_CNPJ].report.equity_per_share.is_none());
        assert!(result.reports[AGRO_CNPJ].report.net_assets.is_none());
        let conflict = one.replace("12.3456", "9999");
        for subclass in [
            conflict.clone(),
            format!("{one}{conflict}"),
            one.replace(";100;", ";99;"),
        ] {
            let result =
                fiagro::parse(&fiagro_fixture(general, &subclass), "202609", at()).unwrap();
            let report = &result.reports[AGRO_CNPJ].report;
            assert!(report.equity_per_share.is_none());
            assert_eq!(report.net_assets.as_deref(), Some("1234.56"));
            assert_eq!(report.shareholders, Some(7));
            assert_eq!(report.issued_shares.as_deref(), Some("100"));
            assert!(result.rejected.is_empty());
        }
    }

    #[test]
    fn fiagro_rejects_wrong_schema_dates_and_cnpj() {
        let good = "26502794000185;Classe;2026-09-01;2026-10-15;1;N;7;1234;100;12.34\n";
        for broken in [
            good.replace("26502794000185", "26502794000186"),
            good.replace("2026-09-01", "2026-08-01"),
            good.replace("2026-09-01", "2026-09-30"),
            good.replace("2026-10-15", "2026-11-15"),
            good.replace("2026-10-15", "2026-08-15"),
            good.replace(";1;N;", ";0;N;"),
            good.replace(";1;N;", ";1;UNKNOWN;"),
        ] {
            assert!(fiagro::parse(&fiagro_fixture(&broken, ""), "202609", at()).is_err());
        }
        let main = "CNPJ_Fundo_Classe;Nome_Classe;Data_Referencia;Data_Entrega;Versao;Classe_Unica;Numero_Cotistas;Patrimonio_Liquido;Cotas_Emitidas;Valor_Patrimonial_Cotas\n";
        assert!(fiagro::parse(
            &archive(&[
                ("inf_mensal_fiagro_202609.csv", main),
                ("inf_mensal_fiagro_subclasse_202609.csv", "x\n")
            ]),
            "202609",
            at()
        )
        .is_err());
        assert!(!fiagro::valid_url(&fiagro::url("202613")));
        assert!(!fiagro::valid_url(&fiagro::url("202001")));
        assert!(!fiagro::valid_url("http://dados.cvm.gov.br/dados/FIAGRO/DOC/INF_MENSAL/DADOS/inf_mensal_fiagro_202609.zip"));
        assert!(!fiagro::valid_url(
            "https://evil.test/inf_mensal_fiagro_202609.zip"
        ));
    }

    #[test]
    fn fiagro_conflicting_highest_revision_is_isolated_without_hiding_other_classes() {
        let good = "26502794000185;Classe;2026-09-01;2026-10-15;1;N;7;1234;100;12.34\n";
        let stable = "00332266000131;Classe Estavel;2026-09-01;2026-10-15;1;N;17;4567;100;45.67\n";
        let conflict = good.replace(";1234;", ";9999;");
        let subclasses = "26502794000185;Classe;2026-09-01;Unica;100;12.34\n00332266000131;Classe Estavel;2026-09-01;Unica;100;45.67\n";
        let result = fiagro::parse(
            &fiagro_fixture(&format!("{good}{stable}{conflict}{good}"), subclasses),
            "202609",
            at(),
        )
        .unwrap();
        assert!(!result.reports.contains_key(AGRO_CNPJ));
        assert_eq!(result.rejected[AGRO_CNPJ], "conflicting_revision");
        assert_eq!(
            result.reports[CNPJ].report.equity_per_share.as_deref(),
            Some("45.67")
        );
        let mut cached = fiagro_cache();
        let register = cached.register.as_mut().unwrap();
        register.funds.remove(CNPJ);
        register
            .fiagro_funds
            .insert(CNPJ.into(), "Classe Estavel".into());
        cached.fiagro_months.insert("202609".into(), result);
        validate_cache(&cached).unwrap();
        let mut funds = request(Some(AGRO_CNPJ));
        funds.extend(request(Some(CNPJ)));
        let panel = build_panel(&cached, &funds, &[], false);
        assert_eq!(panel.items[0].status, "unavailable");
        assert!(panel.items[0].report.is_none());
        assert!(panel.items[0]
            .error
            .as_deref()
            .unwrap()
            .contains("conflitantes"));
        assert_eq!(panel.items[1].status, "available");
        assert_eq!(panel.items[1].report.as_ref().unwrap().fund_type, "FIAGRO");

        let revised = good.replace(";1;N;", ";2;N;");
        let result = fiagro::parse(
            &fiagro_fixture(&format!("{good}{conflict}{revised}"), subclasses),
            "202609",
            at(),
        )
        .unwrap();
        assert!(result.rejected.is_empty());
        assert_eq!(result.reports[AGRO_CNPJ].version, 2);
    }

    #[test]
    fn fiagro_conflicting_month_retains_previous_valid_report_with_an_explicit_warning() {
        let mut cached = fiagro_cache();
        let mut previous = cached.fiagro_months.remove("202609").unwrap();
        let stored = previous.reports.get_mut(AGRO_CNPJ).unwrap();
        stored.report.reference_date = "2026-08-01".into();
        stored.report.source_url = fiagro::url("202608");
        cached.fiagro_months.insert("202608".into(), previous);
        cached.fiagro_months.insert(
            "202609".into(),
            AnnualReports {
                fetched_at: stamp(at()),
                reports: BTreeMap::new(),
                rejected: BTreeMap::from([(AGRO_CNPJ.into(), "conflicting_revision".into())]),
            },
        );
        validate_cache(&cached).unwrap();
        let panel = build_panel(&cached, &request(Some(AGRO_CNPJ)), &[], false);
        assert_eq!(panel.items[0].status, "cached");
        assert_eq!(
            panel.items[0].report.as_ref().unwrap().reference_date,
            "2026-08-01"
        );
        assert!(panel.items[0]
            .error
            .as_deref()
            .unwrap()
            .contains("conflitantes"));
        for mutation in 0..3 {
            let mut tampered = cached.clone();
            let month = tampered.fiagro_months.get_mut("202609").unwrap();
            match mutation {
                0 => {
                    month
                        .rejected
                        .insert(AGRO_CNPJ.into(), "unknown_reason".into());
                }
                1 => {
                    month
                        .rejected
                        .insert("12345678900000".into(), "conflicting_revision".into());
                }
                _ => {
                    month
                        .reports
                        .insert(AGRO_CNPJ.into(), monthly().reports[AGRO_CNPJ].clone());
                }
            }
            assert!(validate_cache(&tampered).is_err());
        }
    }

    #[test]
    fn legacy_v2_cache_defaults_to_fii_without_fabricating_fiagro_support() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("cvm.json");
        let mut json = serde_json::to_value(cache()).unwrap();
        json.as_object_mut().unwrap().remove("fiagroMonths");
        json["register"]
            .as_object_mut()
            .unwrap()
            .remove("fiagroFunds");
        json["years"]["2026"]["reports"][CNPJ]["report"]
            .as_object_mut()
            .unwrap()
            .remove("fundType");
        let original = serde_json::to_vec(&json).unwrap();
        fs::write(&path, &original).unwrap();
        let service = CvmService::new(path.clone()).unwrap();
        let panel = service.read_cache(&request(Some(CNPJ))).unwrap();
        assert_eq!(panel.items[0].report.as_ref().unwrap().fund_type, "FII");
        assert_eq!(
            service.read_cache(&request(Some(AGRO_CNPJ))).unwrap().items[0].status,
            "unsupported"
        );
        assert_eq!(fs::read(path).unwrap(), original);
    }

    #[test]
    fn fiagro_cache_routing_ignores_old_fii_at_same_cnpj_and_preserves_failures() {
        let mut cached = fiagro_cache();
        let mut historical = cached.years[&2026].reports[CNPJ].clone();
        historical.report.cnpj = AGRO_CNPJ.into();
        historical.report.equity_per_share = Some("9999".into());
        cached
            .years
            .get_mut(&2026)
            .unwrap()
            .reports
            .insert(AGRO_CNPJ.into(), historical);
        validate_cache(&cached).unwrap();
        let panel = build_panel(&cached, &request(Some(AGRO_CNPJ)), &[], false);
        assert_eq!(panel.items[0].report.as_ref().unwrap().fund_type, "FIAGRO");
        assert_eq!(
            panel.items[0]
                .report
                .as_ref()
                .unwrap()
                .equity_per_share
                .as_deref(),
            Some("12.3456")
        );
        let panel = build_panel(
            &cached,
            &request(Some(AGRO_CNPJ)),
            &["HTTP 503".into()],
            true,
        );
        assert_eq!(panel.items[0].status, "cached");
        assert_eq!(
            panel.items[0].report.as_ref().unwrap().fetched_at,
            stamp(at())
        );
        cached.fiagro_months.clear();
        assert!(
            build_panel(&cached, &request(Some(AGRO_CNPJ)), &[], false).items[0]
                .report
                .is_none()
        );
    }

    #[test]
    fn fiagro_cache_validates_type_month_source_identity_and_roundtrips() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("cvm.json");
        write_cache(&path, &fiagro_cache()).unwrap();
        assert_eq!(
            CvmService::new(path.clone())
                .unwrap()
                .read_cache(&request(Some(AGRO_CNPJ)))
                .unwrap()
                .items[0]
                .report
                .as_ref()
                .unwrap()
                .fund_type,
            "FIAGRO"
        );
        for mutation in 0..6 {
            let mut cached = fiagro_cache();
            let report = &mut cached
                .fiagro_months
                .get_mut("202609")
                .unwrap()
                .reports
                .get_mut(AGRO_CNPJ)
                .unwrap()
                .report;
            match mutation {
                0 => report.fund_type = "FII".into(),
                1 => report.source_url = annual_url(2026),
                2 => report.source_url = fiagro::url("202608"),
                3 => report.cnpj = CNPJ.into(),
                4 => report.reference_date = "2026-08-01".into(),
                _ => report.net_assets = Some("NaN".into()),
            }
            assert!(validate_cache(&cached).is_err());
        }
        let mut cached = fiagro_cache();
        cached
            .years
            .get_mut(&2026)
            .unwrap()
            .reports
            .get_mut(CNPJ)
            .unwrap()
            .report
            .fund_type = "FIAGRO".into();
        assert!(validate_cache(&cached).is_err());
    }

    #[test]
    fn fiagro_lookback_is_bounded_and_crosses_years_without_future_months() {
        let months = fiagro::recent_months(at(), 99);
        assert_eq!(months.len(), 12);
        assert_eq!(months[0], "202609");
        assert_eq!(months[11], "202510");
        assert!(months
            .iter()
            .all(|month| fiagro::valid_url(&fiagro::url(month))));
        let january: DateTime<Utc> = "2026-01-15T12:00:00Z".parse().unwrap();
        assert_eq!(fiagro::recent_months(january, 2), vec!["202512", "202511"]);
    }

    #[test]
    fn cnpj_normalization_validates_check_digits_and_preserves_identifier() {
        assert_eq!(normalize_cnpj("00.332.266/0001-31").unwrap(), CNPJ);
        for invalid in [
            "00332266000132",
            "00000000000000",
            "0033226600013",
            "00.332.266/0001-31?",
        ] {
            assert!(normalize_cnpj(invalid).is_err());
        }
    }

    #[test]
    fn monetary_values_are_exact_and_absent_values_remain_absent() {
        assert_eq!(decimal("1234.56000", true).unwrap(), Some("1234.56".into()));
        assert_eq!(
            decimal("1.31856540084388E-08", true).unwrap(),
            Some("0.0000000131856540084388".into())
        );
        assert_eq!(decimal("-123.45", true).unwrap(), Some("-123.45".into()));
        assert_eq!(decimal("", true).unwrap(), None);
        for malformed in ["1.234,50", "NaN", "Infinity", "1 234", "1,2"] {
            assert!(decimal(malformed, true).is_err());
        }
        assert!(decimal("0.00000000000000000000000000001", true).is_err());
        assert!(decimal("1E-29", true).is_err());
        assert!(decimal("-2", false).is_err());
        assert!(integer("7.5").is_err());
        assert_eq!(integer("").unwrap(), None);
    }

    #[test]
    fn latest_competence_then_revision_joins_matching_version_only() {
        let result = parse_annual(&fixture(
            "00.332.266/0001-31;2026-08-01;7;2026-10-15;Agosto;100\n00.332.266/0001-31;2026-09-01;2;2026-10-12;Revisto;100\n00.332.266/0001-31;2026-09-01;1;2026-10-10;Original;100\n",
            "00.332.266/0001-31;2026-09-01;1;999;9;2;100\n00.332.266/0001-31;2026-09-01;2;1234.56;12.3456;7;100\n",
        ), 2026, at()).unwrap();
        let report = &result.reports[CNPJ];
        assert_eq!(report.version, 2);
        assert_eq!(report.report.fund_name, "Revisto");
        assert_eq!(report.report.reference_date, "2026-09-01");
        assert_eq!(report.report.net_assets.as_deref(), Some("1234.56"));
        assert_eq!(report.report.shareholders, Some(7));
    }

    #[test]
    fn missing_complement_does_not_mix_older_version_or_invent_zeroes() {
        let result = parse_annual(
            &fixture(
                "00.332.266/0001-31;2026-09-01;2;2026-10-12;Revisto;\n",
                "00.332.266/0001-31;2026-09-01;1;999;9;2;100\n",
            ),
            2026,
            at(),
        )
        .unwrap();
        let report = &result.reports[CNPJ].report;
        assert!(report.net_assets.is_none());
        assert!(report.issued_shares.is_none());
        assert!(report.shareholders.is_none());
    }

    #[test]
    fn empty_report_name_uses_only_official_register_with_exact_same_cnpj() {
        let mut result = parse_annual(
            &fixture(
                "00.332.266/0001-31;2026-09-01;1;2026-10-10;;100\n",
                "00.332.266/0001-31;2026-09-01;1;1234;12.34;7;100\n",
            ),
            2026,
            at(),
        )
        .unwrap();
        resolve_names(&mut result, cache().register.as_ref().unwrap());
        assert_eq!(result.reports[CNPJ].report.fund_name, "Fundo Oficial");
        resolve_names(
            &mut result,
            &Register {
                fetched_at: stamp(at()),
                funds: BTreeMap::new(),
                fiagro_funds: BTreeMap::new(),
                other_funds: BTreeMap::new(),
            },
        );
        assert!(result.reports.is_empty());
    }

    #[test]
    fn future_wrong_year_or_non_monthly_dates_are_rejected() {
        for reference in ["2026-13-01", "2026-09-30", "2025-09-01", "2026-11-01"] {
            let row = format!("00.332.266/0001-31;{reference};1;2026-10-10;Fundo;100\n");
            assert!(parse_annual(&fixture(&row, ""), 2026, at()).is_err());
        }
    }

    #[test]
    fn unknown_cnpj_never_gets_another_funds_report() {
        let mut cached = cache();
        cached.register.as_mut().unwrap().other_funds.clear();
        let panel = build_panel(&cached, &request(Some("26.502.794/0001-85")), &[], false);
        assert_eq!(panel.items[0].status, "not_found");
        assert!(panel.items[0].report.is_none());
        let panel = build_panel(&cache(), &request(None), &[], false);
        assert_eq!(panel.items[0].status, "mapping_required");
        let panel = build_panel(&cache(), &request(Some("123")), &[], false);
        assert_eq!(panel.items[0].status, "invalid_mapping");
    }

    #[test]
    fn failure_explicitly_marks_validated_cached_report_with_its_original_date() {
        let panel = build_panel(&cache(), &request(Some(CNPJ)), &["HTTP 503".into()], true);
        assert_eq!(panel.items[0].status, "cached");
        assert_eq!(
            panel.items[0].report.as_ref().unwrap().fetched_at,
            stamp(at())
        );
        assert!(panel.items[0].error.is_some());
        assert_eq!(panel.errors, vec!["HTTP 503"]);
    }

    #[test]
    fn register_does_not_accept_fiagro_or_a_name_match_as_fii() {
        let bytes = archive(&[
            ("registro_fundo.csv", "CNPJ_Fundo;Tipo_Fundo;Denominacao_Social;Situacao;Data_Inicio_Situacao\n00332266000131;FII;Fundo Oficial;Em Funcionamento Normal;2025-01-01\n26502794000185;FII;Nome Imobiliario;Cancelado;2025-09-30\n26502794000185;FIAGRO;Nome Imobiliario;Em Funcionamento Normal;2025-09-30\n"),
            ("registro_classe.csv", "CNPJ_Classe;Tipo_Classe;Denominacao_Social;Situacao;Data_Inicio_Situacao\n26502794000185;Classes de Cotas de Fundos FIAGRO;Nome Imobiliario;Em Funcionamento Normal;2025-09-30\n"),
        ]);
        let register = parse_register(&bytes, at()).unwrap();
        assert_eq!(register.funds.len(), 1);
        assert!(register.funds.contains_key(CNPJ));
        assert_eq!(register.fiagro_funds["26502794000185"], "Nome Imobiliario");
        let mut cached = cache();
        cached.register = Some(register);
        let panel = build_panel(&cached, &request(Some("26502794000185")), &[], false);
        assert_eq!(panel.items[0].status, "not_found");
        assert!(panel.items[0].report.is_none());
    }

    #[test]
    fn zip_csv_schema_and_duplicate_entries_are_bounded() {
        assert!(parse_annual(b"not a zip", 2026, at()).is_err());
        assert!(parse_annual(
            &archive(&[("../inf_mensal_fii_geral_2026.csv", "x")]),
            2026,
            at()
        )
        .is_err());
        assert!(Table::read(b"CNPJ;CNPJ\na;b\n", &["CNPJ"]).is_err());
        assert!(Table::read(b"Outra\na\n", &["CNPJ"]).is_err());
        assert_eq!(decode_csv(b"S\xe3o Paulo"), "São Paulo");
    }

    #[test]
    fn cache_rejects_tampered_identity_source_and_value_and_roundtrips() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("cvm.json");
        let valid = cache();
        write_cache(&path, &valid).unwrap();
        assert!(read_cache_file(&path).unwrap().is_some());
        for mutation in 0..3 {
            let mut broken = cache();
            let report = &mut broken
                .years
                .get_mut(&2026)
                .unwrap()
                .reports
                .get_mut(CNPJ)
                .unwrap()
                .report;
            match mutation {
                0 => report.cnpj = "26502794000185".into(),
                1 => report.source_url = "https://example.com/poison".into(),
                _ => report.net_assets = Some("NaN".into()),
            }
            assert!(validate_cache(&broken).is_err());
        }
    }

    #[tokio::test]
    async fn unmapped_requests_are_local_and_corrupt_cache_is_preserved() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("cvm.json");
        fs::write(&path, "broken").unwrap();
        let service = CvmService::new(path.clone()).unwrap();
        assert!(service.read_cache(&request(None)).is_err());
        let panel = service.refresh(&request(None)).await.unwrap();
        assert_eq!(panel.items[0].status, "mapping_required");
        assert_eq!(fs::read_to_string(path).unwrap(), "broken");
    }

    #[test]
    fn read_cache_does_not_create_data_directory_or_lock() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("absent").join("cvm.json");
        let service = CvmService::new(path.clone()).unwrap();
        let panel = service.read_cache(&request(None)).unwrap();
        assert_eq!(panel.items[0].status, "mapping_required");
        assert!(!path.parent().unwrap().exists());
    }

    #[tokio::test]
    async fn instances_share_path_gate_and_recheck_fresh_archive_without_http() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("cvm.json");
        let mut cached = cache();
        let now = stamp(Utc::now());
        cached.register.as_mut().unwrap().fetched_at = now.clone();
        for annual in cached.years.values_mut() {
            annual.fetched_at = now.clone();
            for stored in annual.reports.values_mut() {
                stored.report.fetched_at = now.clone();
            }
        }
        write_cache(&path, &cached).unwrap();
        let first = CvmService::new(path.clone()).unwrap();
        let second = CvmService::new(path.clone()).unwrap();
        assert!(Arc::ptr_eq(&first.refresh_lock, &second.refresh_lock));
        let funds = request(Some(CNPJ));
        let (one, two) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(first.refresh(&funds), second.refresh(&funds))
        })
        .await
        .expect("fresh cache must not wait for CVM HTTP");
        assert_eq!(one.unwrap(), two.unwrap());
        assert_eq!(
            read_cache_file(&path).unwrap().unwrap().years[&2026].fetched_at,
            now
        );
    }

    #[tokio::test]
    async fn automatic_fiagro_with_current_mapping_reuses_daily_cache_without_http() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cvm.json");
        let mut cached = fiagro_cache();
        let now = stamp(Utc::now() - chrono::Duration::hours(12));
        let register = cached.register.as_mut().unwrap();
        register.fetched_at = now.clone();
        for annual in cached
            .years
            .values_mut()
            .chain(cached.fiagro_months.values_mut())
        {
            annual.fetched_at = now.clone();
            for stored in annual.reports.values_mut() {
                stored.report.fetched_at = now.clone();
            }
        }
        write_cache(&path, &cached).unwrap();
        let bytes = fs::read(&path).unwrap();
        let mut service = CvmService::new(path.clone()).unwrap();
        service.client = Client::builder()
            .proxy(reqwest::Proxy::all("http://127.0.0.1:9").unwrap())
            .timeout(Duration::from_millis(100))
            .build()
            .unwrap();
        for _ in 0..2 {
            let outcome = service.refresh_if_due(&request(Some(AGRO_CNPJ))).await;
            assert_eq!(outcome.status, "cached");
            assert!(outcome.message.is_none());
            assert_eq!(
                outcome.data.unwrap().items[0]
                    .report
                    .as_ref()
                    .unwrap()
                    .fund_type,
                "FIAGRO"
            );
        }
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(!path.with_extension("refresh-policy.json").exists());
    }

    #[tokio::test]
    async fn cross_process_network_lock_is_exclusive_and_released_on_drop() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("cvm.json");
        let first = network_lock(&path).await.unwrap();
        let second = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.with_extension("refresh.lock"))
            .unwrap();
        assert!(matches!(
            second.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        drop(first);
        assert!(second.try_lock().is_ok());
    }

    #[test]
    fn endpoint_and_ttl_cannot_be_overridden_by_input() {
        assert!(valid_annual_url(&annual_url(2026)));
        assert!(!valid_annual_url(
            "http://dados.cvm.gov.br/dados/FII/DOC/INF_MENSAL/DADOS/inf_mensal_fii_2026.zip"
        ));
        assert!(!valid_annual_url(
            "https://example.com/inf_mensal_fii_2026.zip"
        ));
        assert!(fresh("2026-10-19T10:00:00Z", at()));
        assert!(!fresh("2026-10-18T10:00:00Z", at()));
        assert!(!fresh("2026-10-20T10:00:00Z", at()));
    }
}
