//! Basic brapi quotes. Only tickers leave this process; positions and balances do not.
//! Quotes are explicitly requested, dated independently from retrieval, and cached
//! outside the financial database. A token belongs in a backend secret store only.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};

use crate::refresh_policy::{fresh, RefreshGate, RefreshMode, RefreshOutcome, QUOTE_DAILY_LIMIT};
use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::{header::HeaderValue, redirect::Policy, Client};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;
use uuid::Uuid;

const ENDPOINT: &str = "https://brapi.dev/api/v2/stocks/quote";
const RESPONSE_LIMIT: usize = 128 * 1024;
const CACHE_LIMIT: usize = 2 * 1024 * 1024;
const MAX_SYMBOLS: usize = 100;
const MAX_CACHED_SYMBOLS: usize = 2000;
const CACHE_VERSION: u32 = 1;
// All QuoteService instances in this process share the free-plan concurrency cap.
static REQUEST_GATE: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QuotePanel {
    pub items: Vec<QuoteItem>,
    pub fetched_at: Option<String>,
    pub credential_configured: bool,
    pub errors: Vec<String>,
}

impl QuotePanel {
    /// Reading a cache cannot know whether a key exists in the OS vault. The
    /// backend applies that fact without exposing the credential to the UI/MCP.
    pub fn set_credential_presence(&mut self, configured: bool) {
        self.credential_configured = configured;
        if configured {
            for item in &mut self.items {
                if item.status == "not_configured" && item.observation.is_none() {
                    item.status = "unavailable".into();
                    item.error = Some(
                        "Nenhuma cotação salva para este ativo. Atualize para consultar a brapi."
                            .into(),
                    );
                }
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QuoteItem {
    pub symbol: String,
    /// available, cached, not_configured, unauthorized, forbidden,
    /// rate_limited or unavailable. A cached quote can accompany an error.
    pub status: String,
    pub observation: Option<QuoteObservation>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QuoteObservation {
    pub symbol: String,
    /// Exact positive decimal text, not a binary floating-point portfolio value.
    pub price: String,
    pub currency: String,
    /// Source market timestamp, independent from the time we fetched it.
    /// None means the source supplied no reliable time; never imply "live".
    pub quoted_at: Option<String>,
    pub fetched_at: String,
    pub source: String,
    pub source_url: String,
    /// Published update interval for the free plan, not a live-market guarantee.
    pub delay_minutes: Option<u32>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedQuotes {
    version: u32,
    observations: BTreeMap<String, QuoteObservation>,
    last_attempt_at: Option<String>,
    retry_after: Option<String>,
}

impl CachedQuotes {
    fn empty() -> Self {
        Self {
            version: CACHE_VERSION,
            ..Self::default()
        }
    }
}

#[derive(Clone)]
pub struct QuoteService {
    client: Client,
    cache_path: PathBuf,
}

impl QuoteService {
    pub fn new(cache_path: PathBuf) -> Result<Self, String> {
        if cache_path.file_name().is_none() {
            return Err("Informe um arquivo para o cache de cotações.".into());
        }
        let client = Client::builder()
            .https_only(true)
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(20))
            .user_agent("IvestFII/0.1 (public market quotes)")
            .build()
            .map_err(|_| "Não foi possível preparar a conexão de cotações.".to_string())?;
        Ok(Self { client, cache_path })
    }

    /// Purely local. The caller supplies credential presence from its secret
    /// configuration; a credential is deliberately never stored in this cache.
    pub fn read_cache(&self, symbols: &[String]) -> Result<QuotePanel, String> {
        let symbols = normalize_symbols(symbols)?;
        // Atomic writers publish whole files. A cache read must not create a
        // directory or lock file just because the user opened the screen.
        let cached = read_cache_file(&self.cache_path)?;
        Ok(panel_from_cache(&symbols, &cached))
    }

    pub async fn refresh(
        &self,
        symbols: &[String],
        token: Option<&str>,
    ) -> Result<QuotePanel, String> {
        self.refresh_with_mode(symbols, token, RefreshMode::Manual)
            .await
    }

    pub async fn refresh_if_due(
        &self,
        symbols: &[String],
        token: Option<&str>,
    ) -> RefreshOutcome<QuotePanel> {
        if symbols.is_empty() {
            return RefreshOutcome::cached(QuotePanel {
                credential_configured: token.is_some_and(|key| !key.trim().is_empty()),
                ..QuotePanel::default()
            });
        }
        if token.is_none_or(|key| key.trim().is_empty()) {
            let mut data = self.read_cache(symbols).ok();
            if let Some(panel) = &mut data {
                panel.set_credential_presence(false);
            }
            return RefreshOutcome {
                data,
                status: "not_configured".into(),
                message: Some(
                    "Cotações automáticas precisam da chave brapi configurada neste computador."
                        .into(),
                ),
            };
        }
        match self
            .refresh_with_mode(symbols, token, RefreshMode::Automatic)
            .await
        {
            Ok(panel)
                if panel.errors.is_empty()
                    && panel.items.iter().any(|item| item.status == "available") =>
            {
                RefreshOutcome::updated(panel)
            }
            Ok(panel) if panel.errors.is_empty() => RefreshOutcome::cached(panel),
            Ok(panel) => {
                let message = panel.errors.join(" ");
                let status = if panel.items.iter().any(|item| item.observation.is_some()) {
                    "cached"
                } else {
                    "deferred"
                };
                RefreshOutcome {
                    data: Some(panel),
                    status: status.into(),
                    message: Some(message),
                }
            }
            Err(error) => {
                let mut cached = self.read_cache(symbols).ok();
                if let Some(panel) = &mut cached {
                    panel.set_credential_presence(true);
                }
                RefreshOutcome::failed(cached, error)
            }
        }
    }

    async fn refresh_with_mode(
        &self,
        symbols: &[String],
        token: Option<&str>,
        mode: RefreshMode,
    ) -> Result<QuotePanel, String> {
        let symbols = normalize_symbols(symbols)?;
        let token = token.map(str::trim).filter(|s| !s.is_empty());
        let credential = token.is_some();
        let auth = token
            .map(|value| {
                // Sensitive HeaderValue prevents accidental Debug disclosure as well.
                let mut header = HeaderValue::from_str(&format!("Bearer {value}"))
                    .map_err(|_| "A chave brapi contém caracteres inválidos.".to_string())?;
                header.set_sensitive(true);
                Ok::<_, String>(header)
            })
            .transpose()?;
        if symbols.is_empty() {
            return Ok(QuotePanel {
                credential_configured: credential,
                ..QuotePanel::default()
            });
        }
        let _gate = REQUEST_GATE.get_or_init(|| Mutex::new(())).lock().await;
        // The app and its CLI/MCP child can be different processes sharing the
        // same credential. Keep their requests serial without blocking a Tokio
        // worker while another process is using the connection.
        let _process_gate = request_lock(&self.cache_path).await?;
        let before = {
            let _lock = cache_lock(&self.cache_path)?;
            read_refresh_cache(&self.cache_path)?.cached
        };
        let now = Utc::now();
        if let Some(until) = before.retry_after.as_deref().and_then(parse_time) {
            if until > now {
                let failure = QuoteFailure::new(
                    "rate_limited",
                    format!(
                        "Limite brapi atingido. Aguarde até {} antes de atualizar.",
                        until.to_rfc3339()
                    ),
                );
                return Ok(failed_panel(&symbols, &before, credential, &failure, None));
            }
        }
        let mut policy = RefreshGate::acquire(&self.cache_path).await?;
        let ttl = match mode {
            RefreshMode::Manual => 30 * 60,
            RefreshMode::Automatic => 60 * 60,
        };
        let mut skipped = BTreeMap::new();
        let mut outcomes = Vec::new();
        let mut retry_after = None;
        let mut stop: Option<QuoteFailure> = None;
        for symbol in &symbols {
            if before
                .observations
                .get(symbol)
                .is_some_and(|quote| fresh(Some(&quote.fetched_at), ttl, now))
            {
                skipped.insert(
                    symbol.clone(),
                    QuoteItem {
                        symbol: symbol.clone(),
                        status: "cached".into(),
                        observation: before.observations.get(symbol).cloned(),
                        error: None,
                    },
                );
                continue;
            }
            if let Some(failure) = &stop {
                outcomes.push((symbol.clone(), Err(failure.clone())));
                continue;
            }
            if let Some(message) =
                policy.reserve(symbol, None, ttl, Utc::now(), Some(QUOTE_DAILY_LIMIT))?
            {
                skipped.insert(
                    symbol.clone(),
                    failed_item(
                        symbol,
                        &before,
                        &QuoteFailure::new("rate_limited", &message),
                    ),
                );
                continue;
            }
            match self.fetch(symbol, auth.as_ref(), credential).await {
                Ok(observation) => outcomes.push((symbol.clone(), Ok(observation))),
                Err(failure) => {
                    if failure.status == "rate_limited" {
                        retry_after = Some(
                            failure
                                .retry_after
                                .unwrap_or_else(|| Utc::now() + chrono::Duration::seconds(60)),
                        );
                        stop = Some(failure.clone());
                    } else if credential && failure.status == "unauthorized" {
                        // Rejected credentials affect the whole batch. A 403 may
                        // be ticker-specific, so other symbols still need their
                        // own free-plan coverage validation.
                        stop = Some(failure.clone());
                    }
                    outcomes.push((symbol.clone(), Err(failure)));
                }
            }
        }
        if outcomes.is_empty() {
            let mut panel = panel_from_cache(&symbols, &before);
            panel.set_credential_presence(credential);
            panel.items = symbols
                .iter()
                .filter_map(|symbol| skipped.remove(symbol))
                .collect();
            panel.errors = panel
                .items
                .iter()
                .filter_map(|item| item.error.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            return Ok(panel);
        }
        let fetched_at = timestamp(Utc::now());
        let mut panel = commit_refresh(
            &self.cache_path,
            &outcomes,
            credential,
            fetched_at,
            retry_after,
        )?;
        for item in std::mem::take(&mut panel.items) {
            skipped.insert(item.symbol.clone(), item);
        }
        panel.items = symbols
            .iter()
            .filter_map(|symbol| skipped.remove(symbol))
            .collect();
        panel.errors = panel
            .errors
            .into_iter()
            .chain(panel.items.iter().filter_map(|item| item.error.clone()))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        Ok(panel)
    }

    async fn fetch(
        &self,
        symbol: &str,
        auth: Option<&HeaderValue>,
        credential: bool,
    ) -> Result<QuoteObservation, QuoteFailure> {
        let mut request = self.client.get(ENDPOINT).query(&[("symbols", symbol)]);
        if let Some(header) = auth {
            request = request.header(reqwest::header::AUTHORIZATION, header.clone());
        }
        let mut response = request.send().await.map_err(|_| {
            QuoteFailure::new(
                "unavailable",
                "Não foi possível consultar a brapi. Verifique a conexão e tente novamente.",
            )
        })?;
        let status = response.status();
        if !status.is_success() {
            return Err(http_failure(
                status.as_u16(),
                credential,
                response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|x| x.to_str().ok()),
                Utc::now(),
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > RESPONSE_LIMIT as u64)
        {
            return Err(QuoteFailure::new(
                "unavailable",
                "A resposta da brapi excedeu o limite seguro de 128 KiB.",
            ));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| {
            QuoteFailure::new("unavailable", "A resposta da brapi foi interrompida.")
        })? {
            if body.len() + chunk.len() > RESPONSE_LIMIT {
                return Err(QuoteFailure::new(
                    "unavailable",
                    "A resposta da brapi excedeu o limite seguro de 128 KiB.",
                ));
            }
            body.extend_from_slice(&chunk);
        }
        parse_quote(&body, symbol, Utc::now())
            .map_err(|message| QuoteFailure::new("unavailable", message))
    }
}

fn normalize_symbols(symbols: &[String]) -> Result<Vec<String>, String> {
    if symbols.len() > MAX_SYMBOLS {
        return Err("Consulte no máximo 100 ativos por atualização.".into());
    }
    let mut unique = BTreeSet::new();
    for raw in symbols {
        unique.insert(normalize_symbol(raw)?);
    }
    Ok(unique.into_iter().collect())
}

pub fn normalize_symbol(raw: &str) -> Result<String, String> {
    let symbol = raw.trim().to_ascii_uppercase();
    if !valid_symbol(&symbol) {
        return Err("Ticker inválido. Use apenas o código B3, sem URL ou parâmetros.".into());
    }
    Ok(symbol)
}

fn valid_symbol(symbol: &str) -> bool {
    (5..=12).contains(&symbol.len())
        && symbol.as_bytes()[0].is_ascii_uppercase()
        && symbol
            .bytes()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        && symbol.bytes().any(|c| c.is_ascii_digit())
}

fn source_url(symbol: &str) -> String {
    format!("{ENDPOINT}?symbols={symbol}")
}
fn timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}
fn parse_time(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|x| x.with_timezone(&Utc))
}

fn decimal_price(raw: &Value) -> Result<String, String> {
    let text = match raw {
        Value::Number(number) => number.to_string(),
        Value::String(s) => s.clone(),
        _ => return Err("A brapi não retornou um preço válido para o ativo.".into()),
    };
    let decimal = Decimal::from_str_exact(&text)
        .map_err(|_| "A brapi retornou um preço inválido.".to_string())?;
    if decimal <= Decimal::ZERO || decimal > Decimal::from(1_000_000_000_000u64) {
        return Err("A brapi retornou um preço fora dos limites aceitos.".into());
    }
    Ok(decimal.normalize().to_string())
}

fn parse_quote(bytes: &[u8], symbol: &str, now: DateTime<Utc>) -> Result<QuoteObservation, String> {
    let payload: Value =
        serde_json::from_slice(bytes).map_err(|_| "A brapi retornou JSON inválido.".to_string())?;
    let items = payload
        .get("results")
        .and_then(Value::as_array)
        .ok_or("A resposta brapi não contém cotações.")?;
    let mut matching = items
        .iter()
        .filter(|item| item.get("requestedSymbol").and_then(Value::as_str) == Some(symbol));
    let item = matching
        .next()
        .ok_or("A brapi não retornou cotação para o ticker solicitado.")?;
    if matching.next().is_some() {
        return Err("A brapi retornou cotações duplicadas para o ticker.".into());
    }
    if item.get("symbol").and_then(Value::as_str) != Some(symbol)
        || item.get("changed").and_then(Value::as_bool) == Some(true)
    {
        return Err("O ticker mudou na brapi. Confirme o código e eventual conversão antes de usar essa cotação.".into());
    }
    let data = item
        .get("data")
        .and_then(Value::as_object)
        .ok_or("A brapi não retornou dados da cotação.")?;
    let price = decimal_price(data.get("regularMarketPrice").unwrap_or(&Value::Null))?;
    if data.get("currency").and_then(Value::as_str) != Some("BRL") {
        return Err("A moeda da cotação não é BRL; valor ignorado.".into());
    }
    let quoted_at = match data.get("regularMarketTime") {
        Some(Value::String(raw)) => parse_time(raw)
            .filter(|time| *time <= now + chrono::Duration::minutes(5))
            .map(timestamp),
        _ => None,
    };
    Ok(QuoteObservation {
        symbol: symbol.into(),
        price,
        currency: "BRL".into(),
        quoted_at,
        fetched_at: timestamp(now),
        source: "brapi".into(),
        source_url: source_url(symbol),
        delay_minutes: Some(30),
    })
}

#[derive(Clone, Debug)]
struct QuoteFailure {
    status: &'static str,
    message: String,
    retry_after: Option<DateTime<Utc>>,
}
impl QuoteFailure {
    fn new(status: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            retry_after: None,
        }
    }
}

fn http_failure(
    code: u16,
    credential: bool,
    retry_header: Option<&str>,
    now: DateTime<Utc>,
) -> QuoteFailure {
    match code {
        401 if !credential => QuoteFailure::new("not_configured", "Configure uma chave brapi gratuita para consultar este ticker. O sandbox sem chave tem cobertura limitada."),
        401 => QuoteFailure::new("unauthorized", "A brapi recusou a chave. Verifique a credencial nas configurações."),
        403 => QuoteFailure::new("forbidden", "O plano brapi não permite esta consulta. A cobertura gratuita deste ativo ainda não foi confirmada."),
        429 => {
            let until = retry_header.and_then(|raw| raw.parse::<u64>().ok().filter(|n| *n <= 31_536_000).map(|seconds| now + chrono::Duration::seconds(seconds.max(1) as i64)).or_else(|| DateTime::parse_from_rfc2822(raw).ok().map(|x| x.with_timezone(&Utc)))).filter(|x| *x > now).unwrap_or(now + chrono::Duration::seconds(60));
            QuoteFailure { status: "rate_limited", message: format!("Limite brapi atingido. Aguarde até {} antes de atualizar.", until.to_rfc3339()), retry_after: Some(until) }
        },
        404 => QuoteFailure::new("unavailable", "A brapi não encontrou cotação para o ativo."),
        _ => QuoteFailure::new("unavailable", format!("A brapi está indisponível para esta consulta (HTTP {code}).")),
    }
}

fn panel_from_cache(symbols: &[String], cached: &CachedQuotes) -> QuotePanel {
    QuotePanel {
        items: symbols
            .iter()
            .map(|symbol| QuoteItem {
                symbol: symbol.clone(),
                status: if cached.observations.contains_key(symbol) {
                    "cached"
                } else {
                    "not_configured"
                }
                .into(),
                observation: cached.observations.get(symbol).cloned(),
                error: None,
            })
            .collect(),
        fetched_at: cached.last_attempt_at.clone(),
        credential_configured: false,
        errors: vec![],
    }
}

fn failed_item(symbol: &str, cached: &CachedQuotes, failure: &QuoteFailure) -> QuoteItem {
    let observation = cached.observations.get(symbol).cloned();
    QuoteItem {
        symbol: symbol.into(),
        status: if observation.is_some() {
            "cached"
        } else {
            failure.status
        }
        .into(),
        observation,
        error: Some(failure.message.clone()),
    }
}

fn failed_panel(
    symbols: &[String],
    cached: &CachedQuotes,
    credential: bool,
    failure: &QuoteFailure,
    fetched_at: Option<String>,
) -> QuotePanel {
    let items: Vec<_> = symbols
        .iter()
        .map(|symbol| failed_item(symbol, cached, failure))
        .collect();
    let errors = items
        .iter()
        .filter_map(|item| {
            item.error
                .as_ref()
                .map(|error| format!("{}: {error}", item.symbol))
        })
        .collect();
    QuotePanel {
        items,
        fetched_at: fetched_at.or_else(|| cached.last_attempt_at.clone()),
        credential_configured: credential,
        errors,
    }
}

fn cache_lock(path: &Path) -> Result<File, String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|_| "Não foi possível criar a pasta de cache de cotações.".to_string())?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("quotes.lock"))
        .map_err(|_| "Não foi possível abrir o controle do cache de cotações.".to_string())?;
    file.lock()
        .map_err(|_| "Não foi possível bloquear o cache de cotações.".to_string())?;
    Ok(file)
}

async fn request_lock(path: &Path) -> Result<File, String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|_| "Não foi possível criar a pasta de cotações.".to_string())?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("quotes-network.lock"))
        .map_err(|_| "Não foi possível abrir o controle de consultas brapi.".to_string())?;
    for _ in 0..200 {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) => {
                tokio::time::sleep(Duration::from_millis(100)).await
            }
            Err(_) => {
                return Err("Não foi possível bloquear as consultas brapi com segurança.".into())
            }
        }
    }
    Err("Outra consulta brapi está em andamento. Aguarde e tente novamente.".into())
}

