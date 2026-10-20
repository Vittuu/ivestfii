//! Dated public FII income events. Only a public ticker leaves this service.
//! This cache never edits holdings or turns an expected payment into a receipt.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use reqwest::{redirect::Policy, Client};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    quotes::normalize_symbol,
    refresh_policy::{fresh, RefreshGate, RefreshOutcome},
};

const ENDPOINT: &str =
    "https://sistemaswebb3-listados.b3.com.br/fundsListedProxy/Search/GetEventsCorporateActions/";
const B3_DAILY_LIMIT: u32 = 40;
const TTL: i64 = 24 * 3600;
const RESPONSE_LIMIT: usize = 2 * 1024 * 1024;
const CACHE_LIMIT: usize = 16 * 1024 * 1024;
const MAX_SYMBOLS: usize = 100;
const MAX_EVENTS: usize = 5000;
// The network window is bounded independently from the growing local archive.
// Never evict an old payment merely to fit a cache limit: reject the update and
// preserve the previous file if either archive bound is reached.
const MAX_ARCHIVED_EVENTS: usize = 50_000;
const MAX_CACHED_SYMBOLS: usize = 2000;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DividendPanel {
    pub items: Vec<DividendItem>,
    pub fetched_at: Option<String>,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DividendItem {
    pub symbol: String,
    /// available, cached, unsupported, not_configured, unauthorized,
    /// rate_limited or unavailable. Empty events never mean a zero payment.
    pub status: String,
    pub observation: Option<DividendObservation>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DividendObservation {
    pub symbol: String,
    pub events: Vec<DividendEvent>,
    pub currency: String,
    pub fetched_at: String,
    pub source: String,
    pub source_url: String,
    /// This public B3 feed is a recent-event window, not full-lifetime history.
    pub coverage_warning: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DividendEvent {
    pub event_id: String,
    pub symbol: String,
    /// Exact decimal BRL per share. Never parsed through f64.
    pub per_share: String,
    /// income, amortization, jcp or other. Capital refunds are not income.
    pub kind: String,
    pub label: String,
    /// Payment calendar date; null is not replaced by approval/data-com.
    pub payment_date: Option<String>,
    /// lastDatePrior = data-com: purchases on this day remain eligible.
    pub record_date: Option<String>,
    /// exDate = first calendar day without entitlement, when provided.
    pub ex_date: Option<String>,
    pub declared_date: Option<String>,
    /// exact or incomplete. A missing date is never replaced by another date.
    pub date_precision: String,
    pub verified: Option<bool>,
    pub remarks: Option<String>,
    pub related_to: Option<String>,
    pub isin_code: Option<String>,
    pub source_url: String,
    /// Original collection time for this version of the event. Archive rollover
    /// must not make an old payment appear to have been observed again today.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_observed_at: Option<String>,
    /// The source revised a boundary event without a unique stable identity.
    /// Retain the evidence, but do not count either possible version as money.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reconciliation_pending: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedItem {
    observation: Option<DividendObservation>,
    checked_at: String,
    status: String,
    error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Cache {
    version: u32,
    items: BTreeMap<String, SavedItem>,
    retry_after: Option<String>,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            version: 1,
            items: BTreeMap::new(),
            retry_after: None,
        }
    }
}

#[derive(Clone)]
pub struct DividendService {
    client: Client,
    cache_path: PathBuf,
}

impl DividendService {
    pub fn new(cache_path: PathBuf) -> Result<Self, String> {
        if cache_path.file_name().is_none() {
            return Err("Informe um arquivo para o cache de proventos.".into());
        }
        let client = Client::builder()
            .https_only(true)
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .user_agent("IvestFII/0.1 (public B3 FII income events)")
            .build()
            .map_err(|_| "Não foi possível preparar a consulta de proventos.".to_string())?;
        Ok(Self { client, cache_path })
    }

    /// Local-only; reading creates neither directories nor reservation files.
    pub fn read_cache(&self, symbols: &[String]) -> Result<DividendPanel, String> {
        let symbols = normalize_symbols(symbols)?;
        Ok(panel(&symbols, &read_cache(&self.cache_path)?))
    }

    pub async fn refresh_if_due(&self, symbols: &[String]) -> RefreshOutcome<DividendPanel> {
        match self.refresh(symbols).await {
            Ok(data)
                if data.errors.is_empty() && data.items.iter().any(|x| x.status == "available") =>
            {
                RefreshOutcome::updated(data)
            }
            Ok(data) if data.errors.is_empty() => RefreshOutcome::cached(data),
            Ok(data) => {
                let message = data.errors.join(" ");
                let status = if data.items.iter().any(|x| x.observation.is_some()) {
                    "cached"
                } else if data.items.iter().all(|x| x.status == "unsupported") {
                    "unsupported"
                } else {
                    "unavailable"
                };
                RefreshOutcome {
                    data: Some(data),
                    status: status.into(),
                    message: Some(message),
                }
            }
            Err(error) => RefreshOutcome::failed(self.read_cache(symbols).ok(), error),
        }
    }

    /// Public B3 feed: no credential, financial quantity or purchase date is
    /// transmitted. Manual/MCP/automatic modes share 24h cache and 40 HTTP/day.
    pub async fn refresh(&self, symbols: &[String]) -> Result<DividendPanel, String> {
        let symbols = normalize_symbols(symbols)?;
        if symbols.is_empty() {
            return Ok(DividendPanel::default());
        }
        let mut gate = RefreshGate::acquire(&self.cache_path).await?;
        let mut cached = read_cache(&self.cache_path)?;
        if let Some(until) = cached
            .retry_after
            .as_deref()
            .and_then(parse_time)
            .filter(|at| *at > Utc::now())
        {
            return Ok(error_panel(
                &symbols,
                &cached,
                "rate_limited",
                &format!(
                    "A B3 limitou temporariamente as consultas. Nova tentativa após {}.",
                    stamp(until)
                ),
            ));
        }
        let mut fresh_symbols = BTreeSet::new();
        let mut transient = BTreeMap::new();
        let mut stop = None::<(String, String)>;
        for symbol in &symbols {
            let now = Utc::now();
            if cached.items.get(symbol).is_some_and(|item| {
                let ttl = if item.status == "available" || item.status == "unsupported" {
                    TTL
                } else {
                    300
                };
                fresh(Some(&item.checked_at), ttl, now)
            }) {
                continue;
            }
            if let Some((status, message)) = &stop {
                transient.insert(symbol.clone(), (status.clone(), message.clone()));
                continue;
            }
            if fund_code(symbol).is_err() {
                cached.items.insert(symbol.clone(), SavedItem { observation: None, checked_at: stamp(now), status: "unsupported".into(), error: Some("A identidade deste ticker não foi confirmada no serviço público de fundos da B3.".into()) });
                store_cache(&self.cache_path, &cached)?;
                continue;
            }
            if let Some(message) = gate.reserve(
                &format!("dividends:{symbol}"),
                None,
                TTL,
                now,
                Some(B3_DAILY_LIMIT),
            )? {
                transient.insert(
                    symbol.clone(),
                    (
                        "rate_limited".into(),
                        message.replace("de cotações", "de proventos B3"),
                    ),
                );
                continue;
            }
            let result = self.fetch(symbol).await;
            let checked_at = stamp(Utc::now());
            match result {
                Ok(observation) => {
                    replace_observation(&mut cached, symbol, observation, checked_at);
                    if cached.items[symbol].error.is_none() {
                        fresh_symbols.insert(symbol.clone());
                    }
                    store_cache(&self.cache_path, &cached)?;
                }
                Err(failure) => {
                    if let Some(until) = failure.retry_after {
                        cached.retry_after = Some(stamp(until));
                    }
                    if failure.status == "rate_limited" {
                        stop = Some((failure.status.clone(), failure.message.clone()));
                    }
                    let previous = cached.items.get(symbol).and_then(|x| x.observation.clone());
                    cached.items.insert(
                        symbol.clone(),
                        SavedItem {
                            observation: previous,
                            checked_at,
                            status: failure.status,
                            error: Some(failure.message),
                        },
                    );
                    store_cache(&self.cache_path, &cached)?;
                }
            }
        }
        let mut result = panel(&symbols, &cached);
        for item in &mut result.items {
            if fresh_symbols.contains(&item.symbol) {
                item.status = "available".into();
            }
            if let Some((status, message)) = transient.get(&item.symbol) {
                item.status = if item.observation.is_some() {
                    "cached"
                } else {
                    status
                }
                .into();
                item.error = Some(message.clone());
            }
        }
        result.errors = result
            .items
            .iter()
            .filter_map(|x| x.error.as_ref().map(|e| format!("{}: {e}", x.symbol)))
            .collect();
        Ok(result)
    }

    async fn fetch(&self, symbol: &str) -> Result<DividendObservation, Failure> {
        let url = fetch_url(symbol).map_err(|e| Failure::new("unsupported", e))?;
        let mut response = self.client.get(url).send().await.map_err(|_| {
            Failure::new(
                "unavailable",
                "Não foi possível consultar os proventos da B3. Os dados salvos foram mantidos.",
            )
        })?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(http_failure(
                status,
                response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|x| x.to_str().ok()),
                Utc::now(),
            ));
        }
        if response
            .content_length()
            .is_some_and(|x| x > RESPONSE_LIMIT as u64)
        {
            return Err(Failure::new(
                "unavailable",
                "A resposta de proventos excedeu 2 MiB.",
            ));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| Failure::new("unavailable", "A resposta de proventos foi interrompida."))?
        {
            if chunk.len() > RESPONSE_LIMIT.saturating_sub(bytes.len()) {
                return Err(Failure::new(
                    "unavailable",
                    "A resposta de proventos excedeu 2 MiB.",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        parse_b3_dividends(&bytes, symbol, Utc::now()).map_err(|e| Failure::new("unavailable", e))
    }
}

#[derive(Debug)]
struct Failure {
    status: String,
    message: String,
    retry_after: Option<DateTime<Utc>>,
}
impl Failure {
    fn new(status: &str, message: impl Into<String>) -> Self {
        Self {
            status: status.into(),
            message: message.into(),
            retry_after: None,
        }
    }
}
fn http_failure(code: u16, retry: Option<&str>, now: DateTime<Utc>) -> Failure {
    match code {
        403 => Failure::new("unavailable", "A B3 recusou temporariamente esta consulta pública. Os proventos salvos foram preservados."),
        429 => {
            let until = retry.and_then(|s| s.parse::<u64>().ok().filter(|x| *x <= 31_536_000).map(|x| now + chrono::Duration::seconds(x.max(1) as i64)).or_else(|| DateTime::parse_from_rfc2822(s).ok().map(|x| x.with_timezone(&Utc)))).filter(|x| *x > now).unwrap_or(now + chrono::Duration::seconds(300));
            Failure { status: "rate_limited".into(), message: format!("A B3 limitou as consultas de proventos. Aguarde até {}.", stamp(until)), retry_after: Some(until) }
        }
        400 | 404 => Failure::new("unsupported", "O histórico público de proventos não está disponível para este ticker na B3."),
        _ => Failure::new("unavailable", format!("A B3 está indisponível para proventos (HTTP {code}).")),
    }
}

fn fund_code(symbol: &str) -> Result<&str, String> {
    if symbol.len() != 6
        || !symbol.ends_with("11")
        || !symbol.as_bytes()[..4].iter().all(u8::is_ascii_uppercase)
    {
        return Err("A identidade deste ticker de fundo não foi confirmada na B3.".into());
    }
    Ok(&symbol[..4])
}
fn source_url(symbol: &str) -> Result<String, String> {
    let code = fund_code(symbol)?;
    Ok(format!(
        "https://sistemaswebb3-listados.b3.com.br/fundsListedPage/events-main/{code}"
    ))
}
fn fetch_url(symbol: &str) -> Result<String, String> {
    let code = fund_code(symbol)?;
    let query = format!("{{\"language\":\"pt-br\",\"idCEM\":\"{code}\"}}");
    Ok(format!("{ENDPOINT}{}", STANDARD.encode(query)))
}
fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|x| x.with_timezone(&Utc))
}
fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}
fn normalize_symbols(symbols: &[String]) -> Result<Vec<String>, String> {
    if symbols.len() > MAX_SYMBOLS {
        return Err("Consulte no máximo 100 fundos por atualização de proventos.".into());
    }
    symbols
        .iter()
        .map(|s| normalize_symbol(s))
        .collect::<Result<BTreeSet<_>, _>>()
        .map(|s| s.into_iter().collect())
}

fn decimal(raw: &Value) -> Result<String, String> {
    let text = match raw {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return Err("A B3 não retornou um valor decimal de provento.".into()),
    };
    if text.len() > 50 {
        return Err("O provento excedeu a precisão aceita.".into());
    }
    let value = Decimal::from_str_exact(&text)
        .map_err(|_| "A B3 retornou provento inválido.".to_string())?;
    if value < Decimal::ZERO || value > Decimal::from(1_000_000_000u64) {
        return Err("A B3 retornou provento fora dos limites aceitos.".into());
    }
    Ok(value.normalize().to_string())
}

/// B3 dates are explicit calendar dates, not timezone-dependent instants.
fn date(raw: Option<&Value>) -> Result<Option<String>, String> {
    let Some(raw) = raw.filter(|x| !x.is_null()) else {
        return Ok(None);
    };
    let text = raw
        .as_str()
        .ok_or("A B3 retornou uma data de provento inválida.")?
        .trim();
    if text.is_empty() {
        return Ok(None);
    }
    let format = if text.as_bytes().get(2) == Some(&b'/') {
        "%d/%m/%Y"
    } else {
        "%Y-%m-%d"
    };
    let parsed =
        NaiveDate::parse_from_str(text, format).map_err(|_| "A data do provento B3 é inválida.")?;
    if text.len() != 10 || parsed.format(format).to_string() != text {
        return Err("A data do provento B3 é ambígua.".into());
    }
    Ok(Some(parsed.format("%Y-%m-%d").to_string()))
}
fn text(raw: Option<&Value>, max: usize) -> Result<Option<String>, String> {
    match raw {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.len() <= max => Ok((!s.is_empty()).then(|| s.clone())),
        _ => Err("A B3 retornou metadados de proventos inválidos.".into()),
    }
}
fn event_id(event: &DividendEvent) -> String {
    // Dates/ISIN/type define a payment; rate is excluded to allow corrections.
    let identity = serde_json::to_vec(&(
        &event.symbol,
        &event.payment_date,
        &event.record_date,
        &event.declared_date,
        &event.label,
        &event.isin_code,
        &event.related_to,
    ))
    .expect("serializable event identity");
    format!("b3:{:x}", Sha256::digest(identity))
}
fn normal_isin(code: &str, isin: &str) -> bool {
    let prefix = format!("BR{code}CTF");
    isin.len() == 12
        && isin.starts_with(&prefix)
        && isin.as_bytes()[9..].iter().all(u8::is_ascii_digit)
}

fn replace_observation(
    cache: &mut Cache,
    symbol: &str,
    observation: DividendObservation,
    checked_at: String,
) {
    let previous = cache.items.get(symbol).and_then(|x| x.observation.as_ref());
    let (observation, error) = match merge_archive(previous, observation) {
        Ok(result) => result,
        Err(message) => {
            // Cache limits are failures, never an instruction to truncate money.
            if let Some(item) = cache.items.get_mut(symbol) {
                item.checked_at = checked_at;
                item.status = "unavailable".into();
                item.error = Some(message);
            }
            return;
        }
    };
    cache.items.insert(
        symbol.into(),
        SavedItem {
            observation: Some(observation),
            checked_at,
            status: if error.is_some() {
                "unavailable"
            } else {
                "available"
            }
            .into(),
            error,
        },
    );
}

fn event_family(event: &DividendEvent) -> (&str, &Option<String>, &str, &str) {
    (&event.symbol, &event.isin_code, &event.kind, &event.label)
}

/// Declaration and reference competence survive payment/data-com corrections.
/// They are matching hints only: uniqueness is checked on both source windows.
fn stable_keys(event: &DividendEvent) -> Vec<String> {
    [
        ("declared", event.declared_date.as_deref()),
        ("related", event.related_to.as_deref().map(str::trim)),
    ]
    .into_iter()
    .filter_map(|(field, value)| {
        value.filter(|value| !value.is_empty()).map(|value| {
            serde_json::to_string(&(event_family(event), field, value))
                .expect("serializable event reference")
        })
    })
    .collect()
}

fn merge_archive(
    previous: Option<&DividendObservation>,
    mut current: DividendObservation,
) -> Result<(DividendObservation, Option<String>), String> {
    let Some(previous) = previous else {
        return Ok((current, None));
    };
    if current.events.is_empty() && !previous.events.is_empty() {
        return Ok((previous.clone(), Some("A B3 retornou uma lista vazia. O histórico salvo foi mantido; isso não confirma ausência de pagamentos.".into())));
    }
    // A dated event is required to locate the beginning of the source window.
    // Undated-only responses cannot erase known historical payments either.
    let Some(boundary) = current
        .events
        .iter()
        .filter_map(|event| event.payment_date.clone())
        .min()
    else {
        return if previous.events.is_empty() {
            Ok((current, None))
        } else {
            Ok((previous.clone(), Some("A B3 não informou datas de pagamento na atualização. O histórico salvo foi mantido até a fonte esclarecer os eventos.".into())))
        };
    };
    let fresh_ids: BTreeSet<_> = current
        .events
        .iter()
        .map(|event| event.event_id.clone())
        .collect();
    let old_ids: BTreeSet<_> = previous
        .events
        .iter()
        .map(|event| event.event_id.as_str())
        .collect();
    let old_pending: BTreeSet<_> = previous
        .events
        .iter()
        .filter(|event| event.reconciliation_pending)
        .map(|event| event.event_id.as_str())
        .collect();
    let mut archive: Vec<_> = previous
        .events
        .iter()
        .filter(|event| {
            !fresh_ids.contains(event.event_id.as_str())
                && event
                    .payment_date
                    .as_deref()
                    .is_some_and(|date| date < boundary.as_str())
        })
        .cloned()
        .collect();
    for event in &mut current.events {
        event.reconciliation_pending |= old_pending.contains(event.event_id.as_str());
    }
    for event in &mut archive {
        if event.last_observed_at.is_none() {
            event.last_observed_at = Some(previous.fetched_at.clone());
        }
    }
    let mut old_keys: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut new_keys: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    for event in &previous.events {
        for key in stable_keys(event) {
            old_keys
                .entry(key)
                .or_default()
                .insert(event.event_id.clone());
        }
    }
    for (index, event) in current.events.iter().enumerate() {
        for key in stable_keys(event) {
            new_keys.entry(key).or_default().insert(index);
        }
    }
    let archive_ids: BTreeSet<_> = archive.iter().map(|event| event.event_id.clone()).collect();
    let mut archive_families = BTreeMap::<String, (BTreeSet<String>, BTreeSet<String>)>::new();
    for event in &archive {
        let family =
            serde_json::to_string(&event_family(event)).expect("serializable event family");
        let (all, missing_reference) = archive_families.entry(family).or_default();
        all.insert(event.event_id.clone());
        if stable_keys(event).is_empty() {
            missing_reference.insert(event.event_id.clone());
        }
    }
    let mut replaced = BTreeSet::new();
    let mut pending_old = BTreeSet::new();
    let mut pending_new = BTreeSet::new();
    let mut pending_families = BTreeSet::new();
    let mut pending_reference_missing = BTreeSet::new();
    for (index, event) in current.events.iter().enumerate() {
        // An identical dated identity was already observed: it is not a newly
        // appearing version of an archived event with a shared declaration.
        if old_ids.contains(event.event_id.as_str()) {
            continue;
        }
        let keys = stable_keys(event);
        let candidates: BTreeSet<_> = keys
            .iter()
            .filter_map(|key| old_keys.get(key))
            .flatten()
            .cloned()
            .collect();
        let archived_candidates: BTreeSet<_> =
            candidates.intersection(&archive_ids).cloned().collect();
        if !archived_candidates.is_empty() {
            let fresh_matches: BTreeSet<_> = keys
                .iter()
                .filter_map(|key| new_keys.get(key))
                .flatten()
                .copied()
                .collect();
            if candidates.len() == 1 && fresh_matches.len() == 1 {
                replaced.extend(archived_candidates);
            } else {
                pending_old.extend(archived_candidates);
                pending_new.extend(fresh_matches);
            }
        }
        // Without a reference on one side, a moved payment cannot be
        // distinguished safely from a new payment. Quarantine only this family
        // instead of freezing unrelated, unambiguous payments in the response.
        let family =
            serde_json::to_string(&event_family(event)).expect("serializable event family");
        if let Some((all, missing_reference)) = archive_families.get(&family) {
            if keys.is_empty() && !all.is_empty() {
                pending_families.insert(family);
                pending_new.insert(index);
            } else if !missing_reference.is_empty() {
                pending_reference_missing.insert(family);
                pending_new.insert(index);
            }
        }
    }
    for family in pending_families {
        pending_old.extend(archive_families[&family].0.iter().cloned());
    }
    for family in pending_reference_missing {
        pending_old.extend(archive_families[&family].1.iter().cloned());
    }
    archive.retain(|event| !replaced.contains(&event.event_id));
    for event in &mut archive {
        event.reconciliation_pending |= pending_old.contains(&event.event_id);
    }
    for (index, event) in current.events.iter_mut().enumerate() {
        event.reconciliation_pending |= pending_new.contains(&index);
    }
    current.events.extend(archive);
    if current.events.len() > MAX_ARCHIVED_EVENTS {
        return Err("O histórico local de proventos atingiu o limite seguro. A atualização foi suspensa e o arquivo anterior foi preservado; nenhum pagamento foi descartado.".into());
    }
    current
        .events
        .sort_by(|a, b| (&a.payment_date, &a.event_id).cmp(&(&b.payment_date, &b.event_id)));
    current.coverage_warning = "A B3 publica uma janela recente de eventos. Pagamentos anteriores já consultados são preservados no histórico local; meses nunca consultados podem estar incompletos.".into();
    let error = current.events.iter().any(|event| event.reconciliation_pending).then(||
        "Há proventos com revisão de datas sem identificação única na B3. As versões foram preservadas para conferência e não foram somadas aos rendimentos; os demais eventos foram atualizados.".into());
    Ok((current, error))
}

fn income_kind(label: &str) -> &'static str {
    let label = label.to_uppercase();
    if label.contains("AMORT") || label.contains("REST CAP") {
        "amortization"
    } else if label.contains("RENDIMENTO") || label.contains("DIVIDENDO") {
        "income"
    } else if label.contains("JCP") || label.contains("JRS CAP") || label.contains("JUROS SOBRE") {
        "jcp"
    } else {
        "other"
    }
}

