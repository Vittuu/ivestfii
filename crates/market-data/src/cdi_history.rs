//! Daily CDI observations from BCB/SGS 12 (% per day), never annualized.
//!
//! Only explicit `refresh` calls use the network. Queries contain dates, never
//! portfolio data. Missing dates are never replaced with zero or invented rates.
//! No business-day calendar is inferred: weekends and holidays may be absent.
//! A gap greater than seven calendar days is rejected as insufficient coverage,
//! an operational freshness safeguard rather than a holiday classification.

use crate::refresh_policy::{RefreshGate, RefreshOutcome};
use chrono::{DateTime, FixedOffset, Months, NaiveDate, SecondsFormat, Utc};
use reqwest::{redirect::Policy, Client};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};
use uuid::Uuid;

pub const CDI_HISTORY_SERIES_ID: u32 = 12;
pub const CDI_HISTORY_SOURCE: &str = "Banco Central do Brasil · SGS 12";
pub const CDI_HISTORY_UNIT: &str = "% a.d.";
const ENDPOINT: &str = "https://api.bcb.gov.br/dados/serie/bcdata.sgs.12/dados";
const MAX_ROWS: usize = 3664;
const MAX_COVERAGE_GAP_DAYS: i64 = 7;
const MAX_BYTES: usize = 256 * 1024;
const CACHE_VERSION: u32 = 1;
const TTL_SECONDS: i64 = 6 * 60 * 60;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CdiDailyObservation {
    /// Source reference date (YYYY-MM-DD), not the retrieval date.
    pub date: String,
    /// Exact decimal percentage. "0.050788" means 0.050788% on this date.
    pub value: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CdiHistorySnapshot {
    pub series_id: u32,
    pub unit: String,
    pub source: String,
    /// Exact bounded query shared by every observation in this snapshot.
    pub source_url: String,
    pub requested_start_date: String,
    pub requested_end_date: String,
    pub observations: Vec<CdiDailyObservation>,
    pub fetched_at: String,
    /// Recomputed locally on every read. No network is needed to check it.
    pub stale: bool,
}

#[derive(Serialize, Deserialize)]
struct Cache {
    version: u32,
    snapshot: CdiHistorySnapshot,
}

#[derive(Clone)]
pub struct CdiHistoryService {
    client: Client,
    cache_path: PathBuf,
}

impl CdiHistoryService {
    pub fn new(cache_path: PathBuf) -> Result<Self, String> {
        if cache_path.file_name().is_none() {
            return Err("Informe um arquivo para o histórico diário do CDI.".into());
        }
        let cache_path = std::path::absolute(cache_path)
            .map_err(|_| "Caminho do histórico CDI inválido.".to_string())?;
        let client = Client::builder()
            .https_only(true)
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .user_agent("IvestFII/0.1 (BCB public daily CDI history)")
            .build()
            .map_err(|_| {
                "Não foi possível preparar a conexão HTTPS com o Banco Central.".to_string()
            })?;
        Ok(Self { client, cache_path })
    }

    /// Local and read-only: never creates a directory, lock or network request.
    pub fn read_cache(&self) -> Result<Option<CdiHistorySnapshot>, String> {
        read_cache(&self.cache_path, Utc::now())
    }

    /// Reuses an identical range for six hours. Future end dates are clamped
    /// to today in Brasilia. Each query is limited to ten calendar years.
    /// Errors preserve the existing cache, and callers may read it explicitly.
    pub async fn refresh(
        &self,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<CdiHistorySnapshot, String> {
        let range = Range::new(start, end, Utc::now())?;
        // Separate from the atomic file write lock. Cross-process app/MCP calls
        // recheck freshness after acquiring this bounded, asynchronous lock.
        let _network = network_lock(&self.cache_path).await?;
        let mut policy = RefreshGate::acquire(&self.cache_path).await?;
        if let Some(cached) = self.read_cache()? {
            if !cached.stale && range.matches(&cached) {
                return Ok(cached);
            }
        }
        let key = format!(
            "{}:{}",
            range.start.format("%Y%m%d"),
            range.end.format("%Y%m%d")
        );
        if let Some(message) = policy.reserve(&key, None, TTL_SECONDS, Utc::now(), None)? {
            return Err(message);
        }
        let mut response = self
            .client
            .get(endpoint(&range))
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(http_error)?;
        if !response.status().is_success() {
            return Err(if response.status().as_u16() == 429 {
                "O Banco Central limitou as consultas ao histórico CDI (HTTP 429). Tente novamente depois.".into()
            } else {
                format!(
                    "Histórico CDI indisponível no Banco Central (HTTP {}).",
                    response.status().as_u16()
                )
            });
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_BYTES as u64)
        {
            return Err("O histórico CDI excedeu o limite de 256 KiB.".into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(http_error)? {
            if chunk.len() > MAX_BYTES.saturating_sub(bytes.len()) {
                return Err("O histórico CDI excedeu o limite de 256 KiB.".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let snapshot = parse_response(&bytes, &range, Utc::now())?;
        write_cache(&self.cache_path, snapshot)
    }

    pub async fn refresh_if_due(
        &self,
        start: NaiveDate,
        end: NaiveDate,
    ) -> RefreshOutcome<CdiHistorySnapshot> {
        let before = self.read_cache().ok().flatten();
        match self.refresh(start, end).await {
            Ok(data)
                if before
                    .as_ref()
                    .is_some_and(|previous| previous.fetched_at == data.fetched_at) =>
            {
                RefreshOutcome::cached(data)
            }
            Ok(data) => RefreshOutcome::updated(data),
            Err(error) => RefreshOutcome::failed(self.read_cache().ok().flatten(), error),
        }
    }
}

#[derive(Clone)]
struct Range {
    start: NaiveDate,
    end: NaiveDate,
}

impl Range {
    fn new(start: NaiveDate, end: NaiveDate, at: DateTime<Utc>) -> Result<Self, String> {
        let end = end.min(brasilia_date(at));
        let first_available = NaiveDate::from_ymd_opt(1986, 3, 6).expect("series start");
        if start < first_available
            || start > end
            || start
                .checked_add_months(Months::new(120))
                .is_none_or(|limit| end > limit)
        {
            return Err(
                "Consulte um período válido de até dez anos de CDI diário, a partir de 06/03/1986."
                    .into(),
            );
        }
        Ok(Self { start, end })
    }

    fn matches(&self, snapshot: &CdiHistorySnapshot) -> bool {
        snapshot.requested_start_date == self.start.format("%Y-%m-%d").to_string()
            && snapshot.requested_end_date == self.end.format("%Y-%m-%d").to_string()
    }
}

fn brasilia_date(at: DateTime<Utc>) -> NaiveDate {
    at.with_timezone(&FixedOffset::west_opt(3 * 3600).expect("UTC-3"))
        .date_naive()
}

fn endpoint(range: &Range) -> String {
    format!(
        "{ENDPOINT}?formato=json&dataInicial={}&dataFinal={}",
        range.start.format("%d/%m/%Y"),
        range.end.format("%d/%m/%Y")
    )
}

fn parse_date(value: &str) -> Result<NaiveDate, String> {
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| "Data do histórico CDI inválida.".to_string())?;
    if date.format("%Y-%m-%d").to_string() != value {
        return Err("Data do histórico CDI não canônica.".into());
    }
    Ok(date)
}

fn percent(value: &str) -> Result<String, String> {
    let unsigned = value.strip_prefix('-').unwrap_or(value);
    let parts: Vec<_> = unsigned.split('.').collect();
    if value.len() > 40
        || parts.is_empty()
        || parts.len() > 2
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err("Percentual diário do CDI inválido.".into());
    }
    let value = Decimal::from_str(value)
        .map_err(|_| "Percentual diário do CDI fora do limite decimal.".to_string())?;
    if value <= Decimal::from(-100) {
        return Err(
            "Percentual diário do CDI não permite um fator de capitalização positivo.".into(),
        );
    }
    Ok(value.normalize().to_string())
}

#[derive(Deserialize)]
struct SgsObservation {
    data: String,
    valor: String,
}

fn parse_response(
    bytes: &[u8],
    range: &Range,
    at: DateTime<Utc>,
) -> Result<CdiHistorySnapshot, String> {
    if bytes.len() > MAX_BYTES {
        return Err("O histórico CDI excedeu o limite de 256 KiB.".into());
    }
    let rows: Vec<SgsObservation> = serde_json::from_slice(bytes)
        .map_err(|_| "O Banco Central não retornou uma série diária CDI válida.".to_string())?;
    if rows.len() > MAX_ROWS {
        return Err("O Banco Central retornou observações CDI além do período permitido.".into());
    }
    let mut values = BTreeMap::new();
    for row in rows {
        let date = NaiveDate::parse_from_str(&row.data, "%d/%m/%Y")
            .map_err(|_| "Data da observação diária CDI inválida.".to_string())?;
        if date.format("%d/%m/%Y").to_string() != row.data || date < range.start || date > range.end
        {
            return Err(
                "O Banco Central retornou CDI com data inválida ou fora do período solicitado."
                    .into(),
            );
        }
        let value = percent(&row.valor)?;
        if let Some(previous) = values.insert(date, value.clone()) {
            if previous != value {
                return Err(
                    "O Banco Central retornou valores CDI conflitantes para a mesma data.".into(),
                );
            }
        }
    }
    let snapshot = CdiHistorySnapshot {
        series_id: CDI_HISTORY_SERIES_ID,
        unit: CDI_HISTORY_UNIT.into(),
        source: CDI_HISTORY_SOURCE.into(),
        source_url: endpoint(range),
        requested_start_date: range.start.format("%Y-%m-%d").to_string(),
        requested_end_date: range.end.format("%Y-%m-%d").to_string(),
        observations: values
            .into_iter()
            .map(|(date, value)| CdiDailyObservation {
                date: date.format("%Y-%m-%d").to_string(),
                value,
            })
            .collect(),
        fetched_at: at.to_rfc3339_opts(SecondsFormat::Secs, true),
        stale: false,
    };
    validate_snapshot(&snapshot, at)?;
    Ok(snapshot)
}

fn validate_snapshot(snapshot: &CdiHistorySnapshot, now: DateTime<Utc>) -> Result<(), String> {
    let at = DateTime::parse_from_rfc3339(&snapshot.fetched_at)
        .map_err(|_| "Data de consulta do histórico CDI inválida.".to_string())?
        .with_timezone(&Utc);
    if at > now + chrono::Duration::minutes(5) {
        return Err("O histórico CDI possui data de consulta futura.".into());
    }
    let range = Range::new(
        parse_date(&snapshot.requested_start_date)?,
        parse_date(&snapshot.requested_end_date)?,
        at,
    )?;
    if snapshot.series_id != CDI_HISTORY_SERIES_ID
        || snapshot.unit != CDI_HISTORY_UNIT
        || snapshot.source != CDI_HISTORY_SOURCE
        || snapshot.source_url != endpoint(&range)
        || !range.matches(snapshot)
        || snapshot.observations.len() > MAX_ROWS
    {
        return Err("Origem, período ou unidade do histórico CDI inválidos.".into());
    }
    let first = snapshot.observations.first()
        .ok_or_else(|| "O Banco Central não publicou CDI no período solicitado; a comparação não está disponível.".to_string())?;
    let first_date = parse_date(&first.date)?;
    if (first_date - range.start).num_days() > MAX_COVERAGE_GAP_DAYS {
        return Err(format!("O histórico CDI começa em {}, mais de sete dias após o início solicitado; a comparação não está disponível.",first.date));
    }
    let mut previous = None;
    for item in &snapshot.observations {
        let date = parse_date(&item.date)?;
        if date < range.start
            || date > range.end
            || previous.is_some_and(|prev| date <= prev)
            || percent(&item.value)? != item.value
        {
            return Err("Observações do histórico CDI inválidas ou duplicadas.".into());
        }
        if previous.is_some_and(|prev| (date - prev).num_days() > MAX_COVERAGE_GAP_DAYS) {
            return Err(format!("Há uma interrupção superior a sete dias no histórico CDI antes de {}; a comparação não está disponível.",item.date));
        }
        previous = Some(date);
    }
    let last = previous.expect("nonempty validated observations");
    if (range.end - last).num_days() > MAX_COVERAGE_GAP_DAYS {
        return Err(format!("O histórico CDI termina em {}, mais de sete dias antes de {}; atualize antes de comparar.",last,range.end));
    }
    Ok(())
}

fn is_stale(snapshot: &CdiHistorySnapshot, now: DateTime<Utc>) -> bool {
    DateTime::parse_from_rfc3339(&snapshot.fetched_at).map_or(true, |at| {
        (now - at.with_timezone(&Utc)).num_seconds() >= TTL_SECONDS
    })
}

fn read_cache(path: &Path, now: DateTime<Utc>) -> Result<Option<CdiHistorySnapshot>, String> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Não foi possível ler o histórico CDI salvo.".into()),
    };
    let mut bytes = Vec::new();
    (&mut file)
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "Não foi possível ler o histórico CDI salvo.".to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("O cache do histórico CDI excedeu 256 KiB; arquivo preservado.".into());
    }
    let cache: Cache = serde_json::from_slice(&bytes)
        .map_err(|_| "Cache do histórico CDI inválido; arquivo preservado.".to_string())?;
    if cache.version != CACHE_VERSION {
        return Err("Versão do cache de histórico CDI não suportada.".into());
    }
    let mut snapshot = cache.snapshot;
    validate_snapshot(&snapshot, now)?;
    snapshot.stale = is_stale(&snapshot, now);
    Ok(Some(snapshot))
}

async fn network_lock(path: &Path) -> Result<File, String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|_| "Não foi possível criar a pasta do histórico CDI.".to_string())?;
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("cdi-refresh.lock"))
        .map_err(|_| {
            "Não foi possível abrir o controle de consultas do histórico CDI.".to_string()
        })?;
    for _ in 0..150 {
        match lock.try_lock() {
            Ok(()) => return Ok(lock),
            Err(std::fs::TryLockError::WouldBlock) => {
                tokio::time::sleep(Duration::from_millis(100)).await
            }
            Err(_) => return Err("Não foi possível bloquear as consultas do histórico CDI.".into()),
        }
    }
    Err("Outra atualização do histórico CDI está em andamento. Tente novamente depois.".into())
}

