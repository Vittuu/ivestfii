//! Public, dated market context. Requests never include portfolio or chat data.
//!
//! BCB/SGS 4389 is CDI annualized on 252 business days, 432 is the Selic
//! target (% per year), and 433 is the monthly IPCA variation. These are
//! observations with reference dates, not forecasts or a live trading feed.

use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

use chrono::{DateTime, Days, FixedOffset, NaiveDate, SecondsFormat, Utc};
use refresh_policy::RefreshGate;
use reqwest::{redirect::Policy, Client};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub mod cdi_history;
pub mod cvm;
pub mod news;
pub mod performance;
pub mod public_refresh;
pub mod quotes;
mod refresh_policy;
pub mod settings;
pub use cdi_history::{CdiHistoryService, CdiHistorySnapshot};
pub use cvm::{CvmFundRequest, CvmPanel, CvmService};
pub use news::{NewsItem, NewsService, NewsSnapshot};
pub use quotes::{QuotePanel, QuoteService};
pub use refresh_policy::RefreshOutcome;
pub use settings::{MarketDataSettings, MarketSettingsStore};

/// Whole-batch budgets keep a slow public source from blocking the UI or MCP.
pub async fn bounded_quotes(
    service: &QuoteService,
    symbols: &[String],
    key: Option<&str>,
) -> Result<QuotePanel, String> {
    tokio::time::timeout(Duration::from_secs(45), service.refresh(symbols, key))
        .await
        .unwrap_or_else(|_| {
            Err(
                "A consulta de cotações excedeu 45 segundos. Os dados salvos foram preservados."
                    .into(),
            )
        })
}

pub async fn bounded_cvm(
    service: &CvmService,
    funds: &[CvmFundRequest],
) -> Result<CvmPanel, String> {
    tokio::time::timeout(Duration::from_secs(45), service.refresh(funds))
        .await
        .unwrap_or_else(|_| {
            Err("A consulta CVM excedeu 45 segundos. Os dados salvos foram preservados.".into())
        })
}