/// Only ordinary-share income reaches the portfolio calculation. Missing dates
/// remain empty so the core can mark an otherwise partial month unresolved.
/// Future payments are excluded by the core's explicit `today`; capital refunds
/// and unverified asset identities never count as income.
pub fn income_events(panel: &DividendPanel) -> Vec<ivestfii_core::income::IncomeEvent> {
    panel
        .items
        .iter()
        .filter_map(|item| item.observation.as_ref())
        .flat_map(|observation| {
            observation.events.iter().filter_map(move |event| {
                let code = fund_code(&event.symbol).ok()?;
                if observation.source != "b3"
                    || observation.symbol != event.symbol
                    || event.source_url != observation.source_url
                    || source_url(&event.symbol).ok().as_ref() != Some(&event.source_url)
                    || event.kind != "income"
                    || income_kind(&event.label) != "income"
                    || !["exact", "incomplete"].contains(&event.date_precision.as_str())
                    || event.verified != Some(true)
                    || !event
                        .isin_code
                        .as_deref()
                        .is_some_and(|isin| normal_isin(code, isin))
                {
                    return None;
                }
                Some(ivestfii_core::income::IncomeEvent {
                    symbol: event.symbol.clone(),
                    amount_per_share: event.per_share.clone(),
                    // An unresolved source revision is retained as incomplete
                    // evidence, so another event in this month cannot make the
                    // core report a falsely complete monetary total.
                    record_date: if event.reconciliation_pending {
                        String::new()
                    } else {
                        event.record_date.clone().unwrap_or_default()
                    },
                    payment_date: event.payment_date.clone().unwrap_or_default(),
                    source_url: event.source_url.clone(),
                    source: "B3 · eventos corporativos públicos de fundos".into(),
                    id: event.event_id.clone(),
                })
            })
        })
        .collect()
}