fn write_cache(path: &Path, snapshot: CdiHistorySnapshot) -> Result<CdiHistorySnapshot, String> {
    validate_snapshot(&snapshot, Utc::now())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|_| "Não foi possível criar a pasta do histórico CDI.".to_string())?;
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("cdi-write.lock"))
        .map_err(|_| "Não foi possível abrir o bloqueio do histórico CDI.".to_string())?;
    lock.lock()
        .map_err(|_| "Não foi possível bloquear a gravação do histórico CDI.".to_string())?;
    // Never replace a newer result from another process with this older fetch.
    if let Some(previous) = read_cache(path, Utc::now())? {
        let previous_at =
            DateTime::parse_from_rfc3339(&previous.fetched_at).expect("validated timestamp");
        let fetched_at =
            DateTime::parse_from_rfc3339(&snapshot.fetched_at).expect("validated timestamp");
        if previous_at > fetched_at {
            return Ok(
                if previous.requested_start_date == snapshot.requested_start_date
                    && previous.requested_end_date == snapshot.requested_end_date
                {
                    previous
                } else {
                    // Preserve the newer cache without returning an unrelated range.
                    snapshot
                },
            );
        }
    }
    let bytes = serde_json::to_vec(&Cache {
        version: CACHE_VERSION,
        snapshot: snapshot.clone(),
    })
    .map_err(|_| "Não foi possível preparar o histórico CDI para gravação.".to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("O cache do histórico CDI excedeu 256 KiB.".into());
    }
    let temporary = path.with_extension(format!("{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|_| "Não foi possível criar o histórico CDI temporário.".to_string())?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_| "Não foi possível salvar o histórico CDI temporário.".to_string())?;
        drop(file);
        fs::rename(&temporary, path)
            .map_err(|_| "Não foi possível substituir o cache do histórico CDI.".to_string())?;
        Ok(snapshot)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn http_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "A consulta ao histórico CDI excedeu 10 segundos; o cache foi preservado.".into()
    } else {
        "Não foi possível obter o histórico CDI por HTTPS; o cache foi preservado.".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-20T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn day(value: &str) -> NaiveDate {
        parse_date(value).unwrap()
    }

    fn range() -> Range {
        Range::new(day("2026-10-10"), day("2026-10-19"), at()).unwrap()
    }

    fn response() -> &'static [u8] {
        br#"[{"data":"13/10/2026","valor":"0.050788"},{"data":"14/10/2026","valor":"0.050788"},{"data":"15/10/2026","valor":"0.050788"},{"data":"16/10/2026","valor":"0.050788"},{"data":"19/10/2026","valor":"0.050788"}]"#
    }

    fn snapshot() -> CdiHistorySnapshot {
        parse_response(response(), &range(), at()).unwrap()
    }

    #[test]
    fn daily_percent_is_not_annualized_and_keeps_public_provenance_and_exact_dates() {
        let item = snapshot();
        assert_eq!(item.series_id, 12);
        assert_eq!(item.unit, "% a.d.");
        assert_eq!(item.source, CDI_HISTORY_SOURCE);
        assert_eq!(item.source_url,"https://api.bcb.gov.br/dados/serie/bcdata.sgs.12/dados?formato=json&dataInicial=10/10/2026&dataFinal=19/10/2026");
        assert_eq!(item.requested_start_date, "2026-10-10");
        assert_eq!(item.requested_end_date, "2026-10-19");
        assert_eq!(item.fetched_at, "2026-10-20T12:00:00Z");
        assert_eq!(item.observations.len(), 5);
        assert_eq!(item.observations[0].date, "2026-10-13");
        assert_eq!(item.observations[0].value, "0.050788");
        // October 12 is a holiday. No inferred business-day row or rate exists.
        assert!(!item
            .observations
            .iter()
            .any(|item| item.date == "2026-10-12"));
        let json = serde_json::to_value(item).unwrap();
        assert_eq!(json["seriesId"], 12);
        assert_eq!(json["requestedEndDate"], "2026-10-19");
    }

    #[test]
    fn public_query_is_bounded_to_ten_years_and_today_in_brasilia() {
        let midnight = DateTime::parse_from_rfc3339("2026-10-20T01:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let clipped = Range::new(day("2026-10-10"), day("2030-10-25"), midnight).unwrap();
        assert_eq!(clipped.end, day("2026-10-19"));
        assert!(Range::new(day("2016-10-20"), day("2026-10-20"), at()).is_ok());
        assert!(Range::new(day("2016-10-19"), day("2026-10-20"), at()).is_err());
        assert!(Range::new(day("1986-03-05"), day("1986-03-10"), at()).is_err());
        assert!(Range::new(day("2026-10-21"), day("2026-10-30"), at()).is_err());
        assert!(Range::new(day("2026-10-20"), day("2026-10-19"), at()).is_err());
        assert!(!endpoint(&clipped).contains("2030"));
    }

    #[test]
    fn absent_dates_are_not_zero_and_weekend_endpoints_are_valid() {
        let requested = Range::new(day("2026-10-16"), day("2026-10-18"), at()).unwrap();
        let parsed = parse_response(
            br#"[{"data":"16/10/2026","valor":"0.050788"}]"#,
            &requested,
            at(),
        )
        .unwrap();
        assert_eq!(parsed.observations.len(), 1);
        assert_eq!(parsed.observations[0].date, "2026-10-16");
        let zero = parse_response(
            br#"[{"data":"16/10/2026","valor":"0.000000"}]"#,
            &requested,
            at(),
        )
        .unwrap();
        assert_eq!(zero.observations[0].value, "0");
    }

    #[test]
    fn empty_or_materially_incomplete_coverage_never_becomes_a_zero_benchmark() {
        assert!(parse_response(b"[]", &range(), at())
            .unwrap_err()
            .contains("não publicou"));
        let late = Range::new(day("2026-09-01"), day("2026-10-19"), at()).unwrap();
        assert!(parse_response(response(), &late, at())
            .unwrap_err()
            .contains("começa"));
        let trailing = Range::new(day("2026-10-01"), day("2026-10-19"), at()).unwrap();
        assert!(parse_response(
            br#"[{"data":"01/10/2026","valor":"0.05"}]"#,
            &trailing,
            at()
        )
        .unwrap_err()
        .contains("termina"));
        let gap = br#"[{"data":"01/10/2026","valor":"0.05"},{"data":"19/10/2026","valor":"0.05"}]"#;
        assert!(parse_response(gap, &trailing, at())
            .unwrap_err()
            .contains("interrupção"));
    }

    #[test]
    fn duplicates_and_unordered_rows_are_handled_without_arbitrary_conflict_selection() {
        let equal = br#"[{"data":"19/10/2026","valor":"0.0507880"},{"data":"13/10/2026","valor":"0.050788"},{"data":"19/10/2026","valor":"0.050788"}]"#;
        let parsed = parse_response(equal, &range(), at()).unwrap();
        assert_eq!(parsed.observations.len(), 2);
        assert_eq!(parsed.observations[0].date, "2026-10-13");
        let conflict =
            br#"[{"data":"19/10/2026","valor":"0.05"},{"data":"19/10/2026","valor":"0.06"}]"#;
        assert!(parse_response(conflict, &range(), at())
            .unwrap_err()
            .contains("conflitantes"));
    }

    #[test]
    fn rejects_untrusted_schema_dates_percentages_and_oversize_payloads() {
        for invalid in [
            b"{}".as_slice(),
            b"<html>unavailable</html>",
            br#"[{"data":"19/10/2026"}]"#,
            br#"[{"data":"19/10/2026","valor":0.05}]"#,
            br#"[{"data":"9/10/2026","valor":"0.05"}]"#,
            br#"[{"data":"31/02/2026","valor":"0.05"}]"#,
            br#"[{"data":"20/10/2026","valor":"0.05"}]"#,
            br#"[{"data":"01/10/2026","valor":"0.05"}]"#,
        ] {
            assert!(parse_response(invalid, &range(), at()).is_err());
        }
        for invalid in [
            "NaN",
            "Infinity",
            "1e-2",
            "0,05",
            " 0.05",
            "0.05 ",
            "+0.05",
            "",
            "-100",
            "-101",
            "999999999999999999999999999999999999999999999",
        ] {
            assert!(percent(invalid).is_err(), "{invalid}");
        }
        assert_eq!(percent("-0.05").unwrap(), "-0.05");
        assert!(parse_response(&vec![b' '; MAX_BYTES + 1], &range(), at()).is_err());
        let rows = format!(
            "[{}]",
            vec![r#"{"data":"19/10/2026","valor":"0.05"}"#; MAX_ROWS + 1].join(",")
        );
        assert!(parse_response(rows.as_bytes(), &range(), at()).is_err());
    }

    #[test]
    fn local_reads_are_side_effect_free_and_recompute_expiration() {
        let folder = tempfile::tempdir().unwrap();
        let absent = folder.path().join("not-created").join("cdi.json");
        let service = CdiHistoryService::new(absent.clone()).unwrap();
        assert!(service.read_cache().unwrap().is_none());
        assert!(!absent.parent().unwrap().exists());
        let path = folder.path().join("cdi.json");
        write_cache(&path, snapshot()).unwrap();
        let original = fs::read(&path).unwrap();
        assert!(
            !read_cache(&path, at() + chrono::Duration::seconds(TTL_SECONDS - 1))
                .unwrap()
                .unwrap()
                .stale
        );
        assert!(
            read_cache(&path, at() + chrono::Duration::seconds(TTL_SECONDS))
                .unwrap()
                .unwrap()
                .stale
        );
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(read_cache(&path, at()).unwrap().unwrap(), snapshot());
    }

    #[test]
    fn cache_validation_rejects_forged_origin_and_cannot_hide_missing_coverage() {
        for change in 0..12 {
            let mut changed = snapshot();
            match change {
                0 => changed.series_id = 4391,
                1 => changed.unit = "% a.a.".into(),
                2 => changed.source = "Other source".into(),
                3 => changed.source_url = changed.source_url.replace("api.bcb.gov.br", "evil.test"),
                4 => changed.requested_end_date = "2026-10-20".into(),
                5 => changed.observations[0].date = "2026-10-01".into(),
                6 => changed.observations[0].value = "NaN".into(),
                7 => changed.observations[0].value = "0.0507880".into(),
                8 => changed.observations.swap(0, 1),
                9 => changed.observations.push(changed.observations[0].clone()),
                10 => changed.fetched_at = "2026-11-20T12:00:00Z".into(),
                _ => changed.observations.clear(),
            }
            assert!(
                validate_snapshot(&changed, at()).is_err(),
                "mutation {change}"
            );
        }
    }

    #[test]
    fn failed_validation_preserves_previous_cache_and_original_reference_dates() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("cdi.json");
        write_cache(&path, snapshot()).unwrap();
        let original = fs::read(&path).unwrap();
        let result =
            parse_response(b"[]", &range(), at()).and_then(|item| write_cache(&path, item));
        assert!(result.is_err());
        let mut invalid = snapshot();
        invalid.observations[0].value = "NaN".into();
        assert!(write_cache(&path, invalid).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(
            read_cache(&path, at()).unwrap().unwrap().fetched_at,
            "2026-10-20T12:00:00Z"
        );
    }

    #[test]
    fn corrupt_or_oversize_cache_is_preserved_without_automatic_repair() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("cdi.json");
        for bytes in [
            b"{bad-json".to_vec(),
            vec![b' '; MAX_BYTES + 1],
            serde_json::to_vec(&Cache {
                version: 99,
                snapshot: snapshot(),
            })
            .unwrap(),
        ] {
            fs::write(&path, &bytes).unwrap();
            assert!(read_cache(&path, at()).is_err());
            assert!(write_cache(&path, snapshot()).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn older_refresh_cannot_replace_newer_cache_or_return_a_different_requested_range() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("cdi.json");
        let mut older = snapshot();
        older.fetched_at =
            (Utc::now() - chrono::Duration::hours(2)).to_rfc3339_opts(SecondsFormat::Secs, true);
        let mut newer = older.clone();
        newer.fetched_at =
            (Utc::now() - chrono::Duration::hours(1)).to_rfc3339_opts(SecondsFormat::Secs, true);
        write_cache(&path, newer.clone()).unwrap();
        assert_eq!(write_cache(&path, older.clone()).unwrap(), newer);
        let requested = Range::new(day("2026-10-13"), day("2026-10-19"), at()).unwrap();
        let different = parse_response(response(), &requested, at()).unwrap();
        let returned = write_cache(&path, different.clone()).unwrap();
        assert_eq!(returned, different);
        assert_eq!(
            read_cache(&path, Utc::now())
                .unwrap()
                .unwrap()
                .requested_start_date,
            "2026-10-10"
        );
    }

    #[tokio::test]
    async fn invalid_range_has_no_io_and_matching_fresh_cache_does_not_need_http() {
        let folder = tempfile::tempdir().unwrap();
        let absent = folder.path().join("not-created").join("cdi.json");
        let service = CdiHistoryService::new(absent.clone()).unwrap();
        assert!(service
            .refresh(day("2026-10-20"), day("2026-10-19"))
            .await
            .is_err());
        assert!(!absent.parent().unwrap().exists());
        let path = folder.path().join("cdi.json");
        let now = Utc::now();
        let end = brasilia_date(now);
        let row = format!(
            r#"[{{"data":"{}","valor":"0.050788"}}]"#,
            end.format("%d/%m/%Y")
        );
        let requested = Range::new(end, end, now).unwrap();
        let fresh = parse_response(row.as_bytes(), &requested, now).unwrap();
        write_cache(&path, fresh.clone()).unwrap();
        let service = CdiHistoryService::new(path).unwrap();
        // An invalid proxy proves the TTL path uses no socket or remote request.
        let service = CdiHistoryService {
            client: Client::builder()
                .proxy(reqwest::Proxy::all("http://127.0.0.1:1").unwrap())
                .timeout(Duration::from_millis(50))
                .build()
                .unwrap(),
            ..service
        };
        assert_eq!(service.refresh(end, end).await.unwrap(), fresh);
    }

    #[tokio::test]
    #[ignore = "Manual public BCB smoke; network and isolated temporary cache only"]
    async fn live_public_daily_cdi_refreshes_and_reuses_its_dated_cache() {
        let folder = tempfile::tempdir().unwrap();
        let service = CdiHistoryService::new(folder.path().join("cdi.json")).unwrap();
        let end = brasilia_date(Utc::now()).pred_opt().unwrap();
        let start = end.checked_sub_days(chrono::Days::new(30)).unwrap();
        let first = service.refresh(start, end).await.unwrap();
        assert_eq!(first.series_id, 12);
        assert!(!first.observations.is_empty());
        assert_eq!(service.read_cache().unwrap().unwrap(), first);
        assert_eq!(service.refresh(start, end).await.unwrap(), first);
        println!(
            "{} observações CDI, de {} até {}, consulta {}",
            first.observations.len(),
            first.observations.first().unwrap().date,
            first.observations.last().unwrap().date,
            first.fetched_at
        );
    }
}
