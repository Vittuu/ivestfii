//! Daily fund/class NAVs published in CVM's open Informe Diário archives.
//!
//! A NAV is a public fund price, never a broker balance or a personal tax event.
//! Fund names, CNPJs and source dates are matched locally against the official
//! register. Requests contain only the public archive month, never cash flows.

use crate::refresh_policy::{fresh, RefreshGate, RefreshOutcome};
use chrono::{DateTime, Datelike, FixedOffset, Months, NaiveDate, SecondsFormat, Utc};
use reqwest::{redirect::Policy, Client};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};
use uuid::Uuid;

pub const FUND_NAV_SOURCE: &str = "CVM · Informe Diário de Fundos";
const REGISTER_URL: &str = "https://dados.cvm.gov.br/dados/FI/CAD/DADOS/registro_fundo_classe.zip";
const CACHE_VERSION: u32 = 1;
const MAX_DOWNLOAD: usize = 32 * 1024 * 1024;
const MAX_EXPANDED: u64 = 256 * 1024 * 1024;
const MAX_CACHE: u64 = 32 * 1024 * 1024;
const MAX_ROWS: usize = 2_000_000;
const MAX_REGISTER_ROWS: usize = 150_000;
const MAX_MONTHS: usize = 120;
const MAX_BATCH: usize = 16;
const DAILY_ARCHIVE_LIMIT: u32 = 48;
const CLOSED_TTL: i64 = 7 * 24 * 60 * 60;
const CURRENT_TTL: i64 = 6 * 60 * 60;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FundNavRequest {
    pub fund_name: String,
    pub cnpj: Option<String>,
    pub start_date: String,
    pub end_date: String,
    /// Cotization dates needed before a position can be reconstructed. Amounts
    /// and personal identifiers are deliberately absent from this request.
    #[serde(default)]
    pub anchor_dates: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FundNavObservation {
    pub cnpj: String,
    pub date: String,
    pub value: String,
    pub source: String,
    pub source_url: String,
    pub fetched_at: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FundNavPanel {
    pub cnpj: Option<String>,
    pub fund_name: Option<String>,
    pub observations: Vec<FundNavObservation>,
    pub issues: Vec<String>,
    /// Months still awaiting their first successful download. A successfully
    /// checked archive with no matching NAV is explained separately, never zero.
    pub pending_months: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Register {
    fetched_at: String,
    /// Active FI/FIF identities only; multiple legal names for one CNPJ are
    /// retained across the fund-to-class transition, not chosen fuzzily.
    funds: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Month {
    fetched_at: String,
    observations: Vec<FundNavObservation>,
    issues: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cache {
    version: u32,
    register: Option<Register>,
    /// Stores NAVs only for explicitly requested CNPJs, never whole-market data.
    funds: BTreeMap<String, BTreeMap<String, Month>>,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            version: CACHE_VERSION,
            register: None,
            funds: BTreeMap::new(),
        }
    }
}

#[derive(Clone)]
pub struct CvmNavService {
    client: Client,
    cache_path: PathBuf,
}

impl CvmNavService {
    pub fn new(cache_path: PathBuf) -> Result<Self, String> {
        if cache_path.file_name().is_none() {
            return Err("Informe um arquivo para o histórico de cotas da CVM.".into());
        }
        let cache_path = std::path::absolute(cache_path)
            .map_err(|_| "Caminho do histórico de cotas CVM inválido.".to_string())?;
        let client = Client::builder()
            .https_only(true)
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(7))
            .timeout(Duration::from_secs(35))
            .user_agent("IvestFII/0.1 (CVM public daily fund NAV history)")
            .build()
            .map_err(|_| "Não foi possível preparar a consulta de cotas à CVM.".to_string())?;
        Ok(Self { client, cache_path })
    }

    /// Pure local read: does not create folders, locks, or network requests.
    pub fn read_cache(&self, request: &FundNavRequest) -> Result<FundNavPanel, String> {
        let range = Range::new(request, Utc::now())?;
        let cache = read_file(&self.cache_path)?.unwrap_or_default();
        Ok(panel(&cache, request, &range, Vec::new()))
    }

    /// Every button/background/MCP process shares persistent per-archive request
    /// reservations. Successful batches may continue immediately into missing
    /// months; failures retry after five minutes. No brapi quota is consumed.
    pub async fn refresh(&self, request: &FundNavRequest) -> Result<FundNavPanel, String> {
        let range = Range::new(request, Utc::now())?;
        match tokio::time::timeout(Duration::from_secs(90), self.refresh_inner(request, &range))
            .await
        {
            Ok(value) => value,
            Err(_) => {
                let mut cached = self.read_cache(request)?;
                cached.issues.push("A consulta de cotas atingiu 90 segundos. O progresso salvo foi preservado; os meses restantes podem continuar na próxima consulta.".into());
                Ok(cached)
            }
        }
    }

    pub async fn refresh_if_due(&self, request: &FundNavRequest) -> RefreshOutcome<FundNavPanel> {
        let before = self.read_cache(request).ok();
        match self.refresh(request).await {
            Ok(data) => {
                let changed = before.as_ref().is_none_or(|previous| {
                    previous.cnpj != data.cnpj
                        || previous.observations != data.observations
                        || previous.pending_months != data.pending_months
                });
                let message = (!data.issues.is_empty()).then(|| data.issues.join(" "));
                let mut outcome = if changed {
                    RefreshOutcome::updated(data)
                } else {
                    RefreshOutcome::cached(data)
                };
                outcome.message = message;
                outcome
            }
            Err(error) => RefreshOutcome::failed(self.read_cache(request).ok(), error),
        }
    }

    async fn refresh_inner(
        &self,
        request: &FundNavRequest,
        range: &Range,
    ) -> Result<FundNavPanel, String> {
        // This cross-process lock is held over HTTP and incremental atomic
        // writes. Every waiter re-reads both the dated cache and reservations.
        let mut gate = RefreshGate::acquire(&self.cache_path).await?;
        let mut cache = read_file(&self.cache_path)?.unwrap_or_default();
        let mut issues = Vec::new();
        let now = Utc::now();
        let register_fresh = cache
            .register
            .as_ref()
            .is_some_and(|value| fresh(Some(&value.fetched_at), CLOSED_TTL, now));
        if !register_fresh {
            let cached_at = cache
                .register
                .as_ref()
                .map(|value| value.fetched_at.as_str());
            if let Some(reason) = gate.reserve(
                "cvm-nav-register",
                cached_at,
                CLOSED_TTL,
                now,
                Some(DAILY_ARCHIVE_LIMIT),
            )? {
                issues.push(reason);
            } else {
                match self
                    .download(REGISTER_URL)
                    .await
                    .and_then(|bytes| parse_register(&bytes, Utc::now()))
                {
                    Ok(register) => {
                        cache.register = Some(register);
                        write_file(&self.cache_path, &cache)?;
                    }
                    Err(error) => issues.push(error),
                }
            }
        }
        let Some((cnpj, _)) = resolve(cache.register.as_ref(), request)? else {
            return Ok(panel(&cache, request, range, issues));
        };
        let all_months = range.months();
        let current_month = month_key(brasilia_date(Utc::now()));
        let mut work = prioritise(&all_months, &range.anchors);
        // Missing chunks always precede old chunks whose weekly TTL elapsed.
        // A large backfill therefore advances instead of rechecking old data.
        work.sort_by_key(|month| {
            cache
                .funds
                .get(&cnpj)
                .is_some_and(|f| f.contains_key(month))
        });
        let mut attempted = 0;
        for month in work {
            let cached_at = cache
                .funds
                .get(&cnpj)
                .and_then(|f| f.get(&month))
                .map(|value| value.fetched_at.as_str());
            let ttl = if month == current_month {
                CURRENT_TTL
            } else {
                CLOSED_TTL
            };
            if fresh(cached_at, ttl, Utc::now()) {
                continue;
            }
            if attempted >= MAX_BATCH {
                issues.push(format!("Este lote consultou até {MAX_BATCH} arquivos mensais da CVM. O histórico salvo será reutilizado para continuar os meses restantes."));
                break;
            }
            let key = format!("cvm-nav:{cnpj}:{}", month.replace('-', ""));
            if let Some(reason) =
                gate.reserve(&key, cached_at, ttl, Utc::now(), Some(DAILY_ARCHIVE_LIMIT))?
            {
                issues.push(reason);
                continue;
            }
            attempted += 1;
            let url = monthly_url(&month);
            match self
                .download(&url)
                .await
                .and_then(|bytes| parse_month(&bytes, &cnpj, &month, Utc::now()))
            {
                Ok(value) => {
                    cache
                        .funds
                        .entry(cnpj.clone())
                        .or_default()
                        .insert(month, value);
                    // Save after every successful month, including before a
                    // caller timeout/cancellation or failure in a later archive.
                    write_file(&self.cache_path, &cache)?;
                }
                Err(error) => issues.push(format!("Cotas {month}: {error}")),
            }
        }
        Ok(panel(&cache, request, range, issues))
    }

    async fn download(&self, url: &str) -> Result<Vec<u8>, String> {
        if url != REGISTER_URL && !valid_monthly_url(url) {
            return Err("Origem do histórico CVM não permitida.".into());
        }
        let mut response = self.client.get(url).send().await.map_err(|_| {
            "Não foi possível consultar as cotas públicas da CVM. Os dados salvos foram mantidos.".to_string()
        })?;
        if !response.status().is_success() {
            return Err(format!(
                "Arquivo público CVM indisponível (HTTP {}).",
                response.status().as_u16()
            ));
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_DOWNLOAD as u64)
        {
            return Err("Arquivo de cotas CVM excede o limite de 32 MiB.".into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "Download de cotas CVM interrompido.".to_string())?
        {
            if chunk.len() > MAX_DOWNLOAD.saturating_sub(bytes.len()) {
                return Err("Arquivo de cotas CVM excede o limite de 32 MiB.".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

struct Range {
    start: NaiveDate,
    end: NaiveDate,
    anchors: BTreeSet<String>,
}

impl Range {
    fn new(request: &FundNavRequest, now: DateTime<Utc>) -> Result<Self, String> {
        if request.fund_name.trim().len() < 3
            || request.fund_name.len() > 512
            || request.fund_name.chars().any(char::is_control)
            || request.anchor_dates.len() > 5000
        {
            return Err("Nome ou datas do fundo inválidos para consultar cotas CVM.".into());
        }
        if let Some(cnpj) = &request.cnpj {
            normalize_cnpj(cnpj)?;
        }
        let start = parse_date(&request.start_date)?;
        let end = parse_date(&request.end_date)?.min(brasilia_date(now));
        if start > end || start.year() < 2021 {
            return Err("Consulte cotas desde 2021, em um período válido de até 120 meses.".into());
        }
        let mut anchors = BTreeSet::new();
        for day in &request.anchor_dates {
            let date = parse_date(day)?;
            if date < start || date > end {
                return Err("Data de cotização fora do período solicitado à CVM.".into());
            }
            anchors.insert(month_key(date));
        }
        anchors.insert(month_key(start));
        let output = Self {
            start,
            end,
            anchors,
        };
        if output.months().len() > MAX_MONTHS {
            return Err("O histórico de cotas está limitado a 120 meses por consulta.".into());
        }
        Ok(output)
    }

    fn months(&self) -> Vec<String> {
        let mut day = self.start.with_day(1).expect("valid month");
        let end = self.end.with_day(1).expect("valid month");
        let mut output = Vec::new();
        while day <= end && output.len() <= MAX_MONTHS {
            output.push(month_key(day));
            day = day
                .checked_add_months(Months::new(1))
                .expect("bounded modern month");
        }
        output
    }
}

fn prioritise(months: &[String], anchors: &BTreeSet<String>) -> Vec<String> {
    anchors
        .iter()
        .filter(|m| months.contains(m))
        .cloned()
        .chain(
            months
                .iter()
                .rev()
                .filter(|m| !anchors.contains(*m))
                .cloned(),
        )
        .collect()
}

fn brasilia_date(at: DateTime<Utc>) -> NaiveDate {
    at.with_timezone(&FixedOffset::west_opt(3 * 3600).expect("fixed BRT offset"))
        .date_naive()
}

fn parse_date(value: &str) -> Result<NaiveDate, String> {
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| "Data inválida no histórico de cotas CVM.".to_string())?;
    if value != date.to_string() {
        return Err("Use datas no formato AAAA-MM-DD.".into());
    }
    Ok(date)
}

fn month_key(day: NaiveDate) -> String {
    day.format("%Y-%m").to_string()
}

fn monthly_url(month: &str) -> String {
    format!(
        "https://dados.cvm.gov.br/dados/FI/DOC/INF_DIARIO/DADOS/inf_diario_fi_{}.zip",
        month.replace('-', "")
    )
}

fn valid_monthly_url(url: &str) -> bool {
    let prefix = "https://dados.cvm.gov.br/dados/FI/DOC/INF_DIARIO/DADOS/inf_diario_fi_";
    url.strip_prefix(prefix)
        .and_then(|v| v.strip_suffix(".zip"))
        .is_some_and(|v| {
            v.len() == 6
                && v.bytes().all(|v| v.is_ascii_digit())
                && NaiveDate::parse_from_str(&format!("{v}01"), "%Y%m%d").is_ok()
        })
}

fn normalize_cnpj(value: &str) -> Result<String, String> {
    if value.is_empty()
        || value.len() > 18
        || value
            .bytes()
            .any(|b| !b.is_ascii_digit() && !b"./-".contains(&b))
    {
        return Err("CNPJ inválido para identificar o fundo na CVM.".into());
    }
    let digits: Vec<u8> = value
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    if digits.len() != 14 || digits.iter().all(|v| *v == digits[0]) {
        return Err("CNPJ inválido para identificar o fundo na CVM.".into());
    }
    for (size, weights) in [
        (12, &[5, 4, 3, 2, 9, 8, 7, 6, 5, 4, 3, 2][..]),
        (13, &[6, 5, 4, 3, 2, 9, 8, 7, 6, 5, 4, 3, 2][..]),
    ] {
        let remainder: u32 = digits[..size]
            .iter()
            .zip(weights)
            .map(|(a, b)| u32::from(*a) * b)
            .sum::<u32>()
            % 11;
        let check = if remainder < 2 { 0 } else { 11 - remainder };
        if u32::from(digits[size]) != check {
            return Err("CNPJ inválido para identificar o fundo na CVM.".into());
        }
    }
    Ok(digits.into_iter().map(|d| char::from(d + b'0')).collect())
}

fn latin1(bytes: &[u8]) -> String {
    std::str::from_utf8(bytes)
        .map(str::to_string)
        .unwrap_or_else(|_| bytes.iter().map(|b| char::from(*b)).collect())
}

/// Expand only legal vehicle abbreviations, preserving every identifying token.
/// This is exact equality after normalization, not substring/fuzzy matching.
fn canonical_name(value: &str) -> String {
    let folded: String = value
        .to_uppercase()
        .chars()
        .map(|c| match c {
            'Á' | 'À' | 'Â' | 'Ã' | 'Ä' => 'A',
            'É' | 'È' | 'Ê' | 'Ë' => 'E',
            'Í' | 'Ì' | 'Î' | 'Ï' => 'I',
            'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' => 'O',
            'Ú' | 'Ù' | 'Û' | 'Ü' => 'U',
            'Ç' => 'C',
            c if c.is_ascii_alphanumeric() => c,
            _ => ' ',
        })
        .collect();
    let mut output = format!(
        " {} ",
        folded.split_whitespace().collect::<Vec<_>>().join(" ")
    );
    for (long, short) in [
        (
            "FUNDO DE INVESTIMENTO EM COTAS DE FUNDOS DE INVESTIMENTO",
            "FIC",
        ),
        ("FUNDO DE INVESTIMENTO EM COTAS", "FIC"),
        ("FUNDO DE INVESTIMENTO FINANCEIRO", "FIF"),
        ("FUNDO DE INVESTIMENTO EM RENDA FIXA", "FIRF"),
        ("FUNDO DE INVESTIMENTO RENDA FIXA", "FIRF"),
        ("FUNDO DE INVESTIMENTO", "FI"),
        ("RESPONSABILIDADE LIMITADA", "RL"),
        ("RENDA FIXA", "FIRF"),
    ] {
        output = output.replace(&format!(" {long} "), &format!(" {short} "));
    }
    output.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn resolve(
    register: Option<&Register>,
    request: &FundNavRequest,
) -> Result<Option<(String, String)>, String> {
    let Some(register) = register else {
        return Ok(None);
    };
    if let Some(cnpj) = &request.cnpj {
        let cnpj = normalize_cnpj(cnpj)?;
        return match register.funds.get(&cnpj) {
            Some(names) => Ok(names.iter().next().map(|name| (cnpj, name.clone()))),
            None => Err("O CNPJ informado não consta como fundo/classe FI/FIF ativo no cadastro CVM. Confira a identificação.".into()),
        };
    }
    let name = canonical_name(&request.fund_name);
    let matches: Vec<_> = register
        .funds
        .iter()
        .filter_map(|(cnpj, names)| {
            names
                .iter()
                .find(|official| canonical_name(official) == name)
                .map(|official| (cnpj.clone(), official.clone()))
        })
        .collect();
    match matches.as_slice() {
        [one] => Ok(Some(one.clone())),
        [] => Ok(None),
        _ => Err("Mais de um CNPJ corresponde ao nome do fundo. Informe o CNPJ da aplicação para consultar as cotas com segurança.".into()),
    }
}

fn archive(bytes: &[u8]) -> Result<zip::ZipArchive<Cursor<&[u8]>>, String> {
    if bytes.len() > MAX_DOWNLOAD {
        return Err("Arquivo CVM excede o limite de download.".into());
    }
    let zip = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| "Arquivo CVM não é um ZIP válido.".to_string())?;
    if zip.len() == 0 || zip.len() > 16 {
        return Err("ZIP CVM contém entradas em excesso.".into());
    }
    let mut total = 0u64;
    for entry in zip.clone().file_names() {
        if entry.len() > 256 {
            return Err("Nome de tabela CVM inválido.".into());
        }
    }
    // Check declared sizes before decompression; each CSV is also read through
    // a byte bound so corrupt headers cannot bypass the expanded-size limit.
    let mut inspect = zip.clone();
    for i in 0..inspect.len() {
        let member = inspect
            .by_index(i)
            .map_err(|_| "Entrada ZIP CVM inválida.".to_string())?;
        total = total
            .checked_add(member.size())
            .ok_or("Tamanho ZIP CVM inválido.")?;
        if total > MAX_EXPANDED || member.size() > MAX_EXPANDED {
            return Err("ZIP CVM excede o limite de 256 MiB descompactados.".into());
        }
    }
    Ok(zip)
}

fn header_index(headers: &csv::ByteRecord, name: &str) -> Result<usize, String> {
    let positions: Vec<_> = headers
        .iter()
        .enumerate()
        .filter(|(_, h)| *h == name.as_bytes())
        .map(|(i, _)| i)
        .collect();
    if positions.len() != 1 {
        return Err(format!(
            "Coluna CVM obrigatória ausente ou duplicada: {name}."
        ));
    }
    Ok(positions[0])
}

fn cell<'a>(row: &'a csv::ByteRecord, index: usize) -> Result<&'a str, String> {
    std::str::from_utf8(row.get(index).ok_or("Linha CSV CVM incompleta.")?)
        .map(str::trim)
        .map_err(|_| "Valor CVM inválido.".into())
}

fn parse_register(bytes: &[u8], now: DateTime<Utc>) -> Result<Register, String> {
    let mut zip = archive(bytes)?;
    let mut funds: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for index in 0..zip.len() {
        let file = zip
            .by_index(index)
            .map_err(|_| "Tabela do cadastro CVM inválida.".to_string())?;
        let name = file.name().to_string();
        let class = name == "registro_classe.csv";
        if !class && name != "registro_fundo.csv" {
            continue;
        }
        if !seen.insert(name) {
            return Err("ZIP CVM repete uma tabela do cadastro.".into());
        }
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(b';')
            .from_reader(file.take(MAX_EXPANDED + 1));
        let headers = reader
            .byte_headers()
            .map_err(|_| "Cabeçalho do cadastro CVM inválido.".to_string())?
            .clone();
        let id = header_index(&headers, if class { "CNPJ_Classe" } else { "CNPJ_Fundo" })?;
        let title = header_index(&headers, "Denominacao_Social")?;
        let kind = header_index(&headers, if class { "Tipo_Classe" } else { "Tipo_Fundo" })?;
        let situation = header_index(&headers, "Situacao")?;
        for (n, row) in reader.byte_records().enumerate() {
            if n >= MAX_REGISTER_ROWS {
                return Err("Cadastro CVM excede o limite de linhas.".into());
            }
            let row = row.map_err(|_| "Linha do cadastro CVM inválida.".to_string())?;
            let active = canonical_name(&latin1(row.get(situation).unwrap_or_default()))
                == "EM FUNCIONAMENTO NORMAL";
            let kind = canonical_name(&latin1(row.get(kind).unwrap_or_default()));
            if !active
                || (!class && kind != "FI")
                || (class && kind != "CLASSES DE COTAS DE FUNDOS FIF")
            {
                continue;
            }
            let cnpj = normalize_cnpj(cell(&row, id)?)?;
            let name = latin1(row.get(title).unwrap_or_default())
                .trim()
                .to_string();
            if name.is_empty() || name.len() > 1024 || name.chars().any(char::is_control) {
                return Err("Nome inválido no cadastro de fundos CVM.".into());
            }
            funds.entry(cnpj).or_default().insert(name);
        }
    }
    if seen.len() != 2 || funds.is_empty() {
        return Err("ZIP CVM não contém o cadastro completo de fundos/classes FI/FIF.".into());
    }
    Ok(Register {
        fetched_at: stamp(now),
        funds,
    })
}

fn parse_month(bytes: &[u8], cnpj: &str, month: &str, now: DateTime<Utc>) -> Result<Month, String> {
    let mut zip = archive(bytes)?;
    let expected = format!("inf_diario_fi_{}.csv", month.replace('-', ""));
    let mut found = false;
    let mut quotes: BTreeMap<String, String> = BTreeMap::new();
    let mut subclasses = BTreeSet::new();
    // A malformed/zero NAV invalidates that public date, not the valid prices
    // on every other day in the archive. A later duplicate may not silently
    // repair the invalid date: the source must be corrected and fetched again.
    let mut unusable_dates = BTreeSet::new();
    for index in 0..zip.len() {
        let file = zip
            .by_index(index)
            .map_err(|_| "Tabela de cotas CVM inválida.".to_string())?;
        if file.name() != expected {
            continue;
        }
        if found {
            return Err("ZIP CVM repete uma tabela de cotas.".into());
        }
        found = true;
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(b';')
            .from_reader(file.take(MAX_EXPANDED + 1));
        let headers = reader
            .byte_headers()
            .map_err(|_| "Cabeçalho das cotas CVM inválido.".to_string())?
            .clone();
        let modern = headers.iter().any(|v| v == b"CNPJ_FUNDO_CLASSE");
        let id = header_index(
            &headers,
            if modern {
                "CNPJ_FUNDO_CLASSE"
            } else {
                "CNPJ_FUNDO"
            },
        )?;
        let kind = header_index(
            &headers,
            if modern {
                "TP_FUNDO_CLASSE"
            } else {
                "TP_FUNDO"
            },
        )?;
        let date = header_index(&headers, "DT_COMPTC")?;
        let value = header_index(&headers, "VL_QUOTA")?;
        let subclass = if modern {
            Some(header_index(&headers, "ID_SUBCLASSE")?)
        } else {
            None
        };
        for (n, row) in reader.byte_records().enumerate() {
            if n >= MAX_ROWS {
                return Err("Tabela de cotas CVM excede o limite de linhas.".into());
            }
            let row = row.map_err(|_| "Linha de cotas CVM inválida.".to_string())?;
            let raw_id = cell(&row, id)?;
            // Match cheaply before validating other funds' unused fields.
            let compact: String = raw_id.chars().filter(char::is_ascii_digit).collect();
            if compact != cnpj {
                continue;
            }
            if normalize_cnpj(raw_id)? != cnpj {
                return Err("CNPJ inconsistente no informe CVM.".into());
            }
            let row_kind = cell(&row, kind)?;
            if row_kind != "FI" && row_kind != "CLASSES - FIF" {
                return Err("Tipo do fundo/classe incompatível com cotas FI/FIF.".into());
            }
            let day = parse_date(cell(&row, date)?)?;
            if month_key(day) != month || day > brasilia_date(now) {
                return Err("Data da cota não corresponde ao arquivo mensal CVM.".into());
            }
            let subclass = subclass
                .map(|i| cell(&row, i))
                .transpose()?
                .unwrap_or("")
                .to_string();
            subclasses.insert(subclass);
            let raw = cell(&row, value)?;
            let nav = Decimal::from_str(raw)
                .ok()
                .filter(|nav| *nav > Decimal::ZERO && raw.len() <= 64);
            let Some(nav) = nav else {
                unusable_dates.insert(day.to_string());
                quotes.remove(&day.to_string());
                continue;
            };
            if unusable_dates.contains(&day.to_string()) {
                continue;
            }
            let normalized = nav.normalize().to_string();
            if quotes
                .get(&day.to_string())
                .is_some_and(|old| old != &normalized)
            {
                return Err("A CVM publicou cotas conflitantes para o mesmo CNPJ e data; confira a subclasse.".into());
            }
            quotes.insert(day.to_string(), normalized);
        }
    }
    if !found {
        return Err("ZIP CVM não contém a tabela mensal esperada.".into());
    }
    let fetched_at = stamp(now);
    let mut issues: Vec<String> = unusable_dates.into_iter().map(|day| {
        format!("A CVM informou uma cota inválida ou não positiva em {day}. Essa data ficou sem valor; as demais cotas válidas do mês foram preservadas, sem interpolação.")
    }).collect();
    if subclasses.iter().any(|value| !value.is_empty()) {
        quotes.clear();
        issues.push("O informe identifica subclasse de cotas. Falta o identificador da subclasse da aplicação; nenhuma cota foi escolhida automaticamente.".into());
    }
    if quotes.is_empty() && issues.is_empty() {
        issues.push(format!(
            "A CVM não publicou cotas deste fundo no arquivo de {month}."
        ));
    }
    Ok(Month {
        fetched_at: fetched_at.clone(),
        observations: quotes
            .into_iter()
            .map(|(date, value)| FundNavObservation {
                cnpj: cnpj.into(),
                date,
                value,
                source: FUND_NAV_SOURCE.into(),
                source_url: monthly_url(month),
                fetched_at: fetched_at.clone(),
            })
            .collect(),
        issues,
    })
}

fn panel(
    cache: &Cache,
    request: &FundNavRequest,
    range: &Range,
    mut issues: Vec<String>,
) -> FundNavPanel {
    let identity = match resolve(cache.register.as_ref(), request) {
        Ok(value) => value,
        Err(error) => {
            issues.push(error);
            None
        }
    };
    let Some((cnpj, name)) = identity else {
        if cache.register.is_none() {
            issues.push(
                "O cadastro oficial CVM ainda não foi consultado para identificar o fundo.".into(),
            );
        } else if issues.is_empty() {
            issues.push("O nome não corresponde de forma única ao cadastro oficial CVM. Informe o CNPJ da aplicação.".into());
        }
        return FundNavPanel {
            pending_months: range.months().len(),
            issues,
            ..FundNavPanel::default()
        };
    };
    let months = cache.funds.get(&cnpj);
    let mut observations = Vec::new();
    let mut pending_months = 0;
    for month in range.months() {
        match months.and_then(|rows| rows.get(&month)) {
            None => pending_months += 1,
            Some(value) => {
                observations.extend(
                    value
                        .observations
                        .iter()
                        .filter(|item| {
                            item.date >= range.start.to_string()
                                && item.date <= range.end.to_string()
                        })
                        .cloned(),
                );
                issues.extend(value.issues.iter().cloned());
            }
        }
    }
    if pending_months > 0 {
        issues.push(format!(
            "{pending_months} meses do histórico de cotas ainda aguardam consulta à CVM."
        ));
    }
    observations.sort_by(|a, b| a.date.cmp(&b.date));
    issues = issues
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    FundNavPanel {
        cnpj: Some(cnpj),
        fund_name: Some(name),
        observations,
        issues,
        pending_months,
    }
}

fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn read_file(path: &Path) -> Result<Option<Cache>, String> {
    let file = match File::open(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err("Não foi possível ler as cotas salvas da CVM; arquivo preservado.".into())
        }
    };
    let mut bytes = Vec::new();
    file.take(MAX_CACHE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Leitura das cotas CVM interrompida.".to_string())?;
    if bytes.len() as u64 > MAX_CACHE {
        return Err("Cache de cotas CVM excede 32 MiB; arquivo preservado.".into());
    }
    let cache: Cache = serde_json::from_slice(&bytes)
        .map_err(|_| "Cache de cotas CVM inválido; arquivo preservado.".to_string())?;
    validate_cache(&cache, Utc::now())?;
    Ok(Some(cache))
}

fn validate_cache(cache: &Cache, now: DateTime<Utc>) -> Result<(), String> {
    let valid_stamp = |v: &str| {
        DateTime::parse_from_rfc3339(v).is_ok_and(|at| at <= now + chrono::Duration::minutes(5))
    };
    if cache.version != CACHE_VERSION || cache.funds.len() > 200 {
        return Err("Versão ou quantidade inválida no cache de cotas CVM.".into());
    }
    if let Some(register) = &cache.register {
        if !valid_stamp(&register.fetched_at)
            || register.funds.is_empty()
            || register.funds.len() > MAX_REGISTER_ROWS
        {
            return Err("Cadastro salvo de cotas CVM inválido.".into());
        }
        for (id, names) in &register.funds {
            if normalize_cnpj(id).as_deref() != Ok(id.as_str())
                || names.is_empty()
                || names.len() > 8
                || names.iter().any(|name| {
                    name.is_empty() || name.len() > 1024 || name.chars().any(char::is_control)
                })
            {
                return Err("Identidade inválida no cadastro salvo de cotas CVM.".into());
            }
        }
    }
    let mut count = 0;
    for (cnpj, months) in &cache.funds {
        if normalize_cnpj(cnpj).as_deref() != Ok(cnpj.as_str()) || months.len() > MAX_MONTHS {
            return Err("Fundo/período inválido no cache de cotas CVM.".into());
        }
        for (month, rows) in months {
            let first = parse_date(&format!("{month}-01"))?;
            if month_key(first) != *month
                || first.year() < 2021
                || first > brasilia_date(now)
                || !valid_stamp(&rows.fetched_at)
                || rows.observations.len() > 31
                || rows.issues.len() > 4
                || rows.issues.iter().any(|v| v.len() > 1024)
            {
                return Err("Mês inválido no cache de cotas CVM.".into());
            }
            let mut dates = BTreeSet::new();
            for item in &rows.observations {
                count += 1;
                let date = parse_date(&item.date)?;
                let fetched = DateTime::parse_from_rfc3339(&item.fetched_at)
                    .map_err(|_| "Data de coleta da cota salva inválida.".to_string())?
                    .with_timezone(&Utc);
                let value = Decimal::from_str(&item.value)
                    .map_err(|_| "Cota salva inválida.".to_string())?;
                if item.cnpj != *cnpj
                    || month_key(date) != *month
                    || date > brasilia_date(now)
                    || date > brasilia_date(fetched)
                    || value <= Decimal::ZERO
                    || value.normalize().to_string() != item.value
                    || item.source != FUND_NAV_SOURCE
                    || item.source_url != monthly_url(month)
                    || item.fetched_at != rows.fetched_at
                    || !dates.insert(item.date.clone())
                {
                    return Err("Observação inválida no cache de cotas CVM.".into());
                }
            }
        }
    }
    if count > 100_000 {
        return Err("Cache de cotas CVM excede o limite de observações.".into());
    }
    Ok(())
}

fn write_file(path: &Path, cache: &Cache) -> Result<(), String> {
    validate_cache(cache, Utc::now())?;
    let bytes = serde_json::to_vec(cache)
        .map_err(|_| "Não foi possível serializar as cotas CVM.".to_string())?;
    if bytes.len() as u64 > MAX_CACHE {
        return Err("Cache de cotas CVM excede 32 MiB.".into());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|_| "Não foi possível preparar o cache de cotas CVM.".to_string())?;
    }
    let temp = path.with_extension(format!("{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|_| "Não foi possível criar o cache temporário de cotas CVM.".to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "Não foi possível gravar as cotas CVM.".to_string())?;
        drop(file);
        fs::rename(&temp, path).map_err(|_| {
            "Não foi possível substituir o cache de cotas CVM; dados anteriores preservados."
                .to_string()
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;

    const ID: &str = "00332266000131";
    const OTHER: &str = "11222333000181";
    fn at() -> DateTime<Utc> {
        "2026-10-19T12:00:00Z".parse().unwrap()
    }
    fn request() -> FundNavRequest {
        FundNavRequest {
            fund_name: "Exemplo Pós-Fixado FIC FIRF Simples RL".into(),
            cnpj: None,
            start_date: "2026-09-01".into(),
            end_date: "2026-10-20".into(),
            anchor_dates: vec!["2026-09-01".into()],
        }
    }
    fn zip(files: &[(&str, &str)]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, body) in files {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(body.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }
    fn register() -> Register {
        let body=format!("CNPJ_Classe;Denominacao_Social;Tipo_Classe;Situacao\n{ID};EXEMPLO PÓS-FIXADO FUNDO DE INVESTIMENTO EM COTAS RENDA FIXA SIMPLES RESPONSABILIDADE LIMITADA;Classes de Cotas de Fundos FIF;Em Funcionamento Normal\n");
        parse_register(
            &zip(&[
                ("registro_classe.csv", &body),
                (
                    "registro_fundo.csv",
                    "CNPJ_Fundo;Denominacao_Social;Tipo_Fundo;Situacao\n",
                ),
            ]),
            at(),
        )
        .unwrap()
    }
    fn month(body: &str, modern: bool) -> Result<Month, String> {
        let h = if modern {
            "TP_FUNDO_CLASSE;CNPJ_FUNDO_CLASSE;ID_SUBCLASSE;DT_COMPTC;VL_QUOTA\n"
        } else {
            "TP_FUNDO;CNPJ_FUNDO;DT_COMPTC;VL_QUOTA\n"
        };
        parse_month(
            &zip(&[("inf_diario_fi_202609.csv", &format!("{h}{body}"))]),
            ID,
            "2026-09",
            at(),
        )
    }
    #[test]
    fn legal_aliases_are_exact_and_preserve_vehicle_identity() {
        let r = register();
        assert_eq!(resolve(Some(&r), &request()).unwrap().unwrap().0, ID);
        let mut q = request();
        q.fund_name = "Exemplo Pós-Fixado XP Seguros Prev FIC FIRF Simples RL".into();
        assert!(resolve(Some(&r), &q).unwrap().is_none());
        q.fund_name = "Exemplo Pós-Fixado FIRF Simples RL".into();
        assert!(resolve(Some(&r), &q).unwrap().is_none());
    }
    #[test]
    fn ambiguous_names_require_cnpj_and_inactive_wrong_types_are_excluded() {
        let mut r = register();
        r.funds.insert(OTHER.into(), r.funds[ID].clone());
        assert!(resolve(Some(&r), &request())
            .unwrap_err()
            .contains("Mais de um"));
        let mut q = request();
        q.cnpj = Some(ID.into());
        assert_eq!(resolve(Some(&r), &q).unwrap().unwrap().0, ID);
        let csv=format!("CNPJ_Fundo;Denominacao_Social;Tipo_Fundo;Situacao\n{ID};Exemplo;FII;Em Funcionamento Normal\n{OTHER};Exemplo;FI;Cancelada\n");
        assert!(parse_register(
            &zip(&[
                ("registro_fundo.csv", &csv),
                (
                    "registro_classe.csv",
                    "CNPJ_Classe;Denominacao_Social;Tipo_Classe;Situacao\n"
                )
            ]),
            at()
        )
        .is_err());
    }
    #[test]
    fn old_and_new_schemas_match_only_requested_cnpj() {
        let old = month(
            &format!("FI;{ID};2026-09-01;1.123456789012\nFI;{OTHER};2026-09-01;9999\n"),
            false,
        )
        .unwrap();
        let new = month(
            &format!("CLASSES - FIF;{ID};;2026-09-01;1.123456789012\n"),
            true,
        )
        .unwrap();
        assert_eq!(old.observations, new.observations);
        assert_eq!(old.observations.len(), 1);
        assert_eq!(old.observations[0].value, "1.123456789012");
    }
    #[test]
    fn subclasses_are_never_guessed() {
        let row = month(&format!("CLASSES - FIF;{ID};1;2026-09-01;1.2\n"), true).unwrap();
        assert!(row.observations.is_empty());
        assert!(row.issues[0].contains("subclasse"));
    }
    #[test]
    fn invalid_or_conflicting_navs_do_not_replace_good_data() {
        for body in [
            format!("FI;{ID};2026-10-01;1\n"),
            format!("FI;{ID};2026-09-01;1\nFI;{ID};2026-09-01;2\n"),
        ] {
            assert!(month(&body, false).is_err());
        }
        let equal = month(
            &format!("FI;{ID};2026-09-01;1.20\nFI;{ID};2026-09-01;1.2\n"),
            false,
        )
        .unwrap();
        assert_eq!(equal.observations.len(), 1);
    }
    #[test]
    fn invalid_dated_nav_does_not_discard_the_valid_monthly_close() {
        for invalid in ["0", "-1", "", "invalid"] {
            let row = month(
                &format!("FI;{ID};2026-09-01;{invalid}\nFI;{ID};2026-09-30;1.25\n"),
                false,
            )
            .unwrap();
            assert_eq!(row.observations.len(), 1);
            assert_eq!(row.observations[0].date, "2026-09-30");
            assert_eq!(row.observations[0].value, "1.25");
            assert_eq!(row.issues.len(), 1);
            assert!(row.issues[0].contains("2026-09-01"));
            assert!(row.issues[0].contains("sem interpolação"));
        }
    }
    #[test]
    fn a_valid_duplicate_cannot_replace_an_invalid_anchor_day_in_either_order() {
        for rows in [
            format!("FI;{ID};2026-09-01;0\nFI;{ID};2026-09-01;1.2\n"),
            format!("FI;{ID};2026-09-01;1.2\nFI;{ID};2026-09-01;0\n"),
        ] {
            let parsed = month(&format!("{rows}FI;{ID};2026-09-30;1.25\n"), false).unwrap();
            assert_eq!(parsed.observations.len(), 1);
            assert_eq!(parsed.observations[0].date, "2026-09-30");
            assert!(parsed
                .observations
                .iter()
                .all(|row| row.date != "2026-09-01"));
            assert_eq!(parsed.issues.len(), 1);
        }
        let only_invalid = month(&format!("FI;{ID};2026-09-01;0\n"), false).unwrap();
        assert!(only_invalid.observations.is_empty());
        assert!(only_invalid.issues[0].contains("2026-09-01"));
    }
    #[test]
    fn invalid_nav_does_not_hide_an_unidentified_subclass() {
        let row = month(
            &format!("CLASSES - FIF;{ID};1;2026-09-01;0\nCLASSES - FIF;{ID};;2026-09-30;1.25\n"),
            true,
        )
        .unwrap();
        assert!(row.observations.is_empty());
        assert!(row.issues.iter().any(|issue| issue.contains("subclasse")));
    }
    #[test]
    fn missing_months_are_pending_and_missing_nav_is_never_zero() {
        let mut cache = Cache {
            register: Some(register()),
            ..Cache::default()
        };
        let empty = month("", false).unwrap();
        cache
            .funds
            .entry(ID.into())
            .or_default()
            .insert("2026-09".into(), empty);
        let q = request();
        let output = panel(&cache, &q, &Range::new(&q, at()).unwrap(), vec![]);
        assert_eq!(output.pending_months, 1);
        assert!(output.observations.is_empty());
        assert!(output.issues.iter().any(|v| v.contains("não publicou")));
    }
    #[test]
    fn cache_validates_sources_and_round_trips_without_writing_financials() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("public-nav.json");
        let mut cache = Cache {
            register: Some(register()),
            ..Cache::default()
        };
        cache.funds.entry(ID.into()).or_default().insert(
            "2026-09".into(),
            month(&format!("FI;{ID};2026-09-01;1.2\n"), false).unwrap(),
        );
        write_file(&path, &cache).unwrap();
        let original = fs::read(&path).unwrap();
        let service = CvmNavService::new(path.clone()).unwrap();
        assert_eq!(
            service.read_cache(&request()).unwrap().observations.len(),
            1
        );
        assert_eq!(original, fs::read(&path).unwrap());
        cache
            .funds
            .get_mut(ID)
            .unwrap()
            .get_mut("2026-09")
            .unwrap()
            .observations[0]
            .source_url = "https://example.com".into();
        assert!(write_file(&path, &cache).is_err());
        assert_eq!(original, fs::read(&path).unwrap());
    }
    #[test]
    fn readonly_missing_cache_creates_nothing_and_invalid_cache_is_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("absent").join("nav.json");
        let service = CvmNavService::new(path.clone()).unwrap();
        assert_eq!(service.read_cache(&request()).unwrap().pending_months, 2);
        assert!(!path.parent().unwrap().exists());
        fs::create_dir(path.parent().unwrap()).unwrap();
        fs::write(&path, b"broken").unwrap();
        assert!(service.read_cache(&request()).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"broken");
    }
    #[test]
    fn anchor_months_precede_latest_months_and_dates_are_validated() {
        let mut q = request();
        q.start_date = "2023-01-20".into();
        q.anchor_dates = vec![
            "2023-01-20".into(),
            "2024-04-08".into(),
            "2024-12-11".into(),
        ];
        let range = Range::new(&q, at()).unwrap();
        let work = prioritise(&range.months(), &range.anchors);
        assert_eq!(&work[..4], &["2023-01", "2024-04", "2024-12", "2026-10"]);
        assert_eq!(work.len(), 46);
        q.anchor_dates.push("2022-01-01".into());
        assert!(Range::new(&q, at()).is_err());
        q.anchor_dates.clear();
        q.start_date = "2020-01-01".into();
        assert!(Range::new(&q, at()).is_err());
        assert!(!valid_monthly_url("https://example.com/202610.zip"));
        assert!(!valid_monthly_url(
            "https://dados.cvm.gov.br/dados/FI/DOC/INF_DIARIO/DADOS/inf_diario_fi_202613.zip"
        ));
    }
    #[tokio::test]
    async fn shared_refresh_reservations_dedupe_failures_without_blocking_missing_successes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("nav.json");
        let now = Utc::now();
        {
            let mut gate = RefreshGate::acquire(&path).await.unwrap();
            assert!(gate
                .reserve("cvm-nav:month1", None, 0, now, Some(48))
                .unwrap()
                .is_none());
        }
        let mut gate = RefreshGate::acquire(&path).await.unwrap();
        assert!(gate
            .reserve("cvm-nav:month1", None, 0, now, Some(48))
            .unwrap()
            .is_some());
        assert!(gate
            .reserve("cvm-nav:month2", None, 0, now, Some(48))
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn fresh_complete_history_is_reused_without_any_http() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("nav.json");
        let mut cache = Cache {
            register: Some(register()),
            ..Cache::default()
        };
        cache.register.as_mut().unwrap().fetched_at = stamp(Utc::now());
        let mut saved = month(&format!("FI;{ID};2026-09-01;1.2\n"), false).unwrap();
        saved.fetched_at = stamp(Utc::now());
        saved
            .observations
            .iter_mut()
            .for_each(|row| row.fetched_at = saved.fetched_at.clone());
        cache
            .funds
            .entry(ID.into())
            .or_default()
            .insert("2026-09".into(), saved);
        write_file(&path, &cache).unwrap();
        let original = fs::read(&path).unwrap();
        let service = CvmNavService::new(path.clone()).unwrap();
        let mut q = request();
        q.end_date = "2026-09-30".into();
        let result = service.refresh_if_due(&q).await;
        assert_eq!(result.status, "cached");
        let panel = result.data.unwrap();
        assert_eq!(panel.pending_months, 0);
        assert_eq!(panel.observations.len(), 1);
        assert!(panel.issues.is_empty());
        assert_eq!(fs::read(&path).unwrap(), original);
        // No reservation is written when no source is due.
        assert!(!path.with_extension("refresh-policy.json").exists());
    }
    #[test]
    fn malformed_archives_headers_and_unrelated_tables_are_rejected() {
        assert!(parse_month(b"not a zip", ID, "2026-09", at()).is_err());
        assert!(parse_month(&zip(&[("other.csv", "x")]), ID, "2026-09", at()).is_err());
        assert!(parse_month(
            &zip(&[(
                "inf_diario_fi_202609.csv",
                "CNPJ_FUNDO;DT_COMPTC;VL_QUOTA\n"
            )]),
            ID,
            "2026-09",
            at()
        )
        .is_err());
        assert!(normalize_cnpj("00000000000000").is_err());
        assert!(normalize_cnpj("00332266000132").is_err());
    }

    #[test]
    #[ignore = "manual validation with previously downloaded official CVM ZIPs; no network"]
    fn local_official_archives_match_configured_identity() {
        let directory =
            PathBuf::from(std::env::var_os("IVESTFII_NAV_FIXTURES").expect("fixture directory"));
        let cnpj =
            normalize_cnpj(&std::env::var("IVESTFII_NAV_CNPJ").expect("public fund CNPJ")).unwrap();
        let name = std::env::var("IVESTFII_NAV_NAME").expect("public fund name");
        let register = parse_register(
            &fs::read(directory.join("registro_fundo_classe.zip")).unwrap(),
            Utc::now(),
        )
        .unwrap();
        let mut q = request();
        q.fund_name = name;
        assert_eq!(resolve(Some(&register), &q).unwrap().unwrap().0, cnpj);
        let mut archives = 0;
        for path in fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
        {
            let filename = path.file_name().unwrap().to_string_lossy();
            let Some(ym) = filename
                .strip_prefix("inf_diario_fi_")
                .and_then(|s| s.strip_suffix(".zip"))
            else {
                continue;
            };
            assert_eq!(ym.len(), 6);
            let month = format!("{}-{}", &ym[..4], &ym[4..]);
            let parsed = parse_month(&fs::read(&path).unwrap(), &cnpj, &month, Utc::now()).unwrap();
            assert!(!parsed.observations.is_empty());
            assert!(
                parsed.issues.iter().all(|issue| issue
                    .starts_with("A CVM informou uma cota inválida ou não positiva em ")),
                "Only isolated invalid-day warnings are accepted in these public fixtures"
            );
            assert!(parsed
                .observations
                .iter()
                .all(|row| Decimal::from_str(&row.value).is_ok_and(|value| value > Decimal::ZERO)));
            archives += 1;
        }
        assert!(archives > 0);
    }
}