/// Parses the public B3 fund page feed, rejecting a different issuer and
/// excluding subscription receipts. Those receipts have their own entitlement
/// and must never be multiplied by ordinary FII11 holdings.
pub fn parse_b3_dividends(
    bytes: &[u8],
    requested_symbol: &str,
    fetched_at: DateTime<Utc>,
) -> Result<DividendObservation, String> {
    if bytes.len() > RESPONSE_LIMIT {
        return Err("A resposta de proventos excedeu 2 MiB.".into());
    }
    let symbol = normalize_symbol(requested_symbol)?;
    let code = fund_code(&symbol)?;
    let root: Value =
        serde_json::from_slice(bytes).map_err(|_| "A B3 retornou JSON de proventos inválido.")?;
    if root.get("code").and_then(Value::as_str) != Some(code) {
        return Err("A B3 retornou a identidade de outro fundo; proventos ignorados.".into());
    }
    let raw_events = root
        .get("cashDividends")
        .and_then(Value::as_array)
        .ok_or("A B3 não retornou eventos em dinheiro; ausência não significa rendimento zero.")?;
    if raw_events.len() > MAX_EVENTS {
        return Err("A B3 retornou eventos demais; dados ignorados.".into());
    }
    let ordinary_isins: BTreeSet<_> = raw_events
        .iter()
        .filter_map(|e| {
            e.get("isinCode")
                .or_else(|| e.get("assetIssued"))
                .and_then(Value::as_str)
        })
        .filter(|isin| normal_isin(code, isin))
        .collect();
    if ordinary_isins.len() > 1 {
        return Err("A B3 retornou mais de uma classe de cotas ordinárias; confirme a identidade antes de calcular.".into());
    }
    if !raw_events.is_empty() && ordinary_isins.is_empty() {
        return Err(
            "A B3 não confirmou o ISIN das cotas deste fundo; recibos não foram usados.".into(),
        );
    }
    let source_url = source_url(&symbol)?;
    let mut unique = BTreeMap::<String, DividendEvent>::new();
    for raw in raw_events {
        let isin = text(raw.get("isinCode").or_else(|| raw.get("assetIssued")), 50)?
            .ok_or("O evento não informa o ISIN.")?;
        if !normal_isin(code, &isin) {
            continue;
        }
        if let Some(issued) = text(raw.get("assetIssued"), 50)? {
            if issued != isin {
                return Err("A B3 retornou identidade de ativo inconsistente no provento.".into());
            }
        }
        let label = text(raw.get("label"), 200)?.ok_or("O tipo do provento não foi informado.")?;
        let kind = income_kind(&label);
        let raw_rate = raw
            .get("rate")
            .ok_or("A B3 não informou o valor por cota.")?;
        let rate = match raw_rate {
            Value::String(s) => Value::String(s.replace(',', ".")),
            other => other.clone(),
        };
        let mut event = DividendEvent {
            event_id: String::new(),
            symbol: symbol.clone(),
            per_share: decimal(&rate)?,
            kind: kind.into(),
            label,
            payment_date: date(raw.get("paymentDate"))?,
            record_date: date(raw.get("lastDatePrior"))?,
            ex_date: None,
            declared_date: date(raw.get("approvedOn"))?,
            date_precision: String::new(),
            verified: Some(true),
            remarks: text(raw.get("remarks"), 4000)?,
            related_to: text(raw.get("relatedTo"), 500)?,
            isin_code: Some(isin),
            source_url: source_url.clone(),
            last_observed_at: Some(stamp(fetched_at)),
            reconciliation_pending: false,
        };
        event.date_precision = if event.payment_date.is_some() && event.record_date.is_some() {
            "exact"
        } else {
            "incomplete"
        }
        .into();
        event.event_id = event_id(&event);
        if let Some(previous) = unique.get(&event.event_id) {
            if previous != &event {
                return Err(
                    "A B3 retornou proventos duplicados com valores conflitantes; dados ignorados."
                        .into(),
                );
            }
        } else {
            unique.insert(event.event_id.clone(), event);
        }
    }
    let mut events: Vec<_> = unique.into_values().collect();
    events.sort_by(|a, b| (&a.payment_date, &a.event_id).cmp(&(&b.payment_date, &b.event_id)));
    Ok(DividendObservation { symbol,events,currency:"BRL".into(),fetched_at:stamp(fetched_at),source:"b3".into(),source_url,
        coverage_warning:"A B3 publica uma janela recente de eventos. Meses sem evento na fonte não comprovam ausência de pagamento.".into() })
}

