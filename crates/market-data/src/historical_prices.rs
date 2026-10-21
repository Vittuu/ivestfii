//! Real monthly FII market closes, separate from portfolio records and income.
//!
//! Dados B3's documented, open `/fiis/{ticker}/desempenho` endpoint returns the
//! first and last actual trading dates and unadjusted COTAHIST closes for a date
//! window. We retain only the final traded price, never the calculated return,
//! adjusted-price field, NAV or P/VP. No quantity, amount or purchase date leaves
//! this service. App, MCP and manual refreshes share this public cache and budget.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::{DateTime, Datelike, Days, NaiveDate, SecondsFormat, Utc};
use reqwest::{redirect::Policy, Client};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    brasilia_date,
    quotes::normalize_symbol,
    refresh_policy::{fresh, RefreshGate, RefreshOutcome},
};

const ENDPOINT: &str = "https://dadosb3.com/fiis";
const SOURCE: &str = "Dados B3 · COTAHIST";
const DAILY_LIMIT: u32 = 160;
const BATCH_LIMIT: usize = 48;
const RESPONSE_LIMIT: usize = 128 * 1024;
const CACHE_LIMIT: usize = 8 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;
const MAX_REQUESTS: usize = 2_000;
const CLOSED_TTL: i64 = 7 * 24 * 3600;
const CURRENT_TTL: i64 = 24 * 3600;
const FAILURE_TTL: i64 = 300;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoricalPriceRequest {
    pub symbol: String,
    pub month: String,
    /// Optional real valuation cutoff within this month, for example when the
    /// fund NAV ends before today's market quote. Never transmitted as a user
    /// transaction date: this remains a public calendar market-data window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub through_date: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoricalPrice {
    pub symbol: String,
    pub trade_date: String,
    /// Positive, exact decimal BRL, with neither distribution nor split adjustment.
    pub raw_close: String,
    pub currency: String,
    pub source: String,
    pub source_url: String,
    pub fetched_at: String,
    /// Dados B3 detects large price jumps but has no complete FII corporate-action
    /// register. A plausible raw close is not proof that old quantities need no
    /// split conversion. Consumers must not interpret this as adjusted history.
    pub corporate_actions_checked: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalPricePanel {
    pub observations: Vec<HistoricalPrice>,
    pub issues: Vec<String>,
    pub pending_requests: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SavedItem {
    request: HistoricalPriceRequest,
    requested_through: String,
    observation: Option<HistoricalPrice>,
    checked_at: String,
    available: bool,
    error: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Cache {
    version: u32,
    items: BTreeMap<String, SavedItem>,
    retry_after: Option<String>,
    /// The source's reset timezone is not guaranteed. In addition to the BRT
    /// daily gate, retain attempts across midnight in a rolling 24-hour window.
    #[serde(default)]
    recent_attempts: Vec<String>,
}

#[derive(Clone)]
pub struct HistoricalPriceService {
    client: Client,
    cache_path: PathBuf,
}

impl HistoricalPriceService {
    pub fn new(cache_path: PathBuf) -> Result<Self, String> {
        if cache_path.file_name().is_none() {
            return Err("Informe um arquivo para o histórico público de cotações.".into());
        }
        let client = Client::builder()
            .https_only(true)
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(12))
            .user_agent("IvestFII/0.1 (public historical FII market closes)")
            .build()
            .map_err(|_| "Não foi possível preparar o histórico de cotações.".to_string())?;
        Ok(Self { client, cache_path })
    }

    /// Local only. Reading never creates a file or consumes a public-source call.
    pub fn read_cache(
        &self,
        requests: &[HistoricalPriceRequest],
    ) -> Result<HistoricalPricePanel, String> {
        let now = Utc::now();
        let requests = normalize_requests(requests, brasilia_date(now))?;
        Ok(panel(&requests, &read_cache(&self.cache_path, now)?, now))
    }

    /// At most 48 sequential public requests per call, newest months first.
    /// Repeat in background while pending_requests > 0, within the shared daily
    /// budget. Partial batches are persisted after each successful observation.
    pub async fn refresh(
        &self,
        requests: &[HistoricalPriceRequest],
    ) -> Result<HistoricalPricePanel, String> {
        let now = Utc::now();
        let requests = normalize_requests(requests, brasilia_date(now))?;
        if requests.is_empty() {
            return Ok(HistoricalPricePanel::default());
        }
        let mut gate = RefreshGate::acquire(&self.cache_path).await?;
        let mut cached = read_cache(&self.cache_path, Utc::now())?;
        if cached
            .retry_after
            .as_deref()
            .and_then(parse_time)
            .is_some_and(|at| at > Utc::now())
        {
            let mut result = panel(&requests, &cached, Utc::now());
            result.issues.push("O histórico de cotações está aguardando o intervalo pedido pela fonte. Os preços salvos foram preservados.".into());
            return Ok(result);
        }
        let mut requested = 0;
        let mut deferred = None;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
        for request in &requests {
            let now = Utc::now();
            let key = key(request);
            let saved = cached.items.get(&key);
            if saved.is_some_and(|item| item_fresh(item, now)) {
                continue;
            }
            if requested >= BATCH_LIMIT {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                deferred = Some("O lote de histórico atingiu 90 segundos; os meses já consultados foram salvos para continuar em segundo plano.".into());
                break;
            }
            let previous = saved.and_then(|item| item.observation.clone());
            if let Some(message) = reserve_attempt(
                &mut cached,
                &self.cache_path,
                &mut gate,
                &key,
                ttl(request, brasilia_date(now)),
                now,
            )? {
                deferred = Some(message.replace(
                    "de cotações deste dia",
                    "do histórico de cotações deste dia",
                ));
                continue;
            }
            if !cached.items.contains_key(&key) && cached.items.len() >= MAX_ENTRIES {
                return Err("O histórico de cotações atingiu o limite local. Os dados anteriores foram preservados.".into());
            }
            requested += 1;
            let (start, end) = request_window(request, brasilia_date(now))?;
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let fetched = tokio::time::timeout(remaining, self.fetch(request, start, end))
                .await.unwrap_or_else(|_| Err(Failure::temporary("O lote de histórico atingiu o prazo; os preços anteriores foram preservados.")));
            let stop = fetched.as_ref().err().is_some_and(|error| error.stop);
            let (observation, available, error) = match fetched {
                Ok(observation) => (Some(observation), true, None),
                Err(failure) => {
                    cached.retry_after = failure.retry_after;
                    (previous, false, Some(failure.message))
                }
            };
            cached.items.insert(
                key,
                SavedItem {
                    request: request.clone(),
                    requested_through: end.to_string(),
                    observation,
                    checked_at: stamp(Utc::now()),
                    available,
                    error,
                },
            );
            write_cache(&self.cache_path, &cached)?;
            if stop {
                break;
            }
        }
        let mut result = panel(&requests, &cached, Utc::now());
        if let Some(message) = deferred {
            result.issues.push(message);
        }
        Ok(result)
    }

    pub async fn refresh_if_due(
        &self,
        requests: &[HistoricalPriceRequest],
    ) -> RefreshOutcome<HistoricalPricePanel> {
        let before = self.read_cache(requests).ok();
        match self.refresh(requests).await {
            Ok(data) if before.as_ref() == Some(&data) && data.issues.is_empty() => {
                RefreshOutcome::cached(data)
            }
            Ok(data) if data.issues.is_empty() => RefreshOutcome::updated(data),
            Ok(data) => RefreshOutcome {
                status: if data.observations.is_empty() {
                    "unavailable"
                } else {
                    "cached"
                }
                .into(),
                message: Some(data.issues.join(" ")),
                data: Some(data),
            },
            Err(error) => RefreshOutcome::failed(self.read_cache(requests).ok(), error),
        }
    }

    async fn fetch(
        &self,
        request: &HistoricalPriceRequest,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<HistoricalPrice, Failure> {
        let url = endpoint(&request.symbol, start, end);
        let mut response = self.client.get(&url)
            .header(reqwest::header::ACCEPT, "application/json")
            .send().await.map_err(|_| Failure::temporary("Não foi possível consultar o histórico de cotações. Os dados salvos foram preservados."))?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            return Err(if status == 429 {
                let now = Utc::now();
                let until = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|x| x.to_str().ok())
                    .and_then(|s| s.parse::<i64>().ok())
                    .filter(|seconds| *seconds > 0 && *seconds <= 7 * 24 * 3600)
                    .map(|seconds| now + chrono::Duration::seconds(seconds))
                    .unwrap_or(now + chrono::Duration::hours(1));
                Failure { message: "A fonte limitou as consultas ao histórico de cotações. Os preços salvos foram preservados.".into(), stop: true, retry_after: Some(stamp(until)) }
            } else if status == 401 || status == 403 {
                Failure { message: "A fonte não autorizou este histórico de cotações. Nenhuma alternativa de acesso foi tentada.".into(), stop: true, retry_after: Some(stamp(Utc::now() + chrono::Duration::hours(24))) }
            } else if status >= 500 {
                Failure::temporary("A fonte está temporariamente indisponível para o histórico de cotações. Os preços salvos foram preservados.")
            } else {
                Failure::new(&format!(
                    "Histórico de cotações indisponível para {} (HTTP {status}).",
                    request.symbol
                ))
            });
        }
        if response
            .content_length()
            .is_some_and(|len| len > RESPONSE_LIMIT as u64)
        {
            return Err(Failure::new(
                "A resposta do histórico de cotações excedeu 128 KiB.",
            ));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| Failure::new("A resposta do histórico de cotações foi interrompida."))?
        {
            if chunk.len() > RESPONSE_LIMIT.saturating_sub(bytes.len()) {
                return Err(Failure::new(
                    "A resposta do histórico de cotações excedeu 128 KiB.",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        parse_response(request, start, end, &bytes, Utc::now())
            .map_err(|message| Failure::new(&message))
    }
}

struct Failure {
    message: String,
    stop: bool,
    retry_after: Option<String>,
}

impl Failure {
    fn new(message: &str) -> Self {
        Self {
            message: message.into(),
            stop: false,
            retry_after: None,
        }
    }
    fn temporary(message: &str) -> Self {
        Self {
            message: message.into(),
            stop: true,
            retry_after: Some(stamp(Utc::now() + chrono::Duration::minutes(5))),
        }
    }
}

fn parse_time(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn date(text: &str) -> Result<NaiveDate, String> {
    let result = NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .map_err(|_| "A data do histórico de cotações é inválida.".to_string())?;
    if result.to_string() != text {
        return Err("A data do histórico de cotações é ambígua.".into());
    }
    Ok(result)
}

fn month_start(month: &str) -> Result<NaiveDate, String> {
    let result = date(&format!("{month}-01"))?;
    if month.len() != 7 || result.year() < 1986 {
        return Err("Informe um mês válido para o histórico de cotações.".into());
    }
    Ok(result)
}

fn request_window(
    request: &HistoricalPriceRequest,
    today: NaiveDate,
) -> Result<(NaiveDate, NaiveDate), String> {
    let start = month_start(&request.month)?;
    if start > today {
        return Err("O histórico de cotações não aceita meses futuros.".into());
    }
    let next = if start.month() == 12 {
        NaiveDate::from_ymd_opt(start.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(start.year(), start.month() + 1, 1)
    }
    .ok_or("O mês do histórico excedeu o calendário aceito.")?;
    let end = next
        .checked_sub_days(Days::new(1))
        .ok_or("Mês de histórico inválido.")?
        .min(today);
    let end = match request.through_date.as_deref() {
        Some(text) => {
            let cutoff = date(text)?;
            if cutoff < start || cutoff > end {
                return Err(
                    "A data final deve estar dentro do mês histórico e não pode ser futura.".into(),
                );
            }
            cutoff
        }
        None => end,
    };
    Ok((start, end))
}

fn normalize_requests(
    requests: &[HistoricalPriceRequest],
    today: NaiveDate,
) -> Result<Vec<HistoricalPriceRequest>, String> {
    if requests.len() > MAX_REQUESTS {
        return Err(
            "O histórico aceita até 2.000 combinações de fundo e mês por atualização.".into(),
        );
    }
    let mut unique = BTreeMap::new();
    for request in requests {
        let normalized = HistoricalPriceRequest {
            symbol: normalize_symbol(&request.symbol)?,
            month: request.month.clone(),
            through_date: request.through_date.clone(),
        };
        request_window(&normalized, today)?;
        unique.insert(key(&normalized), normalized);
    }
    let mut result: Vec<_> = unique.into_values().collect();
    result.sort_by(|a, b| b.month.cmp(&a.month).then_with(|| a.symbol.cmp(&b.symbol)));
    Ok(result)
}

fn key(request: &HistoricalPriceRequest) -> String {
    match &request.through_date {
        Some(cutoff) => format!("{}:{}:{cutoff}", request.symbol, request.month),
        None => format!("{}:{}", request.symbol, request.month),
    }
}

fn endpoint(symbol: &str, start: NaiveDate, end: NaiveDate) -> String {
    format!("{ENDPOINT}/{symbol}/desempenho?de={start}&ate={end}")
}

fn decimal(value: &Value) -> Result<String, String> {
    let text = match value {
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        _ => return Err("A fonte não retornou um fechamento histórico válido.".into()),
    };
    if text.is_empty() || text.len() > 40 || text.contains(['e', 'E']) {
        return Err("O fechamento histórico não é um decimal exato aceito.".into());
    }
    let parts: Vec<_> = text.split('.').collect();
    if parts.len() > 2
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()))
        || parts.get(1).is_some_and(|part| part.len() > 12)
    {
        return Err("O fechamento histórico não é um decimal exato aceito.".into());
    }
    let price: Decimal = text
        .parse()
        .map_err(|_| "Fechamento histórico inválido.".to_string())?;
    if price <= Decimal::ZERO || price > Decimal::from(10_000_000u32) {
        return Err("O fechamento histórico está fora dos limites aceitos.".into());
    }
    Ok(price.normalize().to_string())
}

/// Validates the documented source contract without making a request.
fn parse_response(
    request: &HistoricalPriceRequest,
    start: NaiveDate,
    end: NaiveDate,
    bytes: &[u8],
    fetched_at: DateTime<Utc>,
) -> Result<HistoricalPrice, String> {
    if bytes.len() > RESPONSE_LIMIT {
        return Err("A resposta do histórico de cotações excedeu 128 KiB.".into());
    }
    let body: Value = serde_json::from_slice(bytes)
        .map_err(|_| "A fonte retornou JSON histórico inválido.".to_string())?;
    if body.get("ticker_pedido").and_then(Value::as_str) != Some(&request.symbol)
        || body.get("tipo").and_then(Value::as_str) != Some("FII")
    {
        return Err("A resposta histórica não identifica o fundo solicitado.".into());
    }
    if body.get("confiavel").and_then(Value::as_bool) != Some(true)
        || !body
            .get("saltos_suspeitos")
            .and_then(Value::as_array)
            .is_some_and(|events| events.is_empty())
    {
        return Err(format!("{}: a fonte sinalizou possível desdobramento ou inconsistência no preço histórico. O mês aguarda conferência.", request.symbol));
    }
    let first = date(
        body.get("de")
            .and_then(Value::as_str)
            .ok_or("O histórico não informou a primeira data negociada.")?,
    )?;
    let last = date(
        body.get("ate")
            .and_then(Value::as_str)
            .ok_or("O histórico não informou a última data negociada.")?,
    )?;
    let count = body
        .get("pregoes")
        .and_then(Value::as_u64)
        .ok_or("O histórico não informou a quantidade de pregões.")?;
    if first < start
        || last < first
        || last > end
        || last > brasilia_date(fetched_at)
        || first.format("%Y-%m").to_string() != request.month
        || last.format("%Y-%m").to_string() != request.month
        || count == 0
        || count > 31
        || count as i64 > last.signed_duration_since(first).num_days() + 1
    {
        return Err(
            "As datas ou os pregões do histórico não correspondem ao mês solicitado.".into(),
        );
    }
    let premise = body
        .pointer("/premissas/preco")
        .and_then(Value::as_str)
        .ok_or("A fonte não informou se o preço histórico possui ajustes.")?;
    if !premise.contains("COTAHIST") || !premise.contains("SEM ajuste") {
        return Err("A fonte não confirmou o fechamento histórico sem ajustes.".into());
    }
    let raw_close = decimal(
        body.get("fechamento_final_negociado")
            .ok_or("O histórico não trouxe o fechamento original negociado.")?,
    )?;
    Ok(HistoricalPrice {
        symbol: request.symbol.clone(),
        trade_date: last.to_string(),
        raw_close,
        currency: "BRL".into(),
        source: SOURCE.into(),
        source_url: endpoint(&request.symbol, start, end),
        fetched_at: stamp(fetched_at),
        corporate_actions_checked: false,
    })
}

fn ttl(request: &HistoricalPriceRequest, today: NaiveDate) -> i64 {
    if request.month == today.format("%Y-%m").to_string() {
        CURRENT_TTL
    } else {
        CLOSED_TTL
    }
}

fn item_fresh(item: &SavedItem, now: DateTime<Utc>) -> bool {
    let lifetime = if item.available {
        ttl(&item.request, brasilia_date(now))
    } else {
        FAILURE_TTL
    };
    // A partial month saved yesterday is still useful, but a now-closed month
    // needs its final close instead of keeping the former partial value for 7d.
    let complete_window =
        request_window(&item.request, brasilia_date(now)).is_ok_and(|(_, end)| {
            item.requested_through == end.to_string()
                || item.request.month == brasilia_date(now).format("%Y-%m").to_string()
        });
    complete_window && fresh(Some(&item.checked_at), lifetime, now)
}

fn panel(
    requests: &[HistoricalPriceRequest],
    cached: &Cache,
    now: DateTime<Utc>,
) -> HistoricalPricePanel {
    let mut result = HistoricalPricePanel::default();
    for request in requests {
        let saved = cached.items.get(&key(request));
        if !saved.is_some_and(|item| item_fresh(item, now)) {
            result.pending_requests += 1;
        }
        if let Some(item) = saved {
            if let Some(observation) = &item.observation {
                result.observations.push(observation.clone());
            }
            if let Some(error) = &item.error {
                result
                    .issues
                    .push(format!("{} · {}: {error}", request.symbol, request.month));
            }
        }
    }
    result.observations.sort_by(|a, b| {
        a.trade_date
            .cmp(&b.trade_date)
            .then_with(|| a.symbol.cmp(&b.symbol))
    });
    result.issues.sort();
    result.issues.dedup();
    result
}

/// Runs under the same cross-process RefreshGate held for the HTTP call. The
/// rolling count is checked BEFORE the daily gate can reserve a request, and a
/// successful attempt is atomically persisted before entering any HTTP await.
fn reserve_attempt(
    cached: &mut Cache,
    cache_path: &Path,
    gate: &mut RefreshGate,
    key: &str,
    ttl: i64,
    now: DateTime<Utc>,
) -> Result<Option<String>, String> {
    let cutoff = now - chrono::Duration::hours(24);
    let in_window = cached
        .recent_attempts
        .iter()
        .filter(|value| parse_time(value).is_some_and(|at| at > cutoff))
        .count();
    if in_window >= DAILY_LIMIT as usize {
        return Ok(Some("Limite local de 160 consultas do histórico nas últimas 24 horas atingido. O orçamento não reinicia à meia-noite; os preços salvos foram preservados.".into()));
    }
    if let Some(message) = gate.reserve(key, None, ttl, now, Some(DAILY_LIMIT))? {
        return Ok(Some(message));
    }
    cached
        .recent_attempts
        .retain(|value| parse_time(value).is_some_and(|at| at > cutoff));
    cached.recent_attempts.push(stamp(now));
    write_cache(cache_path, cached)?;
    Ok(None)
}

fn read_cache(path: &Path, now: DateTime<Utc>) -> Result<Cache, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Cache {
                version: 1,
                ..Cache::default()
            })
        }
        Err(_) => return Err("Não foi possível ler o histórico de cotações salvo.".into()),
    };
    let mut bytes = Vec::new();
    file.take(CACHE_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Não foi possível ler o histórico de cotações salvo.".to_string())?;
    if bytes.len() > CACHE_LIMIT {
        return Err("Histórico de cotações excedeu 8 MiB; arquivo preservado.".into());
    }
    let cached: Cache = serde_json::from_slice(&bytes)
        .map_err(|_| "Histórico de cotações salvo inválido; arquivo preservado.".to_string())?;
    if cached.version != 1
        || cached.items.len() > MAX_ENTRIES
        || cached.recent_attempts.len() > DAILY_LIMIT as usize
        || cached
            .recent_attempts
            .iter()
            .any(|value| value.len() > 64 || parse_time(value).is_none_or(|at| at > now))
        || cached
            .retry_after
            .as_deref()
            .is_some_and(|at| parse_time(at).is_none())
    {
        return Err("Histórico de cotações contém versão ou limites inválidos.".into());
    }
    for (saved_key, item) in &cached.items {
        let normalized =
            normalize_requests(std::slice::from_ref(&item.request), brasilia_date(now))?;
        let (start, end) = request_window(&item.request, brasilia_date(now))?;
        let through = date(&item.requested_through)?;
        if normalized[0] != item.request
            || saved_key != &key(&item.request)
            || through < start
            || through > end
            || parse_time(&item.checked_at).is_none_or(|at| at > now + chrono::Duration::minutes(5))
            || item.error.as_ref().is_some_and(|error| error.len() > 2_000)
            || (item.available && (item.observation.is_none() || item.error.is_some()))
        {
            return Err(
                "O histórico de cotações contém uma entrada inválida; arquivo preservado.".into(),
            );
        }
        if let Some(observation) = &item.observation {
            let trade = date(&observation.trade_date)?;
            if observation.symbol != item.request.symbol
                || trade < start
                || trade > through
                || decimal(&Value::String(observation.raw_close.clone()))? != observation.raw_close
                || observation.currency != "BRL"
                || observation.source != SOURCE
                || parse_time(&observation.fetched_at).is_none()
                || observation.source_url
                    != endpoint(
                        &observation.symbol,
                        start,
                        request_window(
                            &item.request,
                            brasilia_date(
                                parse_time(&observation.fetched_at).expect("validated timestamp"),
                            ),
                        )?
                        .1,
                    )
                || observation.corporate_actions_checked
                || parse_time(&observation.fetched_at).is_none_or(|at| {
                    at > now + chrono::Duration::minutes(5) || trade > brasilia_date(at)
                })
            {
                return Err(
                    "O histórico de cotações contém um preço inválido; arquivo preservado.".into(),
                );
            }
        }
    }
    Ok(cached)
}

fn write_cache(path: &Path, cached: &Cache) -> Result<(), String> {
    let bytes = serde_json::to_vec(cached)
        .map_err(|_| "Não foi possível preparar o cache histórico.".to_string())?;
    if bytes.len() > CACHE_LIMIT {
        return Err("Histórico de cotações excedeu 8 MiB; arquivo anterior preservado.".into());
    }
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|_| {
            "Não foi possível preparar a pasta do histórico de cotações.".to_string()
        })?;
    }
    let temporary = path.with_extension(format!("{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
        return Err(
            "Não foi possível salvar o histórico; o arquivo anterior foi preservado.".into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-21T14:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }
    fn request(month: &str) -> HistoricalPriceRequest {
        HistoricalPriceRequest {
            symbol: "ABCD11".into(),
            month: month.into(),
            through_date: None,
        }
    }
    fn fixture() -> Value {
        json!({"ticker_pedido":"ABCD11","tipo":"FII","de":"2026-05-04","ate":"2026-05-29","pregoes":20,"confiavel":true,"saltos_suspeitos":[],"fechamento_final_negociado":9.98,"fechamento_final_ajustado":7.15,"premissas":{"preco":"fechamento do COTAHIST (B3), SEM ajuste: não há tabela de desdobramento/grupamento para FII"}})
    }
    fn parse(value: &Value) -> Result<HistoricalPrice, String> {
        parse_response(
            &request("2026-05"),
            date("2026-05-01").unwrap(),
            date("2026-05-31").unwrap(),
            &serde_json::to_vec(value).unwrap(),
            now(),
        )
    }
    fn saved() -> SavedItem {
        SavedItem {
            request: request("2026-05"),
            requested_through: "2026-05-31".into(),
            observation: Some(parse(&fixture()).unwrap()),
            checked_at: stamp(now()),
            available: true,
            error: None,
        }
    }

    #[test]
    fn takes_negotiated_close_and_actual_trading_date_not_adjusted_field_or_query_end() {
        let observation = parse(&fixture()).unwrap();
        assert_eq!(observation.raw_close, "9.98");
        assert_eq!(observation.trade_date, "2026-05-29");
        assert_eq!(observation.currency, "BRL");
        assert!(!observation.corporate_actions_checked);
        assert!(observation
            .source_url
            .ends_with("?de=2026-05-01&ate=2026-05-31"));
    }

    #[test]
    fn rejects_identity_mismatch_unknown_adjustment_and_suspicious_actions() {
        for (field, bad) in [
            ("ticker_pedido", json!("OTHER11")),
            ("tipo", json!("ETF")),
            ("confiavel", json!(false)),
            ("saltos_suspeitos", json!([{"factor":10}])),
        ] {
            let mut value = fixture();
            value[field] = bad;
            assert!(parse(&value).is_err(), "accepted {field}");
        }
        let mut value = fixture();
        value["premissas"]["preco"] = json!("fechamento ajustado");
        assert!(parse(&value).is_err());
        let mut value = fixture();
        value
            .as_object_mut()
            .unwrap()
            .remove("fechamento_final_negociado");
        assert!(parse(&value).is_err());
    }

    #[test]
    fn rejects_dates_outside_the_requested_month_or_impossible_trading_counts() {
        for (field, bad) in [
            ("de", json!("2026-04-30")),
            ("ate", json!("2026-06-01")),
            ("ate", json!("2026-05-01")),
            ("ate", json!("2026-5-29")),
            ("pregoes", json!(0)),
            ("pregoes", json!(31)),
        ] {
            let mut value = fixture();
            value[field] = bad;
            assert!(parse(&value).is_err(), "accepted {field}");
        }
    }

    #[test]
    fn raw_money_never_round_trips_through_binary_float_and_has_safe_bounds() {
        let mut value = fixture();
        value["fechamento_final_negociado"] = json!("9.980000000001");
        assert_eq!(parse(&value).unwrap().raw_close, "9.980000000001");
        for bad in [
            json!(0),
            json!(-1),
            json!(null),
            json!("1e2"),
            json!("9,98"),
            json!("1.0000000000001"),
            json!(10000001),
        ] {
            value["fechamento_final_negociado"] = bad;
            assert!(parse(&value).is_err());
        }
    }

    #[test]
    fn normalizes_and_deduplicates_public_month_requests_newest_first() {
        let requests = vec![
            request("2026-03"),
            HistoricalPriceRequest {
                symbol: " abcd11 ".into(),
                month: "2026-05".into(),
                through_date: None,
            },
            request("2026-05"),
        ];
        let result = normalize_requests(&requests, brasilia_date(now())).unwrap();
        assert_eq!(result, vec![request("2026-05"), request("2026-03")]);
        for invalid in ["2026-13", "2026-5", "2026-11", "1985-01"] {
            assert!(normalize_requests(&[request(invalid)], brasilia_date(now())).is_err());
        }
        let mut bad = request("2026-05");
        bad.symbol = "ABCD11/OTHER11".into();
        assert!(normalize_requests(&[bad], brasilia_date(now())).is_err());
    }

    #[test]
    fn cache_is_local_and_validates_each_observation_before_exposing_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        let service = HistoricalPriceService::new(path.clone()).unwrap();
        let empty = service.read_cache(&[request("2026-05")]).unwrap();
        assert_eq!(empty.pending_requests, 1);
        assert!(!path.exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        let item = saved();
        let cache = Cache {
            version: 1,
            items: BTreeMap::from([(key(&item.request), item)]),
            retry_after: None,
            recent_attempts: Vec::new(),
        };
        write_cache(&path, &cache).unwrap();
        assert_eq!(read_cache(&path, now()).unwrap().items.len(), 1);
        let mut corrupt = cache;
        corrupt
            .items
            .values_mut()
            .next()
            .unwrap()
            .observation
            .as_mut()
            .unwrap()
            .source_url = "https://example.com/price".into();
        write_cache(&path, &corrupt).unwrap();
        let bytes = fs::read(&path).unwrap();
        assert!(read_cache(&path, now()).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn failure_keeps_known_price_and_uses_short_retry_not_success_ttl() {
        let mut item = saved();
        item.available = false;
        item.error = Some("Falha da fonte".into());
        assert!(item_fresh(&item, now() + chrono::Duration::minutes(4)));
        assert!(!item_fresh(&item, now() + chrono::Duration::minutes(6)));
        let cache = Cache {
            version: 1,
            items: BTreeMap::from([(key(&item.request), item)]),
            retry_after: None,
            recent_attempts: Vec::new(),
        };
        let result = panel(
            &[request("2026-05")],
            &cache,
            now() + chrono::Duration::minutes(6),
        );
        assert_eq!(result.observations.len(), 1);
        assert_eq!(result.pending_requests, 1);
        assert_eq!(result.issues.len(), 1);
    }

    #[test]
    fn closed_month_refreshes_after_partial_month_rollover() {
        let mut item = saved();
        item.request = request("2026-10");
        item.requested_through = "2026-10-20".into();
        assert!(item_fresh(&item, now()));
        let november = DateTime::parse_from_rfc3339("2026-11-01T04:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        item.checked_at = stamp(november - chrono::Duration::minutes(10));
        assert!(!item_fresh(&item, november));
    }

    #[test]
    fn explicit_cutoffs_have_distinct_cache_keys_and_cannot_escape_the_month() {
        let mut earlier = request("2026-10");
        earlier.through_date = Some("2026-10-19".into());
        let mut later = earlier.clone();
        later.through_date = Some("2026-10-20".into());
        assert_ne!(key(&earlier), key(&later));
        assert_eq!(
            request_window(&earlier, brasilia_date(now()))
                .unwrap()
                .1
                .to_string(),
            "2026-10-19"
        );
        for invalid in ["2026-09-30", "2026-11-01", "2026-10-22"] {
            earlier.through_date = Some(invalid.into());
            assert!(request_window(&earlier, brasilia_date(now())).is_err());
        }
    }

    #[test]
    fn failed_current_month_refresh_retains_previous_source_window() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        let previous_at = now() - chrono::Duration::days(1);
        let mut body = fixture();
        body["de"] = json!("2026-10-01");
        body["ate"] = json!("2026-10-20");
        body["pregoes"] = json!(14);
        let request = request("2026-10");
        let observation = parse_response(
            &request,
            date("2026-10-01").unwrap(),
            date("2026-10-20").unwrap(),
            &serde_json::to_vec(&body).unwrap(),
            previous_at,
        )
        .unwrap();
        let item = SavedItem {
            request: request.clone(),
            requested_through: "2026-10-21".into(),
            observation: Some(observation.clone()),
            checked_at: stamp(now()),
            available: false,
            error: Some("Fonte indisponível".into()),
        };
        let cache = Cache {
            version: 1,
            items: BTreeMap::from([(key(&request), item)]),
            retry_after: None,
            recent_attempts: Vec::new(),
        };
        write_cache(&path, &cache).unwrap();
        let restored = read_cache(&path, now()).unwrap();
        assert_eq!(
            restored.items.values().next().unwrap().observation,
            Some(observation)
        );
    }

    #[tokio::test]
    async fn fresh_cache_refresh_makes_no_http_reservation_and_preserves_prices() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        let mut item = saved();
        item.checked_at = stamp(Utc::now());
        item.observation.as_mut().unwrap().fetched_at = stamp(Utc::now());
        let cache = Cache {
            version: 1,
            items: BTreeMap::from([(key(&item.request), item)]),
            retry_after: None,
            recent_attempts: Vec::new(),
        };
        write_cache(&path, &cache).unwrap();
        let before = fs::read(&path).unwrap();
        let service = HistoricalPriceService::new(path.clone()).unwrap();
        let result = service.refresh(&[request("2026-05")]).await.unwrap();
        assert_eq!(result.pending_requests, 0);
        assert_eq!(result.observations.len(), 1);
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(!path.with_extension("refresh-policy.json").exists());
    }

    #[tokio::test]
    async fn independent_budget_is_shared_between_instances_and_reserved_before_network() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        let at = Utc::now();
        let mut gate = RefreshGate::acquire(&path).await.unwrap();
        for i in 0..DAILY_LIMIT {
            assert!(gate
                .reserve(&format!("TEST:{i}"), None, 1, at, Some(DAILY_LIMIT))
                .unwrap()
                .is_none());
        }
        drop(gate);
        let mut second = RefreshGate::acquire(&path).await.unwrap();
        assert!(second
            .reserve("TEST:extra", None, 1, at, Some(DAILY_LIMIT))
            .unwrap()
            .unwrap()
            .contains("Limite local"));
        assert!(!path.exists());
        assert!(!path.with_file_name("quotes.refresh-policy.json").exists());
    }

    #[tokio::test]
    async fn rolling_limit_survives_brt_midnight_without_reserving_http() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        // 23:59 BRT and 00:01 BRT are different calendar budgets but remain
        // inside the same rolling 24 hours for an unknown provider reset zone.
        let before: DateTime<Utc> = "2026-01-16T02:59:00Z".parse().unwrap();
        let after: DateTime<Utc> = "2026-01-16T03:01:00Z".parse().unwrap();
        assert_ne!(brasilia_date(before), brasilia_date(after));
        let mut cache = Cache {
            version: 1,
            recent_attempts: vec![stamp(before); DAILY_LIMIT as usize],
            ..Cache::default()
        };
        write_cache(&path, &cache).unwrap();
        let original = fs::read(&path).unwrap();
        let mut gate = RefreshGate::acquire(&path).await.unwrap();
        let reason = reserve_attempt(&mut cache, &path, &mut gate, "TEST:new-day", 0, after)
            .unwrap()
            .unwrap();
        assert!(reason.contains("últimas 24 horas"));
        assert!(!path.with_extension("refresh-policy.json").exists());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(read_cache(&path, after).unwrap().recent_attempts.len(), 160);
    }

    #[tokio::test]
    async fn expired_rolling_attempts_allow_progress_and_new_attempt_is_saved_before_http() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        let before: DateTime<Utc> = "2026-01-16T02:59:00Z".parse().unwrap();
        let after = before + chrono::Duration::hours(24);
        let mut cache = Cache {
            version: 1,
            recent_attempts: vec![stamp(before); DAILY_LIMIT as usize],
            ..Cache::default()
        };
        write_cache(&path, &cache).unwrap();
        let mut gate = RefreshGate::acquire(&path).await.unwrap();
        assert!(
            reserve_attempt(&mut cache, &path, &mut gate, "TEST:expired", 0, after)
                .unwrap()
                .is_none()
        );
        // Simulate cancellation without ever calling HTTP: the reservation is
        // already durable and must still count when another process resumes.
        let restored = read_cache(&path, after).unwrap();
        assert_eq!(restored.recent_attempts, vec![stamp(after)]);
        assert!(restored.items.is_empty());
        assert!(path.with_extension("refresh-policy.json").exists());
    }

    #[test]
    fn legacy_cache_without_rolling_field_loads_and_bad_timestamps_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        let legacy = br#"{"version":1,"items":{},"retryAfter":null}"#;
        fs::write(&path, legacy).unwrap();
        assert!(read_cache(&path, now()).unwrap().recent_attempts.is_empty());
        assert_eq!(fs::read(&path).unwrap(), legacy);
        for attempts in [
            vec!["not a timestamp".to_string()],
            vec![stamp(now() + chrono::Duration::seconds(1))],
            vec![stamp(now()); DAILY_LIMIT as usize + 1],
        ] {
            let invalid = Cache {
                version: 1,
                recent_attempts: attempts,
                ..Cache::default()
            };
            write_cache(&path, &invalid).unwrap();
            let original = fs::read(&path).unwrap();
            assert!(read_cache(&path, now()).is_err());
            assert_eq!(fs::read(&path).unwrap(), original);
        }
    }
}