fn read_cache_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err("Não foi possível ler o cache de cotações; arquivo preservado.".into())
        }
    };
    let mut bytes = Vec::new();
    file.take(CACHE_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Não foi possível ler o cache de cotações.".to_string())?;
    if bytes.len() > CACHE_LIMIT {
        return Err("Cache de cotações excedeu 2 MiB; arquivo preservado.".into());
    }
    Ok(Some(bytes))
}

fn decode_cache(bytes: &[u8]) -> Result<CachedQuotes, String> {
    let cached: CachedQuotes = serde_json::from_slice(bytes)
        .map_err(|_| "Cache de cotações inválido; arquivo preservado.".to_string())?;
    validate_cache(&cached)?;
    Ok(cached)
}

fn read_cache_file(path: &Path) -> Result<CachedQuotes, String> {
    read_cache_bytes(path)?
        .as_deref()
        .map(decode_cache)
        .unwrap_or_else(|| Ok(CachedQuotes::empty()))
}

struct RefreshCache {
    cached: CachedQuotes,
    corrupt_bytes: Option<Vec<u8>>,
}

/// Only an explicit refresh may reconstruct an invalid, bounded cache. None of
/// its observations is trusted. A parseable backoff from this cache version is
/// retained independently so a damaged observation cannot bypass HTTP 429.
fn read_refresh_cache(path: &Path) -> Result<RefreshCache, String> {
    let mut state = match read_cache_bytes(path)? {
        None => RefreshCache {
            cached: CachedQuotes::empty(),
            corrupt_bytes: None,
        },
        Some(bytes) => match decode_cache(&bytes) {
            Ok(cached) => RefreshCache {
                cached,
                corrupt_bytes: None,
            },
            Err(_) => {
                let mut cached = CachedQuotes::empty();
                if let Ok(raw) = serde_json::from_slice::<Value>(&bytes) {
                    if raw["version"].as_u64() == Some(u64::from(CACHE_VERSION)) {
                        cached.retry_after = raw["retryAfter"]
                            .as_str()
                            .filter(|value| parse_time(value).is_some())
                            .map(str::to_owned);
                    }
                }
                RefreshCache {
                    cached,
                    corrupt_bytes: Some(bytes),
                }
            }
        },
    };
    // This control file is independent of corrupt observations. Reads do not
    // create it; malformed/unreadable metadata must block, never bypass backoff.
    let sidecar = read_cache_file(&recovery_backoff_path(path))
        .map_err(|_| "O controle de espera da recuperação de cotações está inválido ou inacessível; consulta bloqueada para preservar o limite da fonte.".to_string())?;
    if !sidecar.observations.is_empty() {
        return Err(
            "O controle de espera da recuperação contém dados inesperados; consulta bloqueada."
                .into(),
        );
    }
    state.cached.retry_after = state
        .cached
        .retry_after
        .as_deref()
        .and_then(parse_time)
        .into_iter()
        .chain(sidecar.retry_after.as_deref().and_then(parse_time))
        .max()
        .map(timestamp);
    Ok(state)
}