fn panel(symbols: &[String], cache: &Cache) -> DividendPanel {
    let items: Vec<_> = symbols
        .iter()
        .map(|symbol| {
            let saved = cache.items.get(symbol);
            DividendItem {
                symbol: symbol.clone(),
                status: saved
                    .map(|x| {
                        if x.observation.is_some() {
                            "cached"
                        } else {
                            &x.status
                        }
                    })
                    .unwrap_or("unavailable")
                    .into(),
                observation: saved.and_then(|x| x.observation.clone()),
                error: saved.and_then(|x| x.error.clone()),
            }
        })
        .collect();
    DividendPanel {
        fetched_at: symbols
            .iter()
            .filter_map(|s| cache.items.get(s).map(|x| x.checked_at.clone()))
            .max(),
        errors: items
            .iter()
            .filter_map(|x| x.error.as_ref().map(|e| format!("{}: {e}", x.symbol)))
            .collect(),
        items,
    }
}
fn error_panel(symbols: &[String], cache: &Cache, status: &str, message: &str) -> DividendPanel {
    let mut result = panel(symbols, cache);
    for item in &mut result.items {
        item.status = if item.observation.is_some() {
            "cached"
        } else {
            status
        }
        .into();
        item.error = Some(message.into());
    }
    result.errors = vec![message.into()];
    result
}
fn validate(cache: &Cache) -> Result<(), String> {
    let invalid = || "Cache de proventos inválido; arquivo preservado.".to_string();
    if cache.version != 1
        || cache.items.len() > MAX_CACHED_SYMBOLS
        || cache
            .retry_after
            .as_deref()
            .is_some_and(|s| parse_time(s).is_none())
    {
        return Err(invalid());
    }
    for (symbol, item) in &cache.items {
        if normalize_symbol(symbol).ok().as_ref() != Some(symbol)
            || DateTime::parse_from_rfc3339(&item.checked_at)
                .map_or(true, |at| at > Utc::now() + chrono::Duration::minutes(5))
            || ![
                "available",
                "unsupported",
                "unauthorized",
                "rate_limited",
                "not_configured",
                "unavailable",
            ]
            .contains(&item.status.as_str())
            || item.error.as_ref().is_some_and(|s| s.len() > 2000)
        {
            return Err(invalid());
        }
        if let Some(observation) = &item.observation {
            if observation.symbol != *symbol
                || observation.currency != "BRL"
                || observation.source != "b3"
                || source_url(symbol).ok().as_ref() != Some(&observation.source_url)
                || observation.coverage_warning.len() > 1000
                || DateTime::parse_from_rfc3339(&observation.fetched_at)
                    .map_or(true, |at| at > Utc::now() + chrono::Duration::minutes(5))
                || observation.events.len() > MAX_ARCHIVED_EVENTS
            {
                return Err(invalid());
            }
            let mut ids = BTreeSet::new();
            for event in &observation.events {
                if event.symbol != *symbol
                    || event.source_url != observation.source_url
                    || event.event_id != event_id(event)
                    || !ids.insert(&event.event_id)
                    || decimal(&Value::String(event.per_share.clone()))
                        .ok()
                        .as_ref()
                        != Some(&event.per_share)
                    || !["income", "amortization", "jcp", "other"].contains(&event.kind.as_str())
                    || event.kind != income_kind(&event.label)
                    || event.verified != Some(true)
                    || event.date_precision
                        != if event.payment_date.is_some() && event.record_date.is_some() {
                            "exact"
                        } else {
                            "incomplete"
                        }
                    || !event.isin_code.as_deref().is_some_and(|isin| {
                        fund_code(symbol).is_ok_and(|code| normal_isin(code, isin))
                    })
                    || event.ex_date.is_some()
                    || event.label.is_empty()
                    || event.label.len() > 200
                    || event.remarks.as_ref().is_some_and(|s| s.len() > 4000)
                    || event.related_to.as_ref().is_some_and(|s| s.len() > 500)
                    || event.isin_code.as_ref().is_some_and(|s| s.len() > 50)
                    || event.last_observed_at.as_deref().is_some_and(|value| {
                        parse_time(value).is_none_or(|observed| {
                            observed > Utc::now() + chrono::Duration::minutes(5)
                                || parse_time(&observation.fetched_at)
                                    .is_none_or(|fetched| observed > fetched)
                        })
                    })
                {
                    return Err(invalid());
                }
                for value in [
                    &event.payment_date,
                    &event.record_date,
                    &event.ex_date,
                    &event.declared_date,
                ]
                .into_iter()
                .flatten()
                {
                    if date(Some(&Value::String(value.clone())))
                        .ok()
                        .flatten()
                        .as_ref()
                        != Some(value)
                    {
                        return Err(invalid());
                    }
                }
            }
        }
    }
    Ok(())
}
fn read_cache(path: &Path) -> Result<Cache, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Cache::default()),
        Err(_) => {
            return Err("Não foi possível ler o cache de proventos; arquivo preservado.".into())
        }
    };
    let mut bytes = Vec::new();
    file.take(CACHE_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Não foi possível ler o cache de proventos.".to_string())?;
    if bytes.len() > CACHE_LIMIT {
        return Err("Cache de proventos excedeu 16 MiB; arquivo preservado.".into());
    }
    let cache = serde_json::from_slice(&bytes)
        .map_err(|_| "Cache de proventos inválido; arquivo preservado.".to_string())?;
    validate(&cache)?;
    Ok(cache)
}
fn store_cache(path: &Path, cache: &Cache) -> Result<(), String> {
    validate(cache)?;
    let bytes = serde_json::to_vec(cache)
        .map_err(|_| "Não foi possível serializar o cache de proventos.".to_string())?;
    if bytes.len() > CACHE_LIMIT {
        return Err("Cache de proventos excedeu 16 MiB; arquivo preservado.".into());
    }
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|_| "Não foi possível criar a pasta de proventos.".to_string())?;
    }
    let temporary = path.with_extension(format!("{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|_| "Não foi possível criar o cache temporário de proventos.".to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "Não foi possível gravar o cache de proventos.".to_string())?;
        drop(file);
        fs::rename(&temporary, path).map_err(|_| {
            "Não foi possível substituir o cache de proventos; arquivo anterior preservado."
                .to_string()
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-20T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }
    fn event(rate: Value) -> Value {
        json!({
            "assetIssued":"BRMXRFCTF008","isinCode":"BRMXRFCTF008",
            "rate":rate,"label":"RENDIMENTO","lastDatePrior":"30/06/2026",
            "paymentDate":"14/07/2026","approvedOn":"30/06/2026",
            "relatedTo":"Junho-2026/2026","remarks":""
        })
    }
    fn fixture(events: Vec<Value>) -> DividendObservation {
        parse_b3_dividends(
            &serde_json::to_vec(&json!({"code":"MXRF","cashDividends":events})).unwrap(),
            "MXRF11",
            now(),
        )
        .unwrap()
    }

    fn august(rate: &str) -> Value {
        let mut event = event(json!(rate));
        event["paymentDate"] = json!("14/08/2026");
        event["lastDatePrior"] = json!("31/07/2026");
        event["approvedOn"] = json!("31/07/2026");
        event["relatedTo"] = json!("Julho-2026/2026");
        event
    }

    fn income_snapshot() -> ivestfii_core::PortfolioSnapshot {
        serde_json::from_value(json!({
            "funds":[{"id":"fund","ticker":"MXRF11","name":"Synthetic fund","sector":"", "entries":[{
                "id":"purchase","month":"2026-01","purchaseDate":"2026-01-02",
                "cotasAdded":"10","pricePerCota":"10","dividendPerCota":"0","dividendTotal":null,"notes":""
            }],"totalCotas":"10","totalInvested":"100","averagePrice":"10","dividendsReceived":"0","estimatedMonthlyIncome":"0"}],
            "totals":{"invested":"100","cotas":"10","dividendsReceived":"0","estimatedMonthlyIncome":"0"},
            "recordCount":1
        })).unwrap()
    }

    fn income(
        cache: &Cache,
        snapshot: &ivestfii_core::PortfolioSnapshot,
    ) -> ivestfii_core::income::MonthlyIncomeReport {
        ivestfii_core::income::calculate_monthly_income(
            snapshot,
            &income_events(&panel(&["MXRF11".into()], cache)),
            "2026-07",
            "2026-08",
            NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn rolling_window_archives_payments_so_accumulated_income_never_drops_at_rollover() {
        let mut cache = Cache::default();
        let mut old = fixture(vec![event(json!("0,1"))]);
        old.fetched_at = "2026-10-19T12:00:00Z".into();
        old.events[0].last_observed_at = None; // existing v1 cache
        replace_observation(&mut cache, "MXRF11", old, "2026-10-19T12:00:00Z".into());
        assert_eq!(income(&cache, &income_snapshot()).months[0].api_amount, "1");
        replace_observation(
            &mut cache,
            "MXRF11",
            fixture(vec![august("0,2")]),
            stamp(now()),
        );
        let saved = cache.items["MXRF11"].observation.as_ref().unwrap();
        assert_eq!(saved.events.len(), 2);
        assert_eq!(
            saved.events[0].last_observed_at.as_deref(),
            Some("2026-10-19T12:00:00Z")
        );
        assert_eq!(
            saved.events[1].last_observed_at.as_deref(),
            Some("2026-10-20T12:00:00Z")
        );
        let snapshot = income_snapshot();
        let report = income(&cache, &snapshot);
        assert_eq!(
            report
                .months
                .iter()
                .map(|month| Decimal::from_str_exact(&month.api_amount).unwrap())
                .sum::<Decimal>(),
            Decimal::from(3)
        );
        assert_eq!(report.months[0].api_amount, "1");
        assert_eq!(report.months[1].api_amount, "2");
        assert!(cache.items["MXRF11"].error.is_none());
        // An actual receipt replaces only its own archived fund/month.
        let mut with_receipt = snapshot.clone();
        let mut receipt = with_receipt.funds[0].entries[0].clone();
        receipt.id = "receipt".into();
        receipt.month = "2026-07".into();
        receipt.purchase_date = None;
        receipt.cotas_added = "0".into();
        receipt.dividend_total = Some("1.25".into());
        with_receipt.funds[0].entries.push(receipt);
        let reconciled = income(&cache, &with_receipt);
        assert_eq!(reconciled.months[0].recorded_amount, "1.25");
        assert_eq!(reconciled.months[0].api_amount, "0");
        assert_eq!(reconciled.months[1].api_amount, "2");
        assert_eq!(with_receipt.funds[0].entries.len(), 2);
        assert_eq!(snapshot.funds[0].entries.len(), 1);
    }

    #[test]
    fn revised_rate_and_dates_replace_recent_window_even_without_stable_references() {
        let mut cache = Cache::default();
        let mut old = event(json!("0,1"));
        old["approvedOn"] = Value::Null;
        old["relatedTo"] = Value::Null;
        // A preceding event locates both date versions inside the current
        // authoritative window, rather than across its uncertain boundary.
        let mut anchor = event(json!("0,01"));
        anchor["paymentDate"] = json!("15/06/2026");
        anchor["lastDatePrior"] = json!("31/05/2026");
        anchor["approvedOn"] = json!("31/05/2026");
        anchor["relatedTo"] = json!("Maio-2026/2026");
        replace_observation(
            &mut cache,
            "MXRF11",
            fixture(vec![anchor.clone(), old.clone()]),
            stamp(now()),
        );
        old["rate"] = json!("0,2");
        old["paymentDate"] = json!("15/07/2026");
        old["lastDatePrior"] = json!("29/06/2026");
        replace_observation(
            &mut cache,
            "MXRF11",
            fixture(vec![anchor, old]),
            stamp(now()),
        );
        let saved = cache.items["MXRF11"].observation.as_ref().unwrap();
        assert_eq!(saved.events.len(), 2);
        assert_eq!(saved.events[1].per_share, "0.2");
        assert_eq!(saved.events[1].payment_date.as_deref(), Some("2026-07-15"));
        assert_eq!(income(&cache, &income_snapshot()).months[0].api_amount, "2");
    }

    #[test]
    fn rolling_boundary_can_split_a_month_without_erasing_an_earlier_payment() {
        let mut first = event(json!("0,1"));
        first["paymentDate"] = json!("15/06/2026");
        first["lastDatePrior"] = json!("31/05/2026");
        first["approvedOn"] = json!("31/05/2026");
        first["relatedTo"] = json!("Maio-2026/2026");
        let mut second = event(json!("0,1"));
        second["paymentDate"] = json!("30/06/2026");
        second["lastDatePrior"] = json!("15/06/2026");
        second["approvedOn"] = json!("15/06/2026");
        second["relatedTo"] = json!("Junho-parcial-2026/2026");
        let mut cache = Cache::default();
        replace_observation(
            &mut cache,
            "MXRF11",
            fixture(vec![first, second.clone()]),
            stamp(now()),
        );
        replace_observation(
            &mut cache,
            "MXRF11",
            fixture(vec![second, event(json!("0,1"))]),
            stamp(now()),
        );
        let observation = cache.items["MXRF11"].observation.as_ref().unwrap();
        assert_eq!(observation.events.len(), 3);
        assert!(observation
            .events
            .iter()
            .any(|event| event.payment_date.as_deref() == Some("2026-06-15")));
        let report = ivestfii_core::income::calculate_monthly_income(
            &income_snapshot(),
            &income_events(&panel(&["MXRF11".into()], &cache)),
            "2026-06",
            "2026-07",
            NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
        )
        .unwrap();
        assert_eq!(report.months[0].api_amount, "2");
        assert_eq!(report.months[1].api_amount, "1");
        assert!(cache.items["MXRF11"].error.is_none());
    }

    #[test]
    fn stable_reference_reconciles_a_date_revision_across_the_archive_boundary() {
        let mut cache = Cache::default();
        replace_observation(
            &mut cache,
            "MXRF11",
            fixture(vec![event(json!("0,1"))]),
            stamp(now()),
        );
        let mut corrected = event(json!("0,15"));
        corrected["paymentDate"] = json!("03/08/2026");
        corrected["lastDatePrior"] = json!("29/06/2026");
        replace_observation(
            &mut cache,
            "MXRF11",
            fixture(vec![corrected, august("0,2")]),
            stamp(now()),
        );
        let saved = cache.items["MXRF11"].observation.as_ref().unwrap();
        assert_eq!(saved.events.len(), 2);
        assert!(saved.events.iter().all(|event| event
            .payment_date
            .as_deref()
            .unwrap()
            .starts_with("2026-08")));
        assert_eq!(
            income(&cache, &income_snapshot()).months[1].api_amount,
            "3.5"
        );
        assert!(cache.items["MXRF11"].error.is_none());
    }

    #[test]
    fn ambiguous_reference_quarantines_versions_without_freezing_other_events() {
        let mut cache = Cache::default();
        let mut other_old = event(json!("0,02"));
        other_old["paymentDate"] = json!("28/07/2026");
        replace_observation(
            &mut cache,
            "MXRF11",
            fixture(vec![event(json!("0,1")), other_old]),
            stamp(now()),
        );
        let mut revised = event(json!("0,15"));
        revised["paymentDate"] = json!("03/08/2026");
        let fresh = fixture(vec![revised, august("0,2")]);
        replace_observation(&mut cache, "MXRF11", fresh.clone(), stamp(now()));
        let saved = cache.items["MXRF11"].observation.as_ref().unwrap();
        assert_eq!(saved.events.len(), 4);
        assert_eq!(
            saved
                .events
                .iter()
                .filter(|event| event.reconciliation_pending)
                .count(),
            3
        );
        assert!(saved
            .events
            .iter()
            .any(|event| event.per_share == "0.2" && !event.reconciliation_pending));
        assert!(cache.items["MXRF11"]
            .error
            .as_deref()
            .unwrap()
            .contains("revisão"));
        // The uncertain event keeps its payment month incomplete, including
        // when another fully identified payment exists in that same month.
        let report = income(&cache, &income_snapshot());
        assert_eq!(report.months[0].api_amount, "0");
        assert_eq!(report.months[1].api_amount, "0");
        assert_eq!(report.months[1].funds[0].status, "missing");
        // Re-reading the identical fresh window cannot silently clear a flag.
        replace_observation(&mut cache, "MXRF11", fresh, stamp(now()));
        assert_eq!(
            cache.items["MXRF11"]
                .observation
                .as_ref()
                .unwrap()
                .events
                .iter()
                .filter(|event| event.reconciliation_pending)
                .count(),
            3
        );
    }

    #[test]
    fn missing_boundary_identity_is_pending_instead_of_duplicate_income() {
        let mut old = event(json!("0,1"));
        old["approvedOn"] = Value::Null;
        old["relatedTo"] = Value::Null;
        let mut revised = old.clone();
        revised["paymentDate"] = json!("03/08/2026");
        let mut cache = Cache::default();
        replace_observation(&mut cache, "MXRF11", fixture(vec![old]), stamp(now()));
        replace_observation(&mut cache, "MXRF11", fixture(vec![revised]), stamp(now()));
        assert!(cache.items["MXRF11"]
            .observation
            .as_ref()
            .unwrap()
            .events
            .iter()
            .all(|event| event.reconciliation_pending));
        assert_eq!(income(&cache, &income_snapshot()).months[1].api_amount, "0");
        assert!(cache.items["MXRF11"].error.is_some());
    }

    #[test]
    fn empty_or_undated_response_preserves_known_history_with_original_observation_time() {
        let mut cache = Cache::default();
        let old = fixture(vec![event(json!("0,1"))]);
        replace_observation(&mut cache, "MXRF11", old.clone(), stamp(now()));
        replace_observation(&mut cache, "MXRF11", fixture(vec![]), stamp(now()));
        assert_eq!(cache.items["MXRF11"].observation.as_ref(), Some(&old));
        assert!(cache.items["MXRF11"]
            .error
            .as_deref()
            .unwrap()
            .contains("lista vazia"));
        let mut undated = event(json!("0,15"));
        undated["paymentDate"] = Value::Null;
        replace_observation(&mut cache, "MXRF11", fixture(vec![undated]), stamp(now()));
        assert_eq!(cache.items["MXRF11"].observation.as_ref(), Some(&old));
        assert!(cache.items["MXRF11"]
            .error
            .as_deref()
            .unwrap()
            .contains("datas de pagamento"));
    }

    #[test]
    fn legacy_version_one_cache_reopens_and_archive_metadata_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dividends.json");
        let mut cache = Cache::default();
        let mut old = fixture(vec![event(json!("0,1"))]);
        old.events[0].last_observed_at = None;
        replace_observation(&mut cache, "MXRF11", old, stamp(now()));
        store_cache(&path, &cache).unwrap();
        let legacy = fs::read_to_string(&path).unwrap();
        assert!(!legacy.contains("lastObservedAt"));
        assert!(!legacy.contains("reconciliationPending"));
        let mut reopened = read_cache(&path).unwrap();
        replace_observation(
            &mut reopened,
            "MXRF11",
            fixture(vec![august("0,2")]),
            stamp(now()),
        );
        store_cache(&path, &reopened).unwrap();
        let final_cache = read_cache(&path).unwrap();
        assert_eq!(final_cache.version, 1);
        assert_eq!(
            final_cache.items["MXRF11"]
                .observation
                .as_ref()
                .unwrap()
                .events
                .len(),
            2
        );
        assert_eq!(
            income(&final_cache, &income_snapshot()).months[0].api_amount,
            "1"
        );
        assert_eq!(
            income(&final_cache, &income_snapshot()).months[1].api_amount,
            "2"
        );
        assert!(final_cache.items["MXRF11"]
            .observation
            .as_ref()
            .unwrap()
            .events
            .iter()
            .all(|event| event.last_observed_at.is_some()));
    }

    #[test]
    fn archive_size_limit_preserves_previous_file_instead_of_truncating_payments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dividends.json");
        let mut cache = Cache::default();
        replace_observation(
            &mut cache,
            "MXRF11",
            fixture(vec![event(json!("0,1"))]),
            stamp(now()),
        );
        store_cache(&path, &cache).unwrap();
        let original = fs::read(&path).unwrap();
        let observation = cache
            .items
            .get_mut("MXRF11")
            .unwrap()
            .observation
            .as_mut()
            .unwrap();
        let template = observation.events[0].clone();
        observation.events = (0..4000)
            .map(|index| {
                let mut event = template.clone();
                event.related_to = Some(format!("Synthetic reference {index}"));
                event.remarks = Some("x".repeat(4000));
                event.event_id = event_id(&event);
                event
            })
            .collect();
        assert!(validate(&cache).is_ok());
        assert!(store_cache(&path, &cache).unwrap_err().contains("16 MiB"));
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(
            read_cache(&path).unwrap().items["MXRF11"]
                .observation
                .as_ref()
                .unwrap()
                .events
                .len(),
            1
        );
    }

    #[test]
    fn archived_provenance_cannot_be_invalid_or_newer_than_its_source_observation() {
        let mut cache = Cache::default();
        replace_observation(
            &mut cache,
            "MXRF11",
            fixture(vec![event(json!("0,1"))]),
            stamp(now()),
        );
        for value in ["invalid", "2026-10-20T12:00:01Z"] {
            cache
                .items
                .get_mut("MXRF11")
                .unwrap()
                .observation
                .as_mut()
                .unwrap()
                .events[0]
                .last_observed_at = Some(value.into());
            assert!(validate(&cache).is_err());
        }
    }

    #[test]
    fn exact_brazilian_decimal_and_calendar_dates_never_pass_through_float() {
        let result = fixture(vec![event(json!("0,10000000000000000001"))]);
        let event = &result.events[0];
        assert_eq!(event.per_share, "0.10000000000000000001");
        assert_eq!(event.payment_date.as_deref(), Some("2026-07-14"));
        assert_eq!(event.record_date.as_deref(), Some("2026-06-30"));
        assert_eq!(event.declared_date.as_deref(), Some("2026-06-30"));
        assert_eq!(event.date_precision, "exact");
        assert_eq!(result.source, "b3");
        assert_eq!(
            event.source_url,
            "https://sistemaswebb3-listados.b3.com.br/fundsListedPage/events-main/MXRF"
        );
    }

    #[test]
    fn ordinary_isin_excludes_subscription_receipts_from_monthly_income() {
        let ordinary = event(json!("0,1"));
        let mut receipt = ordinary.clone();
        receipt["assetIssued"] = json!("BRMXRFR24M16");
        receipt["isinCode"] = json!("BRMXRFR24M16");
        receipt["rate"] = json!("50,0");
        let parsed = fixture(vec![ordinary, receipt]);
        assert_eq!(parsed.events.len(), 1);
        assert_eq!(parsed.events[0].isin_code.as_deref(), Some("BRMXRFCTF008"));
        assert_eq!(parsed.events[0].per_share, "0.1");
    }

    #[test]
    fn multiple_events_per_month_are_preserved_and_duplicates_are_stable() {
        let first = event(json!("0,1"));
        let mut second = event(json!("0,02"));
        second["paymentDate"] = json!("28/07/2026");
        let parsed = fixture(vec![first.clone(), first.clone(), second]);
        assert_eq!(parsed.events.len(), 2);
        assert_eq!(
            parsed.events[0].event_id,
            fixture(vec![first.clone()]).events[0].event_id
        );
        let mut corrected = first.clone();
        corrected["rate"] = json!("0,2");
        assert_eq!(
            fixture(vec![corrected.clone()]).events[0].event_id,
            parsed.events[0].event_id
        );
        let conflicting = json!({"code":"MXRF","cashDividends":[first,corrected]});
        assert!(
            parse_b3_dividends(&serde_json::to_vec(&conflicting).unwrap(), "MXRF11", now())
                .is_err()
        );
    }

    #[test]
    fn source_revision_replaces_old_payment_dates_without_duplicate_event() {
        let old = fixture(vec![event(json!("0,1"))]);
        let mut changed = event(json!("0,1"));
        changed["paymentDate"] = json!("15/07/2026");
        let corrected = fixture(vec![changed]);
        assert_ne!(old.events[0].event_id, corrected.events[0].event_id);
        let mut cache = Cache::default();
        replace_observation(&mut cache, "MXRF11", old, stamp(now()));
        replace_observation(&mut cache, "MXRF11", corrected, stamp(now()));
        let result = &cache.items["MXRF11"].observation.as_ref().unwrap().events;
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].payment_date.as_deref(), Some("2026-07-15"));
    }

    #[test]
    fn cache_identity_precision_and_kind_are_validated_and_incomplete_income_reaches_core() {
        let mut incomplete = event(json!("0,1"));
        incomplete["lastDatePrior"] = Value::Null;
        let original = fixture(vec![incomplete]);
        let mut cache = Cache::default();
        replace_observation(&mut cache, "MXRF11", original.clone(), stamp(now()));
        assert!(validate(&cache).is_ok());
        let input = income_events(&panel(&["MXRF11".into()], &cache));
        assert_eq!(input.len(), 1);
        assert_eq!(input[0].record_date, "");
        assert_eq!(input[0].payment_date, "2026-07-14");
        for field in ["precision", "isin", "kind"] {
            let mut corrupted = original.clone();
            let event = &mut corrupted.events[0];
            match field {
                "precision" => event.date_precision = "exact".into(),
                "isin" => {
                    event.isin_code = Some("BRMXRFR24M16".into());
                    event.event_id = event_id(event);
                }
                _ => event.kind = "amortization".into(),
            }
            replace_observation(&mut cache, "MXRF11", corrupted, stamp(now()));
            assert!(validate(&cache).is_err(), "{field}");
        }
    }

    #[test]
    fn future_payments_remain_raw_and_amortization_is_not_income() {
        let mut future = event(json!("0,1"));
        future["paymentDate"] = json!("15/01/2027");
        let mut refund = event(json!("0,5"));
        refund["label"] = json!("AMORTIZAÇÃO");
        let items = fixture(vec![future, refund]).events;
        assert!(items
            .iter()
            .any(|x| x.payment_date.as_deref() == Some("2027-01-15")));
        assert!(items.iter().any(|x| x.kind == "amortization"));
    }

    #[test]
    fn missing_date_is_not_substituted_by_approval_or_record_date() {
        let mut incomplete = event(json!("0,1"));
        incomplete["paymentDate"] = Value::Null;
        let event = fixture(vec![incomplete]).events.remove(0);
        assert_eq!(event.payment_date, None);
        assert_eq!(event.date_precision, "incomplete");
    }

    #[test]
    fn missing_coverage_and_unknown_identity_are_not_zero_payment() {
        for payload in [
            json!({}),
            json!({"code":"VGHF","cashDividends":[]}),
            json!({"code":"MXRF"}),
        ] {
            assert!(
                parse_b3_dividends(&serde_json::to_vec(&payload).unwrap(), "MXRF11", now())
                    .is_err()
            );
        }
        assert!(fixture(vec![]).events.is_empty());
        let mut wrong = event(json!("0,1"));
        wrong["isinCode"] = json!("BRVGHFCTF005");
        wrong["assetIssued"] = json!("BRVGHFCTF005");
        assert!(parse_b3_dividends(
            &serde_json::to_vec(&json!({"code":"MXRF","cashDividends":[wrong]})).unwrap(),
            "MXRF11",
            now()
        )
        .is_err());
        let mut ambiguous = event(json!("0,1"));
        ambiguous["isinCode"] = json!("BRMXRFCTF009");
        ambiguous["assetIssued"] = json!("BRMXRFCTF009");
        assert!(parse_b3_dividends(
            &serde_json::to_vec(
                &json!({"code":"MXRF","cashDividends":[event(json!("0,1")),ambiguous]})
            )
            .unwrap(),
            "MXRF11",
            now()
        )
        .is_err());
    }

    #[test]
    fn invalid_calendar_dates_negative_rates_and_inconsistent_assets_fail_closed() {
        let mut invalid = event(json!("0,1"));
        invalid["paymentDate"] = json!("30/02/2026");
        let mut negative = event(json!("-0,1"));
        negative["paymentDate"] = json!("14/07/2026");
        let mut inconsistent = event(json!("0,1"));
        inconsistent["assetIssued"] = json!("BRMXRFR24M16");
        for raw in [invalid, negative, inconsistent] {
            assert!(parse_b3_dividends(
                &serde_json::to_vec(&json!({"code":"MXRF","cashDividends":[raw]})).unwrap(),
                "MXRF11",
                now()
            )
            .is_err());
        }
    }

    #[tokio::test]
    async fn fresh_or_unsupported_cache_never_reserves_http_and_corruption_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dividends.json");
        let service = DividendService::new(path.clone()).unwrap();
        let mut cache = Cache::default();
        let mut observation = fixture(vec![event(json!("0,1"))]);
        observation.fetched_at = stamp(Utc::now());
        cache.items.insert(
            "MXRF11".into(),
            SavedItem {
                observation: Some(observation),
                checked_at: stamp(Utc::now()),
                status: "available".into(),
                error: None,
            },
        );
        cache.items.insert(
            "VGHF11".into(),
            SavedItem {
                observation: None,
                checked_at: stamp(Utc::now()),
                status: "unsupported".into(),
                error: Some("Histórico indisponível".into()),
            },
        );
        store_cache(&path, &cache).unwrap();
        let bytes = fs::read(&path).unwrap();
        let result = service
            .refresh(&["MXRF11".into(), "VGHF11".into()])
            .await
            .unwrap();
        assert_eq!(result.items[0].status, "cached");
        assert_eq!(result.items[1].status, "unsupported");
        assert!(!path.with_extension("refresh-policy.json").exists());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::write(&path, b"invalid original").unwrap();
        assert!(service.refresh(&["MXRF11".into()]).await.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"invalid original");
    }

    #[tokio::test]
    async fn saved_rate_limit_blocks_all_symbols_without_creating_reservations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dividends.json");
        let cache = Cache {
            retry_after: Some(stamp(Utc::now() + chrono::Duration::hours(1))),
            ..Cache::default()
        };
        store_cache(&path, &cache).unwrap();
        let result = DividendService::new(path.clone())
            .unwrap()
            .refresh(&["MXRF11".into(), "VGHF11".into()])
            .await
            .unwrap();
        assert!(result
            .items
            .iter()
            .all(|x| x.status == "rate_limited" && x.observation.is_none()));
        assert!(!path.with_extension("refresh-policy.json").exists());
        assert_eq!(
            http_failure(429, Some("3600"), now()).retry_after,
            Some(now() + chrono::Duration::hours(1))
        );
    }

    #[tokio::test]
    async fn b3_daily_budget_is_conservative_and_does_not_consume_brapi_budget() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dividends.json");
        let mut gate = RefreshGate::acquire(&path).await.unwrap();
        let requested_at = Utc::now();
        for n in 0..B3_DAILY_LIMIT {
            assert!(gate
                .reserve(
                    &format!("fixture:{n}"),
                    None,
                    TTL,
                    requested_at,
                    Some(B3_DAILY_LIMIT)
                )
                .unwrap()
                .is_none());
        }
        drop(gate);
        let service = DividendService::new(path.clone()).unwrap();
        let result = service.refresh(&["MXRF11".into()]).await.unwrap();
        assert_eq!(result.items[0].status, "rate_limited");
        assert!(!dir.path().join("quotes.refresh-policy.json").exists());
    }

    #[test]
    fn local_reads_are_read_only_and_forged_source_cannot_be_cached() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("absent");
        let service = DividendService::new(folder.join("dividends.json")).unwrap();
        assert!(service.read_cache(&["MXRF11".into()]).unwrap().items[0]
            .observation
            .is_none());
        assert!(!folder.exists());
        let mut cache = Cache::default();
        let mut observation = fixture(vec![event(json!("0,1"))]);
        observation.source_url = "https://example.org".into();
        cache.items.insert(
            "MXRF11".into(),
            SavedItem {
                observation: Some(observation),
                checked_at: stamp(now()),
                status: "available".into(),
                error: None,
            },
        );
        assert!(validate(&cache).is_err());
    }
}
