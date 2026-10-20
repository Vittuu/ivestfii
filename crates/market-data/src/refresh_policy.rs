//! Persistent request reservations shared by app, buttons and MCP processes.
//! Contains public source keys and request counts only, never credentials or positions.
use chrono::{DateTime, Duration, FixedOffset, NaiveDate, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration as StdDuration,
};
use uuid::Uuid;

const MAX_BYTES: usize = 512 * 1024;
const MAX_KEYS: usize = 2000;
pub(crate) const RETRY_SECONDS: i64 = 300;
pub(crate) const QUOTE_DAILY_LIMIT: u32 = 80;

#[derive(Clone, Copy)]
pub(crate) enum RefreshMode {
    Manual,
    Automatic,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshOutcome<T> {
    pub data: Option<T>,
    pub status: String,
    pub message: Option<String>,
}

impl<T> RefreshOutcome<T> {
    pub(crate) fn cached(data: T) -> Self {
        Self {
            data: Some(data),
            status: "cached".into(),
            message: None,
        }
    }
    pub(crate) fn updated(data: T) -> Self {
        Self {
            data: Some(data),
            status: "updated".into(),
            message: None,
        }
    }
    pub(crate) fn failed(data: Option<T>, message: String) -> Self {
        Self {
            status: if data.is_some() {
                "cached"
            } else {
                "unavailable"
            }
            .into(),
            data,
            message: Some(message),
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Policy {
    version: u32,
    attempts: BTreeMap<String, String>,
    daily_requests: BTreeMap<String, u32>,
}

pub(crate) fn fresh(at: Option<&str>, seconds: i64, now: DateTime<Utc>) -> bool {
    at.and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .is_some_and(|at| {
            at <= now + Duration::minutes(5)
                && (now - at.with_timezone(&Utc)).num_seconds() < seconds
        })
}

fn policy_path(cache: &Path) -> PathBuf {
    cache.with_extension("refresh-policy.json")
}

fn read_policy(path: &Path, now: DateTime<Utc>) -> Result<Policy, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Policy { version: 1, ..Policy::default() }),
        Err(_) => return Err("Não foi possível ler a proteção de consultas. Cache preservado; nenhuma consulta foi feita.".into()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Não foi possível ler a proteção de consultas.".to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err(
            "Proteção de consultas excedeu o limite seguro; nenhuma consulta foi feita.".into(),
        );
    }
    let policy: Policy = serde_json::from_slice(&bytes)
        .map_err(|_| "Proteção de consultas inválida; arquivo e cache preservados.".to_string())?;
    if policy.version != 1 || policy.attempts.len() > MAX_KEYS || policy.daily_requests.len() > 40 {
        return Err(
            "Proteção de consultas contém versão ou quantidade inválida; arquivo preservado."
                .into(),
        );
    }
    for (key, at) in &policy.attempts {
        if key.is_empty()
            || key.len() > 100
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-:".contains(&b))
            || DateTime::parse_from_rfc3339(at).is_err()
            || DateTime::parse_from_rfc3339(at).is_ok_and(|at| at > now + Duration::minutes(5))
        {
            return Err(
                "Proteção de consultas contém uma reserva inválida; nenhuma consulta foi feita."
                    .into(),
            );
        }
    }
    let today = brt_day(now);
    if policy.daily_requests.iter().any(|(day, count)| {
        *count > 10_000
            || NaiveDate::parse_from_str(day, "%Y-%m-%d").map_or(true, |day| day > today)
    }) {
        return Err("Contador de consultas inválido; nenhuma consulta foi feita.".into());
    }
    Ok(policy)
}

fn brt_day(now: DateTime<Utc>) -> NaiveDate {
    now.with_timezone(&FixedOffset::west_opt(3 * 3600).expect("Brasília offset"))
        .date_naive()
}

pub(crate) struct RefreshGate {
    _lock: File,
    path: PathBuf,
    policy: Policy,
}

impl RefreshGate {
    pub(crate) async fn acquire(cache: &Path) -> Result<Self, String> {
        let path = policy_path(cache);
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)
                .map_err(|_| "Não foi possível preparar a proteção de consultas.".to_string())?;
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(cache.with_extension("refresh-policy.lock"))
            .map_err(|_| "Não foi possível abrir a proteção de consultas.".to_string())?;
        // Holding this lock over the request coalesces different app/MCP processes.
        for _ in 0..100 {
            match lock.try_lock() {
                Ok(()) => {
                    return Ok(Self {
                        _lock: lock,
                        policy: read_policy(&path, Utc::now())?,
                        path,
                    })
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    tokio::time::sleep(StdDuration::from_millis(100)).await
                }
                Err(_) => return Err("Não foi possível bloquear a proteção de consultas.".into()),
            }
        }
        Err(
            "Outra consulta desta fonte está em andamento. Os dados salvos foram preservados."
                .into(),
        )
    }