fn recovery_backoff_path(path: &Path) -> PathBuf {
    path.with_extension("recovery-backoff.json")
}

/// Merge the latest validated cache after networking. A failed batch cannot
/// replace corrupt bytes; recovery requires a newly validated observation and
/// an exact local backup before the atomic replacement.
fn commit_refresh(
    path: &Path,
    outcomes: &[(String, Result<QuoteObservation, QuoteFailure>)],
    credential: bool,
    fetched_at: String,
    retry_after: Option<DateTime<Utc>>,
) -> Result<QuotePanel, String> {
    let _lock = cache_lock(path)?;
    let RefreshCache {
        cached: mut merged,
        corrupt_bytes,
    } = read_refresh_cache(path)?;
    let has_observation = outcomes.iter().any(|(_, result)| result.is_ok());
    merged.last_attempt_at = Some(fetched_at.clone());
    // Never discard a valid existing backoff, including one from a partially
    // damaged cache. An explicit future timestamp remains visible to the user.
    merged.retry_after = merged
        .retry_after
        .as_deref()
        .and_then(parse_time)
        .into_iter()
        .chain(retry_after)
        .max()
        .map(timestamp);
    for (symbol, outcome) in outcomes {
        if let Ok(observation) = outcome {
            if symbol != &observation.symbol {
                return Err("A cotação recebida não corresponde ao ativo solicitado.".into());
            }
            match merged.observations.get(symbol) {
                Some(previous) if previous.fetched_at > observation.fetched_at => {}
                _ => {
                    merged
                        .observations
                        .insert(symbol.clone(), observation.clone());
                }
            }
        }
    }
    if corrupt_bytes.is_some() && !has_observation && retry_after.is_some() {
        // A fresh 429 still needs cross-process persistence when replacing the
        // original corrupt cache is forbidden. This file contains no quotes.
        write_cache_file(
            &recovery_backoff_path(path),
            &CachedQuotes {
                last_attempt_at: Some(fetched_at.clone()),
                retry_after: merged.retry_after.clone(),
                ..CachedQuotes::empty()
            },
        )?;
    }
    if corrupt_bytes.is_none() || has_observation {
        // Validate all observations and size before creating a recovery copy.
        validate_cache(&merged)?;
        let size = serde_json::to_vec(&merged)
            .map_err(|_| "Não foi possível serializar o cache de cotações.".to_string())?
            .len();
        if size > CACHE_LIMIT {
            return Err("Cache de cotações excedeu 2 MiB; arquivo anterior preservado.".into());
        }
        if let Some(original) = &corrupt_bytes {
            backup_corrupt_cache(path, original)?;
        }
        write_cache_file(path, &merged)?;
    }
    let mut panel = QuotePanel {
        fetched_at: Some(fetched_at),
        credential_configured: credential,
        ..QuotePanel::default()
    };
    for (symbol, outcome) in outcomes {
        let item = match outcome {
            Ok(_) => QuoteItem {
                symbol: symbol.clone(),
                status: "available".into(),
                observation: merged.observations.get(symbol).cloned(),
                error: None,
            },
            Err(failure) => failed_item(symbol, &merged, failure),
        };
        if let Some(error) = &item.error {
            panel.errors.push(format!("{}: {error}", item.symbol));
        }
        panel.items.push(item);
    }
    if corrupt_bytes.is_some() && !has_observation {
        panel.errors.push(
            "O cache inválido foi preservado porque nenhuma cotação nova foi validada.".into(),
        );
    }
    Ok(panel)
}