const RESPONSE_LIMIT: usize = 256 * 1024;
const CACHE_VERSION: u32 = 1;
const IDS: [&str; 3] = ["cdi", "selic", "ipca"];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EconomicIndicator {
    pub id: String,
    pub name: String,
    /// Exact decimal percentage, never an IEEE-754 monetary value.
    pub value: String,
    pub unit: String,
    pub reference_date: String,
    pub source_id: String,
    pub source: String,
    pub source_url: String,
    pub fetched_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IndicatorError {
    pub id: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EconomicPanel {
    pub indicators: Vec<EconomicIndicator>,
    pub errors: Vec<IndicatorError>,
}

#[derive(Clone, Copy)]
struct Series {
    id: &'static str,
    number: u32,
    name: &'static str,
    unit: &'static str,
}

fn series(id: &str) -> Result<Series, String> {
    match id {
        "cdi" => Ok(Series {
            id: "cdi",
            number: 4389,
            name: "CDI anualizado (base 252)",
            unit: "percent_per_year",
        }),
        "selic" => Ok(Series {
            id: "selic",
            number: 432,
            name: "Selic meta",
            unit: "percent_per_year",
        }),
        "ipca" => Ok(Series {
            id: "ipca",
            number: 433,
            name: "IPCA mensal",
            unit: "percent_per_month",
        }),
        _ => Err("Indicador desconhecido. Use cdi, selic ou ipca.".into()),
    }
}

fn brasilia_date(at: DateTime<Utc>) -> NaiveDate {
    // The source calendar is Brasilia (UTC-3), independent of the user's OS zone.
    at.with_timezone(&FixedOffset::west_opt(3 * 3600).expect("fixed UTC-3 offset"))
        .date_naive()
}

fn endpoint(spec: Series, date: NaiveDate) -> String {
    if spec.id == "selic" {
        // SGS 432 can contain already-scheduled future target observations.
        // A bounded query ending today avoids presenting them as today's rate.
        let start = date
            .checked_sub_days(Days::new(30))
            .expect("current dates can subtract 30 days");
        format!(
            "https://api.bcb.gov.br/dados/serie/bcdata.sgs.{}/dados?formato=json&dataInicial={}&dataFinal={}",
            spec.number,
            start.format("%d/%m/%Y"),
            date.format("%d/%m/%Y")
        )
    } else {
        format!(
            "https://api.bcb.gov.br/dados/serie/bcdata.sgs.{}/dados/ultimos/1?formato=json",
            spec.number
        )
    }
}

#[derive(Clone)]
pub struct EconomicClient {
    client: Client,
}

impl EconomicClient {
    pub fn new() -> Result<Self, String> {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .redirect(Policy::none())
            .https_only(true)
            .user_agent("IvestFII/0.1 (public economic reference data)")
            .build()
            .map_err(|_| {
                "Não foi possível preparar a conexão HTTPS com o Banco Central.".to_string()
            })?;
        Ok(Self { client })
    }

    /// Fetches the three independent series concurrently and keeps partial results.
    pub async fn indicators(&self) -> EconomicPanel {
        let (cdi, selic, ipca) = tokio::join!(
            self.indicator("cdi"),
            self.indicator("selic"),
            self.indicator("ipca")
        );
        let mut panel = EconomicPanel::default();
        for (id, result) in IDS.into_iter().zip([cdi, selic, ipca]) {
            match result {
                Ok(indicator) => panel.indicators.push(indicator),
                Err(message) => panel.errors.push(IndicatorError {
                    id: id.into(),
                    message,
                }),
            }
        }
        panel
    }

    pub async fn indicator(&self, id: &str) -> Result<EconomicIndicator, String> {
        let spec = series(id)?;
        let requested_at = Utc::now();
        let url = endpoint(spec, brasilia_date(requested_at));
        let mut response = self
            .client
            .get(&url)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(http_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(if status.as_u16() == 429 {
                "Banco Central limitou temporariamente as consultas (HTTP 429). Tente novamente depois.".into()
            } else {
                format!(
                    "Banco Central indisponível para esta série (HTTP {}).",
                    status.as_u16()
                )
            });
        }
        if response
            .content_length()
            .is_some_and(|len| len > RESPONSE_LIMIT as u64)
        {
            return Err("Resposta do Banco Central excedeu o limite de 256 KiB.".into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(http_error)? {
            if chunk.len() > RESPONSE_LIMIT.saturating_sub(bytes.len()) {
                return Err("Resposta do Banco Central excedeu o limite de 256 KiB.".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let mut indicator = parse_sgs_response(id, &bytes, Utc::now())?;
        // Keep the URL actually requested, including the dated Selic filter.
        indicator.source_url = url;
        Ok(indicator)
    }
}

fn http_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "A consulta ao Banco Central excedeu o prazo de 10 segundos.".into()
    } else if error.is_connect() {
        "Não foi possível conectar ao Banco Central. Verifique a conexão e tente novamente.".into()
    } else {
        "Falha ao receber uma resposta HTTPS válida do Banco Central.".into()
    }
}

#[derive(Deserialize)]
struct SgsObservation {
    data: String,
    valor: String,
}

fn parse_decimal(value: &str, allow_negative: bool) -> Result<Decimal, String> {
    let normalized = value.replace(',', ".");
    let unsigned = normalized.strip_prefix('-').unwrap_or(&normalized);
    let parts: Vec<_> = unsigned.split('.').collect();
    if value.len() > 40
        || parts.is_empty()
        || parts.len() > 2
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err("Banco Central retornou um percentual decimal inválido.".into());
    }
    let value = Decimal::from_str(&normalized)
        .map_err(|_| "Percentual do Banco Central excedeu a precisão suportada.".to_string())?;
    if value > Decimal::from(1000)
        || value <= Decimal::from(-100)
        || (!allow_negative && value < Decimal::ZERO)
    {
        return Err("Banco Central retornou um percentual fora dos limites esperados.".into());
    }
    Ok(value)
}

fn parse_date(date: &str) -> Result<NaiveDate, String> {
    let parsed = NaiveDate::parse_from_str(date, "%d/%m/%Y")
        .map_err(|_| "Banco Central retornou uma data de referência inválida.".to_string())?;
    if date.len() != 10 || parsed.format("%d/%m/%Y").to_string() != date {
        return Err(
            "Banco Central retornou uma data ambígua ou fora do formato DD/MM/AAAA.".into(),
        );
    }
    Ok(parsed)
}

/// Parses an SGS response without a network request; useful for deterministic
/// validation and offline adapters. Only the three fixed source IDs are accepted.
pub fn parse_sgs_response(
    id: &str,
    bytes: &[u8],
    fetched_at: DateTime<Utc>,
) -> Result<EconomicIndicator, String> {
    let spec = series(id)?;
    if bytes.len() > RESPONSE_LIMIT {
        return Err("Resposta do Banco Central excedeu o limite de 256 KiB.".into());
    }
    let observations: Vec<SgsObservation> = serde_json::from_slice(bytes)
        .map_err(|_| "Banco Central retornou JSON inválido para esta série.".to_string())?;
    if observations.is_empty() || observations.len() > 1000 {
        return Err("Banco Central não retornou observações válidas para esta série.".into());
    }
    let today = brasilia_date(fetched_at);
    let mut selected: Option<(NaiveDate, Decimal)> = None;
    let mut seen_dates = BTreeSet::new();
    for observation in observations {
        let date = parse_date(&observation.data)?;
        let value = parse_decimal(&observation.valor, id == "ipca")?;
        if !seen_dates.insert(date) {
            return Err("Banco Central retornou observações duplicadas para a mesma data.".into());
        }
        if date > today {
            continue;
        }
        if selected.is_none_or(|(latest, _)| date > latest) {
            selected = Some((date, value));
        }
    }
    let (date, value) = selected.ok_or_else(|| {
        "Banco Central retornou apenas referências futuras; nenhuma taxa foi aplicada.".to_string()
    })?;
    Ok(EconomicIndicator {
        id: id.into(),
        name: spec.name.into(),
        value: value.normalize().to_string(),
        unit: spec.unit.into(),
        reference_date: date.format("%Y-%m-%d").to_string(),
        source_id: format!("bcb-sgs-{}", spec.number),
        source: format!("Banco Central do Brasil · SGS {}", spec.number),
        source_url: endpoint(spec, today),
        fetched_at: fetched_at.to_rfc3339_opts(SecondsFormat::Secs, true),
    })
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedPanel {
    version: u32,
    panel: EconomicPanel,
}

/// A shared, atomic cache outside the financial database. Cache reads are local;
/// automatic, manual and MCP refreshes share persistent freshness and retry gates.
#[derive(Clone)]
pub struct EconomicService {
    client: EconomicClient,
    cache_path: PathBuf,
}

impl EconomicService {
    pub fn new(cache_path: PathBuf) -> Result<Self, String> {
        if cache_path.file_name().is_none() {
            return Err("Informe um caminho de arquivo para o cache econômico.".into());
        }
        Ok(Self {
            client: EconomicClient::new()?,
            cache_path,
        })
    }

    pub fn read_cache(&self) -> Result<Option<EconomicPanel>, String> {
        if !self.cache_path.exists() {
            return Ok(None);
        }
        let _lock = cache_lock(&self.cache_path)?;
        read_panel(&self.cache_path)
    }

    pub async fn refresh(&self) -> Result<EconomicPanel, String> {
        let mut policy = RefreshGate::acquire(&self.cache_path).await?;
        let previous = self.read_cache().ok().flatten().unwrap_or_default();
        let mut fetched = EconomicPanel::default();
        let mut requested = BTreeSet::new();
        for id in IDS {
            let saved = previous.indicators.iter().find(|item| item.id == id);
            if saved.is_some_and(|item| {
                refresh_policy::fresh(Some(&item.fetched_at), 6 * 3600, Utc::now())
            }) {
                fetched
                    .indicators
                    .push(saved.expect("checked indicator").clone());
                if let Some(error) = previous.errors.iter().find(|error| error.id == id) {
                    fetched.errors.push(error.clone());
                }
                continue;
            }
            if let Some(message) = policy.reserve(id, None, 6 * 3600, Utc::now(), None)? {
                if let Some(saved) = saved {
                    fetched.indicators.push(saved.clone());
                }
                fetched.errors.push(IndicatorError {
                    id: id.into(),
                    message,
                });
                continue;
            }
            requested.insert(id);
        }
        if requested.is_empty() {
            return Ok(fetched);
        }
        let one = |id: &'static str| {
            let should_fetch = requested.contains(id);
            async move {
                if should_fetch {
                    Some(self.client.indicator(id).await)
                } else {
                    None
                }
            }
        };
        let (cdi, selic, ipca) = tokio::join!(one("cdi"), one("selic"), one("ipca"));
        for (id, result) in IDS.into_iter().zip([cdi, selic, ipca]) {
            let Some(result) = result else {
                continue;
            };
            match result {
                Ok(indicator) => fetched.indicators.push(indicator),
                Err(message) => fetched.errors.push(IndicatorError {
                    id: id.into(),
                    message,
                }),
            }
        }
        // No file lock is held while waiting for a public service.
        self.merge_and_store(fetched)
    }

    pub async fn refresh_if_due(&self) -> RefreshOutcome<EconomicPanel> {
        let before = self.read_cache().ok().flatten();
        match self.refresh().await {
            Ok(data) if !data.errors.is_empty() => {
                let message = data
                    .errors
                    .iter()
                    .map(|error| error.message.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                let status = if data.indicators.is_empty() {
                    "unavailable"
                } else {
                    "cached"
                };
                RefreshOutcome {
                    data: Some(data),
                    status: status.into(),
                    message: Some(message),
                }
            }
            Ok(data)
                if before
                    .as_ref()
                    .is_some_and(|previous| previous.indicators == data.indicators) =>
            {
                RefreshOutcome::cached(data)
            }
            Ok(data) if data.indicators.is_empty() => RefreshOutcome::failed(
                Some(data),
                "Banco Central não retornou indicadores disponíveis. Verifique os erros por série."
                    .into(),
            ),
            Ok(data) => RefreshOutcome::updated(data),
            Err(error) => RefreshOutcome::failed(self.read_cache().ok().flatten(), error),
        }
    }

    /// Reuses CDI for six hours. A failed or deferred new request is an error;
    /// callers must consciously choose whether to use an older cached reference.
    pub async fn cdi_reference(&self) -> Result<EconomicIndicator, String> {
        let mut policy = RefreshGate::acquire(&self.cache_path).await?;
        let cached = self
            .read_cache()
            .ok()
            .flatten()
            .and_then(|panel| panel.indicators.into_iter().find(|item| item.id == "cdi"));
        if cached
            .as_ref()
            .is_some_and(|item| refresh_policy::fresh(Some(&item.fetched_at), 6 * 3600, Utc::now()))
        {
            return Ok(cached.expect("checked CDI"));
        }
        if let Some(message) = policy.reserve("cdi", None, 6 * 3600, Utc::now(), None)? {
            return Err(message);
        }
        let indicator = self.client.indicator("cdi").await?;
        self.merge_and_store(EconomicPanel {
            indicators: vec![indicator.clone()],
            errors: vec![],
        })?;
        Ok(indicator)
    }

    fn merge_and_store(&self, fetched: EconomicPanel) -> Result<EconomicPanel, String> {
        let _lock = cache_lock(&self.cache_path)?;
        let previous = match read_panel(&self.cache_path) {
            Ok(previous) => previous,
            // A fresh, validated public observation can repair a broken cache.
            // A failed request must leave that file byte-for-byte intact.
            Err(_) if !fetched.indicators.is_empty() => None,
            Err(error) => return Err(error),
        };
        let panel = merge_panel(previous, fetched);
        write_panel(&self.cache_path, &panel)?;
        Ok(panel)
    }
}

fn merge_panel(previous: Option<EconomicPanel>, mut fetched: EconomicPanel) -> EconomicPanel {
    let Some(previous) = previous else {
        return fetched;
    };
    let refreshed: BTreeSet<_> = fetched
        .indicators
        .iter()
        .map(|indicator| indicator.id.clone())
        .chain(fetched.errors.iter().map(|error| error.id.clone()))
        .collect();
    for indicator in previous.indicators {
        match fetched
            .indicators
            .iter_mut()
            .find(|next| next.id == indicator.id)
        {
            Some(next) if next.fetched_at < indicator.fetched_at => {
                // Another process may have already written a later observation.
                *next = indicator;
            }
            Some(_) => {}
            None => fetched.indicators.push(indicator),
        }
    }
    // Refreshing one series must not erase errors on the untouched series.
    for error in previous.errors {
        if !refreshed.contains(&error.id) {
            fetched.errors.push(error);
        }
    }
    fetched.indicators.sort_by_key(|indicator| {
        IDS.iter()
            .position(|id| *id == indicator.id)
            .unwrap_or(usize::MAX)
    });
    fetched
}

fn cache_lock(path: &Path) -> Result<File, String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    if let Some(parent) = parent {
        fs::create_dir_all(parent)
            .map_err(|_| "Não foi possível criar a pasta de cache econômico.".to_string())?;
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("lock"))
        .map_err(|_| "Não foi possível abrir o controle do cache econômico.".to_string())?;
    lock.lock().map_err(|_| {
        "Não foi possível bloquear o cache econômico para leitura segura.".to_string()
    })?;
    Ok(lock)
}

fn read_panel(path: &Path) -> Result<Option<EconomicPanel>, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Não foi possível ler o cache econômico; arquivo preservado.".into()),
    };
    let mut bytes = Vec::new();
    file.take(RESPONSE_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Não foi possível ler o cache econômico; arquivo preservado.".to_string())?;
    if bytes.len() > RESPONSE_LIMIT {
        return Err("Cache econômico excedeu 256 KiB; arquivo preservado.".into());
    }
    let cached: CachedPanel = serde_json::from_slice(&bytes)
        .map_err(|_| "Cache econômico inválido; arquivo preservado.".to_string())?;
    if cached.version != CACHE_VERSION {
        return Err("Versão do cache econômico não suportada; arquivo preservado.".into());
    }
    validate_panel(&cached.panel)?;
    Ok(Some(cached.panel))
}

fn validate_panel(panel: &EconomicPanel) -> Result<(), String> {
    if panel.indicators.len() > IDS.len() || panel.errors.len() > IDS.len() {
        return Err("Cache econômico contém séries inesperadas; arquivo preservado.".into());
    }
    let mut seen = BTreeSet::new();
    let now = Utc::now();
    for indicator in &panel.indicators {
        let spec = series(&indicator.id)?;
        let fetched_at = DateTime::parse_from_rfc3339(&indicator.fetched_at)
            .map_err(|_| "Cache econômico contém data de consulta inválida.".to_string())?
            .with_timezone(&Utc);
        let reference = NaiveDate::parse_from_str(&indicator.reference_date, "%Y-%m-%d")
            .map_err(|_| "Cache econômico contém referência inválida.".to_string())?;
        parse_decimal(&indicator.value, spec.id == "ipca")?;
        if !seen.insert(&indicator.id)
            || reference.format("%Y-%m-%d").to_string() != indicator.reference_date
            || reference > brasilia_date(fetched_at)
            || fetched_at > now + chrono::Duration::minutes(5)
            || indicator.source_id != format!("bcb-sgs-{}", spec.number)
            || indicator.unit != spec.unit
            || indicator.name != spec.name
            || indicator.source != format!("Banco Central do Brasil · SGS {}", spec.number)
            || !valid_source_url(spec, &indicator.source_url, fetched_at)
        {
            return Err(
                "Cache econômico contém referência ou origem inválida; arquivo preservado.".into(),
            );
        }
    }
    let mut errors = BTreeSet::new();
    for error in &panel.errors {
        series(&error.id)?;
        if !errors.insert(&error.id) || error.message.len() > 2000 {
            return Err("Cache econômico contém erros inválidos; arquivo preservado.".into());
        }
    }
    Ok(())
}

fn valid_source_url(spec: Series, url: &str, fetched_at: DateTime<Utc>) -> bool {
    let date = brasilia_date(fetched_at);
    if url == endpoint(spec, date) {
        return true;
    }
    // A bounded request started just before midnight may finish the next day.
    spec.id == "selic"
        && date
            .checked_sub_days(Days::new(1))
            .is_some_and(|previous| url == endpoint(spec, previous))
}

fn write_panel(path: &Path, panel: &EconomicPanel) -> Result<(), String> {
    validate_panel(panel)?;
    let bytes = serde_json::to_vec_pretty(&CachedPanel {
        version: CACHE_VERSION,
        panel: panel.clone(),
    })
    .map_err(|_| "Não foi possível serializar o cache econômico.".to_string())?;
    let temporary = path.with_extension(format!("{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|_| "Não foi possível criar o cache econômico temporário.".to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "Não foi possível gravar o cache econômico temporário.".to_string())?;
        drop(file);
        fs::rename(&temporary, path).map_err(|_| {
            "Não foi possível substituir o cache econômico; arquivo anterior preservado."
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

    fn at() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-16T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn fixture(id: &str, value: &str, date: &str) -> EconomicIndicator {
        parse_sgs_response(
            id,
            &serde_json::to_vec(&json!([{ "data": date, "valor": value }])).unwrap(),
            at(),
        )
        .unwrap()
    }

    fn cache_fixture() -> EconomicIndicator {
        parse_sgs_response(
            "cdi",
            br#"[{"data":"02/01/2020","valor":"4.4"}]"#,
            DateTime::parse_from_rfc3339("2020-01-03T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn fresh_economic_cache_keeps_prior_errors_without_http_or_new_reference_dates() {
        let directory =
            std::env::temp_dir().join(format!("ivestfii-economics-policy-{}", Uuid::new_v4()));
        let path = directory.join("economics.json");
        let service = EconomicService::new(path.clone()).unwrap();
        let now = Utc::now();
        let source_date = brasilia_date(now).format("%d/%m/%Y").to_string();
        let mut panel = EconomicPanel::default();
        for id in IDS {
            panel.indicators.push(
                parse_sgs_response(
                    id,
                    &serde_json::to_vec(&json!([{"data":source_date,"valor":"1"}])).unwrap(),
                    now,
                )
                .unwrap(),
            );
        }
        panel.errors.push(IndicatorError {
            id: "cdi".into(),
            message: "Falha anterior preservada".into(),
        });
        service.merge_and_store(panel.clone()).unwrap();
        let original = fs::read(&path).unwrap();
        let normal = service.refresh().await.unwrap();
        assert_eq!(normal, panel);
        let outcome = service.refresh_if_due().await;
        assert_eq!(outcome.status, "cached");
        assert_eq!(outcome.data, Some(panel));
        assert!(outcome.message.unwrap().contains("Falha anterior"));
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(!path.with_extension("refresh-policy.json").exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn cdi_is_annualized_decimal_with_provenance_and_reference_date() {
        let cdi = fixture("cdi", "13.6500000000000001", "14/10/2026");
        assert_eq!(cdi.value, "13.6500000000000001");
        assert_eq!(cdi.unit, "percent_per_year");
        assert_eq!(cdi.source_id, "bcb-sgs-4389");
        assert_eq!(cdi.reference_date, "2026-10-14");
        assert_eq!(cdi.fetched_at, "2026-10-16T12:00:00Z");
        assert_eq!(
            cdi.source_url,
            "https://api.bcb.gov.br/dados/serie/bcdata.sgs.4389/dados/ultimos/1?formato=json"
        );
    }

    #[test]
    fn monthly_ipca_accepts_deflation_without_turning_into_annual_rate() {
        let ipca = fixture("ipca", "-0,110", "01/09/2026");
        assert_eq!(ipca.value, "-0.11");
        assert_eq!(ipca.unit, "percent_per_month");
        assert_eq!(ipca.source_id, "bcb-sgs-433");
    }

    #[test]
    fn selic_selects_latest_eligible_date_without_using_future_schedule() {
        let value = parse_sgs_response(
            "selic",
            br#"[{"data":"04/11/2026","valor":"13.75"},{"data":"15/10/2026","valor":"13.75"},{"data":"14/10/2026","valor":"13.75"}]"#,
            at(),
        )
        .unwrap();
        assert_eq!(value.reference_date, "2026-10-15");
        assert!(value
            .source_url
            .ends_with("dataInicial=16/09/2026&dataFinal=16/10/2026"));
    }

    #[test]
    fn invalid_or_empty_responses_do_not_make_up_rates() {
        for fixture in [
            "[]",
            "{}",
            r#"[{"data":"2026-10-15","valor":"13.65"}]"#,
            r#"[{"data":"15/10/2026","valor":13.65}]"#,
            r#"[{"data":"15/10/2026","valor":"NaN"}]"#,
            r#"[{"data":"15/10/2026","valor":"1.000,00"}]"#,
            r#"[{"data":"15/10/2026","valor":"-1"}]"#,
            r#"[{"data":"15/10/2026","valor":"1001"}]"#,
            r#"[{"data":"4/10/2026","valor":"1"}]"#,
            r#"[{"data":"31/09/2026","valor":"1"}]"#,
            r#"[{"data":"04/11/2026","valor":"13.65"}]"#,
            r#"[{"data":"15/10/2026","valor":"13.65"},{"data":"15/10/2026","valor":"13.5"}]"#,
        ] {
            assert!(
                parse_sgs_response("cdi", fixture.as_bytes(), at()).is_err(),
                "{fixture}"
            );
        }
        assert!(parse_sgs_response("unknown", b"[]", at()).is_err());
        assert!(parse_sgs_response("cdi", &vec![b' '; RESPONSE_LIMIT + 1], at()).is_err());
    }

    #[test]
    fn future_date_is_compared_to_source_calendar_in_brasilia() {
        let near_midnight = DateTime::parse_from_rfc3339("2026-10-16T01:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(parse_sgs_response(
            "cdi",
            br#"[{"data":"16/10/2026","valor":"13.65"}]"#,
            near_midnight
        )
        .is_err());
    }

    #[test]
    fn partial_refresh_retains_old_observation_and_explicit_failure() {
        let cdi = fixture("cdi", "13.65", "14/10/2026");
        let previous = EconomicPanel {
            indicators: vec![cdi.clone()],
            errors: vec![],
        };
        let panel = merge_panel(
            Some(previous),
            EconomicPanel {
                indicators: vec![fixture("ipca", "0.82", "01/09/2026")],
                errors: vec![IndicatorError {
                    id: "cdi".into(),
                    message: "Tempo excedido".into(),
                }],
            },
        );
        assert_eq!(panel.indicators[0], cdi);
        assert_eq!(panel.errors[0].id, "cdi");
        assert_eq!(panel.indicators[0].fetched_at, "2026-10-16T12:00:00Z");
    }

    #[test]
    fn cache_read_is_local_atomic_and_preserves_corrupted_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("economic.json");
        let service = EconomicService::new(path.clone()).unwrap();
        assert_eq!(service.read_cache().unwrap(), None);
        let panel = EconomicPanel {
            indicators: vec![cache_fixture()],
            errors: vec![],
        };
        service.merge_and_store(panel.clone()).unwrap();
        assert_eq!(service.read_cache().unwrap(), Some(panel.clone()));
        service.merge_and_store(panel.clone()).unwrap();
        assert_eq!(service.read_cache().unwrap(), Some(panel));
        fs::write(&path, b"preserve invalid cache").unwrap();
        assert!(service.read_cache().is_err());
        assert_eq!(fs::read(path).unwrap(), b"preserve invalid cache");
    }

    #[test]
    fn cache_rejects_forged_provenance_or_units() {
        let mut panel = EconomicPanel {
            indicators: vec![cache_fixture()],
            errors: vec![],
        };
        panel.indicators[0].source_url = "https://example.org/".into();
        assert!(validate_panel(&panel).is_err());
        panel.indicators[0] = cache_fixture();
        panel.indicators[0].unit = "percent_per_day".into();
        assert!(validate_panel(&panel).is_err());
    }

    #[test]
    fn untouched_series_keeps_failure_until_it_is_refreshed_successfully() {
        let cdi = cache_fixture();
        let previous = EconomicPanel {
            indicators: vec![cdi.clone()],
            errors: vec![IndicatorError {
                id: "cdi".into(),
                message: "Falha anterior".into(),
            }],
        };
        let ipca_only = EconomicPanel {
            indicators: vec![fixture("ipca", "0.82", "01/09/2026")],
            errors: vec![],
        };
        let merged = merge_panel(Some(previous), ipca_only);
        assert_eq!(merged.errors.len(), 1);
        assert_eq!(merged.errors[0].id, "cdi");
        let merged = merge_panel(
            Some(merged),
            EconomicPanel {
                indicators: vec![cdi],
                errors: vec![],
            },
        );
        assert!(merged.errors.is_empty());
    }

    #[test]
    fn valid_refresh_repairs_cache_but_failed_refresh_preserves_original_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("economic.json");
        let service = EconomicService::new(path.clone()).unwrap();
        fs::write(&path, b"broken cache").unwrap();
        let failed = EconomicPanel {
            indicators: vec![],
            errors: vec![IndicatorError {
                id: "cdi".into(),
                message: "Falha HTTP".into(),
            }],
        };
        assert!(service.merge_and_store(failed).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"broken cache");
        let valid = EconomicPanel {
            indicators: vec![cache_fixture()],
            errors: vec![],
        };
        service.merge_and_store(valid.clone()).unwrap();
        assert_eq!(service.read_cache().unwrap(), Some(valid));
    }

    #[test]
    fn later_refresh_cannot_be_overwritten_by_an_older_concurrent_fetch() {
        let later = fixture("cdi", "13.65", "14/10/2026");
        let old = cache_fixture();
        let merged = merge_panel(
            Some(EconomicPanel {
                indicators: vec![later.clone()],
                errors: vec![],
            }),
            EconomicPanel {
                indicators: vec![old],
                errors: vec![],
            },
        );
        assert_eq!(merged.indicators, vec![later]);
    }
}