    /// Reserve BEFORE HTTP, including failed/cancelled requests in the budget.
    /// Returns a dated deferral message rather than silently bypassing a limit.
    pub(crate) fn reserve(
        &mut self,
        key: &str,
        cached_at: Option<&str>,
        ttl: i64,
        now: DateTime<Utc>,
        daily_limit: Option<u32>,
    ) -> Result<Option<String>, String> {
        if key.is_empty()
            || key.len() > 100
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-:".contains(&b))
        {
            return Err("Identificador de consulta inválido.".into());
        }
        if fresh(cached_at, ttl, now) {
            return Ok(Some(
                "Dados salvos ainda estão dentro do intervalo de atualização desta fonte.".into(),
            ));
        }
        if let Some(at) = self
            .policy
            .attempts
            .get(key)
            .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
        {
            let until = at.with_timezone(&Utc) + Duration::seconds(RETRY_SECONDS);
            if until > now {
                return Ok(Some(format!(
                    "Nova tentativa disponível após {}. Os dados salvos foram preservados.",
                    until.to_rfc3339_opts(SecondsFormat::Secs, true)
                )));
            }
        }
        let day = brt_day(now).to_string();
        if daily_limit.is_some_and(|limit| {
            self.policy.daily_requests.get(&day).copied().unwrap_or(0) >= limit
        }) {
            return Ok(Some("Limite local de consultas de cotações deste dia (Brasília) atingido. Os dados salvos foram preservados.".into()));
        }
        self.policy.attempts.retain(|_, at| {
            DateTime::parse_from_rfc3339(at)
                .is_ok_and(|at| now - at.with_timezone(&Utc) < Duration::days(7))
        });
        if !self.policy.attempts.contains_key(key) && self.policy.attempts.len() >= MAX_KEYS {
            return Err(
                "Proteção de consultas atingiu o limite de fontes; nenhuma consulta foi feita."
                    .into(),
            );
        }
        self.policy
            .attempts
            .insert(key.into(), now.to_rfc3339_opts(SecondsFormat::Secs, true));
        self.policy.daily_requests.retain(|day, _| {
            NaiveDate::parse_from_str(day, "%Y-%m-%d")
                .is_ok_and(|day| brt_day(now).signed_duration_since(day).num_days() < 32)
        });
        if daily_limit.is_some() {
            *self.policy.daily_requests.entry(day).or_default() += 1;
        }
        self.write()?;
        Ok(None)
    }

    fn write(&self) -> Result<(), String> {
        let bytes = serde_json::to_vec(&self.policy)
            .map_err(|_| "Não foi possível serializar a proteção de consultas.".to_string())?;
        if bytes.len() > MAX_BYTES {
            return Err("Proteção de consultas excedeu o limite seguro.".into());
        }
        let temp = self.path.with_extension(format!("{}.tmp", Uuid::new_v4()));
        let result = (|| {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temp, &self.path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
            return Err(
                "Não foi possível salvar a proteção de consultas; nenhuma consulta foi feita."
                    .into(),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-20T14:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }
    #[tokio::test]
    async fn reservations_persist_failure_cooldown_and_budget_counts_all_modes() {
        let dir = std::env::temp_dir().join(format!("ivestfii-policy-{}", Uuid::new_v4()));
        let path = dir.join("quotes.json");
        let mut first = RefreshGate::acquire(&path).await.unwrap();
        assert!(first
            .reserve("MXRF11", None, 1800, at(), Some(2))
            .unwrap()
            .is_none());
        drop(first);
        let mut other = RefreshGate::acquire(&path).await.unwrap();
        assert!(other
            .reserve("MXRF11", None, 1800, at() + Duration::seconds(1), Some(2))
            .unwrap()
            .is_some());
        assert!(other
            .reserve("XPML11", None, 1800, at(), Some(2))
            .unwrap()
            .is_none());
        assert!(other
            .reserve("VGIA11", None, 1800, at(), Some(2))
            .unwrap()
            .unwrap()
            .contains("Limite local"));
        assert!(other
            .reserve("MXRF11", None, 1800, at() + Duration::days(1), Some(2))
            .unwrap()
            .is_none());
        drop(other);
        fs::remove_dir_all(dir).unwrap();
    }
    #[tokio::test]
    async fn fresh_cache_does_not_consume_budget_and_corruption_blocks_requests() {
        let dir = std::env::temp_dir().join(format!("ivestfii-policy-{}", Uuid::new_v4()));
        let path = dir.join("news.json");
        let mut gate = RefreshGate::acquire(&path).await.unwrap();
        assert!(gate
            .reserve("feed", Some("2026-10-20T13:59:00Z"), 3600, at(), Some(1))
            .unwrap()
            .is_some());
        assert!(!policy_path(&path).exists());
        drop(gate);
        fs::write(policy_path(&path), b"broken").unwrap();
        assert!(RefreshGate::acquire(&path).await.is_err());
        assert_eq!(fs::read(policy_path(&path)).unwrap(), b"broken");
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn ttl_boundary_and_brasilia_day_are_explicit() {
        assert!(fresh(Some("2026-10-20T13:30:01Z"), 1800, at()));
        assert!(!fresh(Some("2026-10-20T13:30:00Z"), 1800, at()));
        assert_eq!(
            brt_day(
                DateTime::parse_from_rfc3339("2026-10-20T02:59:59Z")
                    .unwrap()
                    .with_timezone(&Utc)
            )
            .to_string(),
            "2026-10-19"
        );
    }
}