fn backup_corrupt_cache(path: &Path, original: &[u8]) -> Result<(), String> {
    if original.len() > CACHE_LIMIT {
        return Err("Cache inválido excede o limite de recuperação; arquivo preservado.".into());
    }
    let backup = path.with_extension(format!("corrupt-{}.json", Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(backup)
        .map_err(|_| {
            "Não foi possível preservar uma cópia do cache inválido; arquivo original mantido."
                .to_string()
        })?;
    file.write_all(original)
        .and_then(|_| file.sync_all())
        .map_err(|_| {
            "Não foi possível salvar a cópia do cache inválido; arquivo original mantido."
                .to_string()
        })
}

fn validate_cache(cached: &CachedQuotes) -> Result<(), String> {
    let invalid = || "Cache de cotações contém dados inválidos; arquivo preservado.".to_string();
    if cached.version != CACHE_VERSION || cached.observations.len() > MAX_CACHED_SYMBOLS {
        return Err(invalid());
    }
    let now = Utc::now();
    if cached
        .last_attempt_at
        .as_ref()
        .is_some_and(|s| parse_time(s).is_none_or(|time| time > now + chrono::Duration::minutes(5)))
        || cached
            .retry_after
            .as_ref()
            .is_some_and(|s| parse_time(s).is_none())
    {
        return Err(invalid());
    }
    for (symbol, observation) in &cached.observations {
        let fetched = parse_time(&observation.fetched_at).ok_or_else(invalid)?;
        if !valid_symbol(symbol)
            || symbol != &observation.symbol
            || decimal_price(&Value::String(observation.price.clone()))
                .ok()
                .as_deref()
                != Some(&observation.price)
            || observation.currency != "BRL"
            || observation.source != "brapi"
            || observation.source_url != source_url(symbol)
            || observation.delay_minutes != Some(30)
            || fetched > now + chrono::Duration::minutes(5)
            || observation.quoted_at.as_ref().is_some_and(|s| {
                parse_time(s).is_none_or(|time| time > fetched + chrono::Duration::minutes(5))
            })
        {
            return Err(invalid());
        }
    }
    Ok(())
}

fn write_cache_file(path: &Path, cached: &CachedQuotes) -> Result<(), String> {
    validate_cache(cached)?;
    let bytes = serde_json::to_vec(cached)
        .map_err(|_| "Não foi possível serializar o cache de cotações.".to_string())?;
    if bytes.len() > CACHE_LIMIT {
        return Err("Cache de cotações excedeu 2 MiB; arquivo anterior preservado.".into());
    }
    let temporary = path.with_extension(format!("{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|_| "Não foi possível criar o cache temporário de cotações.".to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "Não foi possível gravar o cache de cotações.".to_string())?;
        drop(file);
        fs::rename(&temporary, path).map_err(|_| {
            "Não foi possível substituir o cache de cotações; arquivo anterior preservado."
                .to_string()
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
    use serde_json::json;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2020-01-03T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }
    fn response() -> Value {
        json!({"results":[{"requestedSymbol":"MXRF11","symbol":"MXRF11","changed":false,"data":{"currency":"BRL","regularMarketPrice":"9.123456789012345678","regularMarketTime":"2020-01-02T20:59:00Z"}}]})
    }
    fn quote() -> QuoteObservation {
        parse_quote(&serde_json::to_vec(&response()).unwrap(), "MXRF11", now()).unwrap()
    }

    #[test]
    fn parses_exact_price_and_separate_market_retrieval_times() {
        let observation = quote();
        assert_eq!(observation.price, "9.123456789012345678");
        assert_eq!(
            observation.quoted_at.as_deref(),
            Some("2020-01-02T20:59:00Z")
        );
        assert_eq!(observation.fetched_at, "2020-01-03T12:00:00Z");
        assert_eq!(
            observation.source_url,
            "https://brapi.dev/api/v2/stocks/quote?symbols=MXRF11"
        );
        let bytes = br#"{"results":[{"requestedSymbol":"MXRF11","symbol":"MXRF11","data":{"currency":"BRL","regularMarketPrice":9.123456789012345678}}]}"#;
        assert_eq!(
            parse_quote(bytes, "MXRF11", now()).unwrap().price,
            "9.123456789012345678"
        );
    }

    #[test]
    fn rejects_missing_wrong_ticker_alias_or_invalid_price() {
        assert!(decimal_price(&json!("9.123456789012345678901234567890")).is_err());
        assert!(parse_quote(br#"{"results":[]}"#, "MXRF11", now()).is_err());
        for (field, value) in [
            ("regularMarketPrice", json!(null)),
            ("regularMarketPrice", json!(-1)),
            ("currency", json!("USD")),
        ] {
            let mut data = response();
            data["results"][0]["data"][field] = value;
            assert!(parse_quote(&serde_json::to_vec(&data).unwrap(), "MXRF11", now()).is_err());
        }
        let mut data = response();
        data["results"][0]["symbol"] = json!("OTHER11");
        assert!(parse_quote(&serde_json::to_vec(&data).unwrap(), "MXRF11", now()).is_err());
    }

    #[test]
    fn absent_or_future_market_time_never_becomes_retrieval_time() {
        for time in [
            Value::Null,
            json!("bad-date"),
            json!("2030-01-01T12:00:00Z"),
        ] {
            let mut data = response();
            data["results"][0]["data"]["regularMarketTime"] = time;
            assert!(
                parse_quote(&serde_json::to_vec(&data).unwrap(), "MXRF11", now())
                    .unwrap()
                    .quoted_at
                    .is_none()
            );
        }
    }

    #[test]
    fn distinguishes_missing_key_invalid_key_plan_and_rate_limit() {
        assert_eq!(
            http_failure(401, false, None, now()).status,
            "not_configured"
        );
        assert_eq!(http_failure(401, true, None, now()).status, "unauthorized");
        assert_eq!(http_failure(403, true, None, now()).status, "forbidden");
        let failure = http_failure(429, true, Some("180"), now());
        assert_eq!(failure.status, "rate_limited");
        assert_eq!(
            failure.retry_after.unwrap(),
            now() + chrono::Duration::seconds(180)
        );
        assert_eq!(
            http_failure(429, true, None, now()).retry_after.unwrap(),
            now() + chrono::Duration::seconds(60)
        );
    }

    #[test]
    fn failures_preserve_cached_quote_and_original_dates() {
        let mut cached = CachedQuotes::empty();
        cached.observations.insert("MXRF11".into(), quote());
        let item = failed_item("MXRF11", &cached, &http_failure(401, true, None, now()));
        assert_eq!(item.status, "cached");
        assert!(item.error.is_some());
        assert_eq!(item.observation.unwrap(), quote());
        assert_eq!(
            failed_item("XPML11", &cached, &http_failure(429, true, None, now())).status,
            "rate_limited"
        );
    }

    #[tokio::test]
    async fn automatic_empty_wallet_needs_no_key_or_files_and_fresh_quotes_are_reused() {
        let dir = tempfile::tempdir().unwrap();
        let empty_path = dir.path().join("missing/quotes.json");
        let service = QuoteService::new(empty_path.clone()).unwrap();
        let outcome = service.refresh_if_due(&[], None).await;
        assert_eq!(outcome.status, "cached");
        assert!(outcome.message.is_none());
        assert!(!empty_path.parent().unwrap().exists());

        let path = dir.path().join("quotes.json");
        let mut cached = CachedQuotes::empty();
        let mut observed = quote();
        observed.fetched_at = timestamp(Utc::now() - chrono::Duration::minutes(45));
        cached
            .observations
            .insert("MXRF11".into(), observed.clone());
        write_cache_file(&path, &cached).unwrap();
        let bytes = fs::read(&path).unwrap();
        let mut service = QuoteService::new(path.clone()).unwrap();
        service.client = Client::builder()
            .proxy(reqwest::Proxy::all("http://127.0.0.1:9").unwrap())
            .timeout(Duration::from_millis(100))
            .build()
            .unwrap();
        let outcome = service
            .refresh_if_due(&["MXRF11".into()], Some("synthetic-test-credential"))
            .await;
        assert_eq!(outcome.status, "cached");
        assert!(outcome.message.is_none());
        assert_eq!(
            outcome.data.unwrap().items[0].observation.as_ref(),
            Some(&observed)
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(!path.with_extension("refresh-policy.json").exists());
    }

    #[tokio::test]
    async fn manual_and_mcp_reuse_thirty_minute_cache_without_changing_market_or_fetch_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("quotes.json");
        let mut cached = CachedQuotes::empty();
        let mut observed = quote();
        observed.fetched_at = timestamp(Utc::now());
        cached.observations.insert("MXRF11".into(), observed);
        write_cache_file(&path, &cached).unwrap();
        let original = fs::read(&path).unwrap();
        let first = QuoteService::new(path.clone()).unwrap();
        let second = QuoteService::new(path.clone()).unwrap();
        let symbols = vec!["MXRF11".into()];
        let (one, two) = tokio::join!(
            first.refresh(&symbols, None),
            second.refresh(&symbols, None)
        );
        assert_eq!(one.unwrap().items[0].status, "cached");
        assert_eq!(two.unwrap().items[0].status, "cached");
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(!path.with_extension("refresh-policy.json").exists());
    }

    #[test]
    fn local_atomic_cache_is_scoped_and_rejects_tampering_without_overwrite() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("quotes.json");
        let service = QuoteService::new(path.clone()).unwrap();
        let mut cached = CachedQuotes::empty();
        cached.observations.insert("MXRF11".into(), quote());
        {
            let _lock = cache_lock(&path).unwrap();
            write_cache_file(&path, &cached).unwrap();
        }
        let panel = service
            .read_cache(&["MXRF11".into(), "XPML11".into()])
            .unwrap();
        assert_eq!(panel.items[0].status, "cached");
        assert_eq!(panel.items[1].status, "not_configured");
        assert!(!fs::read_to_string(&path).unwrap().contains("token"));
        cached.observations.get_mut("MXRF11").unwrap().source_url = "https://evil.invalid/".into();
        fs::write(&path, serde_json::to_vec(&cached).unwrap()).unwrap();
        let original = fs::read(&path).unwrap();
        assert!(service.read_cache(&["MXRF11".into()]).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    fn damaged_cache(path: &Path, retry_after: Option<String>) -> Vec<u8> {
        let mut cached = CachedQuotes::empty();
        cached.observations.insert("MXRF11".into(), quote());
        let mut tampered = quote();
        tampered.symbol = "XPML11".into();
        tampered.source_url = "https://evil.invalid/".into();
        cached.observations.insert("XPML11".into(), tampered);
        cached.retry_after = retry_after;
        let original = serde_json::to_vec(&cached).unwrap();
        fs::write(path, &original).unwrap();
        original
    }

    fn recovery_copies(directory: &Path) -> Vec<PathBuf> {
        fs::read_dir(directory)
            .unwrap()
            .filter_map(|entry| {
                let path = entry.unwrap().path();
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("quotes.corrupt-")
                    .then_some(path)
            })
            .collect()
    }

    #[test]
    fn explicit_recovery_uses_only_new_quotes_and_backs_up_exact_corrupt_bytes() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("quotes.json");
        let retry = timestamp(Utc::now() - chrono::Duration::seconds(1));
        let original = damaged_cache(&path, Some(retry.clone()));
        let service = QuoteService::new(path.clone()).unwrap();
        assert!(service.read_cache(&["MXRF11".into()]).is_err());
        assert_eq!(fs::read_dir(folder.path()).unwrap().count(), 1);
        let mut newly_fetched = quote();
        newly_fetched.price = "10.123456789".into();
        let panel = commit_refresh(
            &path,
            &[
                ("MXRF11".into(), Ok(newly_fetched.clone())),
                (
                    "XPML11".into(),
                    Err(QuoteFailure::new("unavailable", "Falha sintética")),
                ),
            ],
            true,
            timestamp(Utc::now()),
            None,
        )
        .unwrap();
        assert_eq!(panel.items[0].observation.as_ref(), Some(&newly_fetched));
        assert_eq!(panel.items[1].status, "unavailable");
        assert!(panel.items[1].observation.is_none());
        let repaired = read_cache_file(&path).unwrap();
        assert_eq!(repaired.observations.len(), 1);
        assert_eq!(repaired.retry_after, Some(retry));
        let copies = recovery_copies(folder.path());
        assert_eq!(copies.len(), 1);
        assert_eq!(fs::read(&copies[0]).unwrap(), original);
    }

    #[test]
    fn failed_recovery_preserves_corrupt_file_and_never_returns_its_observations() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("quotes.json");
        let original = damaged_cache(&path, None);
        let panel = commit_refresh(
            &path,
            &[
                ("MXRF11".into(), Err(http_failure(401, true, None, now()))),
                (
                    "XPML11".into(),
                    Err(QuoteFailure::new("unavailable", "Falha sintética")),
                ),
            ],
            true,
            timestamp(Utc::now()),
            None,
        )
        .unwrap();
        assert!(panel.items.iter().all(|item| item.observation.is_none()));
        assert!(panel
            .errors
            .iter()
            .any(|error| error.contains("cache inválido foi preservado")));
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(recovery_copies(folder.path()).is_empty());
        // Even a nominally successful outcome cannot replace the original if
        // validation rejects its price or provenance before persistence.
        let mut invalid = quote();
        invalid.price = "-1".into();
        assert!(commit_refresh(
            &path,
            &[("MXRF11".into(), Ok(invalid))],
            true,
            timestamp(Utc::now()),
            None
        )
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(recovery_copies(folder.path()).is_empty());
    }

    #[tokio::test]
    async fn corrupt_observation_cannot_bypass_a_parseable_persisted_retry_after() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("quotes.json");
        let until = timestamp(Utc::now() + chrono::Duration::minutes(3));
        let original = damaged_cache(&path, Some(until.clone()));
        let service = QuoteService::new(path.clone()).unwrap();
        // This returns before HTTP because the independent backoff survives.
        let panel = service.refresh(&["MXRF11".into()], None).await.unwrap();
        assert_eq!(panel.items[0].status, "rate_limited");
        assert!(panel.items[0].observation.is_none());
        assert!(panel.items[0].error.as_ref().unwrap().contains("Aguarde"));
        assert_eq!(
            read_refresh_cache(&path).unwrap().cached.retry_after,
            Some(until)
        );
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(recovery_copies(folder.path()).is_empty());
    }

    #[tokio::test]
    async fn failed_corrupt_cache_refresh_persists_new_429_for_other_instances() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("quotes.json");
        let original = damaged_cache(&path, None);
        let at = Utc::now();
        let failure = http_failure(429, true, Some("180"), at);
        let until = failure.retry_after.unwrap();
        let panel = commit_refresh(
            &path,
            &[("MXRF11".into(), Err(failure))],
            true,
            timestamp(at),
            Some(until),
        )
        .unwrap();
        assert_eq!(panel.items[0].status, "rate_limited");
        assert!(panel.items[0].observation.is_none());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(recovery_copies(folder.path()).is_empty());
        let sidecar = recovery_backoff_path(&path);
        let saved = read_cache_file(&sidecar).unwrap();
        assert!(saved.observations.is_empty());
        assert_eq!(saved.retry_after, Some(timestamp(until)));
        let sidecar_bytes = fs::read(&sidecar).unwrap();
        let another = QuoteService::new(path.clone()).unwrap();
        // A new process/service with no key still returns before HTTP.
        let blocked = another.refresh(&["MXRF11".into()], None).await.unwrap();
        assert_eq!(blocked.items[0].status, "rate_limited");
        assert!(blocked.items[0].observation.is_none());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::read(sidecar).unwrap(), sidecar_bytes);
    }

    #[tokio::test]
    async fn invalid_or_unexpected_recovery_control_blocks_without_repair_or_http() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("quotes.json");
        let original = damaged_cache(&path, None);
        let sidecar = recovery_backoff_path(&path);
        fs::write(&sidecar, b"broken backoff metadata").unwrap();
        let service = QuoteService::new(path.clone()).unwrap();
        assert!(service
            .refresh(&["MXRF11".into()], None)
            .await
            .unwrap_err()
            .contains("consulta bloqueada"));
        assert_eq!(fs::read(&sidecar).unwrap(), b"broken backoff metadata");
        let mut unexpected = CachedQuotes::empty();
        unexpected.observations.insert("MXRF11".into(), quote());
        write_cache_file(&sidecar, &unexpected).unwrap();
        assert!(read_refresh_cache(&path)
            .err()
            .unwrap()
            .contains("dados inesperados"));
        let io_path = folder.path().join("io-quotes.json");
        fs::write(&io_path, b"damaged quote cache").unwrap();
        fs::create_dir(recovery_backoff_path(&io_path)).unwrap();
        assert!(read_refresh_cache(&io_path)
            .err()
            .unwrap()
            .contains("consulta bloqueada"));
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(recovery_copies(folder.path()).is_empty());
    }

    #[test]
    fn recovery_backoff_uses_later_utc_deadline_and_does_not_delete_expired_metadata() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("quotes.json");
        let later = timestamp(Utc::now() + chrono::Duration::minutes(4));
        damaged_cache(&path, Some(later.clone()));
        let sidecar_path = recovery_backoff_path(&path);
        let mut sidecar = CachedQuotes::empty();
        sidecar.retry_after = Some(timestamp(Utc::now() + chrono::Duration::minutes(2)));
        write_cache_file(&sidecar_path, &sidecar).unwrap();
        assert_eq!(
            read_refresh_cache(&path).unwrap().cached.retry_after,
            Some(later)
        );
        fs::write(&path, b"corrupt JSON").unwrap();
        sidecar.retry_after = Some(timestamp(Utc::now() - chrono::Duration::seconds(1)));
        write_cache_file(&sidecar_path, &sidecar).unwrap();
        let original = fs::read(&sidecar_path).unwrap();
        let restored = read_refresh_cache(&path).unwrap();
        assert!(
            restored
                .cached
                .retry_after
                .as_deref()
                .and_then(parse_time)
                .unwrap()
                < Utc::now()
        );
        assert_eq!(fs::read(&sidecar_path).unwrap(), original);
    }

    #[test]
    fn oversized_cache_is_not_copied_or_reconstructed_and_backups_are_unique() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("quotes.json");
        let original = vec![b'x'; CACHE_LIMIT + 1];
        fs::write(&path, &original).unwrap();
        assert!(read_refresh_cache(&path).is_err());
        assert!(backup_corrupt_cache(&path, &original).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(recovery_copies(folder.path()).is_empty());
        backup_corrupt_cache(&path, b"first").unwrap();
        backup_corrupt_cache(&path, b"second").unwrap();
        let mut contents: Vec<_> = recovery_copies(folder.path())
            .iter()
            .map(|copy| fs::read(copy).unwrap())
            .collect();
        contents.sort();
        assert_eq!(contents, vec![b"first".to_vec(), b"second".to_vec()]);
    }

    #[tokio::test]
    async fn persisted_retry_after_prevents_a_new_request() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("quotes.json");
        let mut cached = CachedQuotes::empty();
        cached.retry_after = Some(timestamp(Utc::now() + chrono::Duration::minutes(3)));
        {
            let _lock = cache_lock(&path).unwrap();
            write_cache_file(&path, &cached).unwrap();
        }
        let service = QuoteService::new(path).unwrap();
        let panel = service.refresh(&["MXRF11".into()], None).await.unwrap();
        assert_eq!(panel.items[0].status, "rate_limited");
        assert!(panel.items[0].error.as_ref().unwrap().contains("Aguarde"));
    }

    #[tokio::test]
    async fn process_request_lock_is_exclusive_and_releases_on_drop() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("quotes.json");
        let held = request_lock(&path).await.unwrap();
        let other = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.with_extension("quotes-network.lock"))
            .unwrap();
        assert!(matches!(
            other.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        drop(held);
        assert!(other.try_lock().is_ok());
    }

    #[test]
    fn symbol_validation_rejects_urls_and_limits_request_scope() {
        assert_eq!(
            normalize_symbols(&[" mxrf11 ".into(), "MXRF11".into()]).unwrap(),
            vec!["MXRF11"]
        );
        assert!(normalize_symbols(&["MXRF11?token=secret".into()]).is_err());
        assert!(normalize_symbols(&vec!["MXRF11".into(); 101]).is_err());
    }
}
