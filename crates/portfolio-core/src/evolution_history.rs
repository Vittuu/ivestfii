//! Private observations are derived measurements, never purchase or income rows.
use super::*;
use crate::portfolio_evolution::{validate_history_observation, PortfolioHistoryObservation};

const MAX_HISTORY_MONTHS: usize = 12_000;

impl Repository {
    /// Read this wallet's observations without capturing a new measurement.
    pub fn performance_history(&self) -> Result<Vec<PortfolioHistoryObservation>> {
        let connection = self.connection()?;
        load(&connection)
    }

    /// Keep the latest actual observation in each month. Corrections made on
    /// that same observation date replace its derived value; stale data cannot
    /// move a month's measurement backwards. No financial outbox is written.
    pub fn record_performance_observation(
        &self,
        observation: &PortfolioHistoryObservation,
    ) -> Result<()> {
        validate(observation)?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT payload FROM performance_history WHERE month = ?1",
                [&observation.month],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(payload) = previous {
            let saved: PortfolioHistoryObservation = serde_json::from_str(&payload)?;
            validate(&saved)?;
            if saved.month != observation.month {
                return Err(invalid_history());
            }
            if saved.as_of > observation.as_of || saved == *observation {
                return Ok(());
            }
        }
        tx.execute(
            "INSERT INTO performance_history(month, payload) VALUES (?1, ?2)
             ON CONFLICT(month) DO UPDATE SET payload = excluded.payload",
            params![observation.month, serde_json::to_string(observation)?],
        )?;
        tx.commit()?;
        Ok(())
    }
}

fn invalid_history() -> CoreError {
    CoreError::Validation(
        "O histórico de evolução da carteira contém uma observação inválida.".into(),
    )
}

fn validate(observation: &PortfolioHistoryObservation) -> Result<()> {
    validate_history_observation(observation)?;
    // Market dates belong to Brazil; neither a host timezone nor a JSON import
    // can turn tomorrow's balance into an actual observation today.
    let brazil = chrono::FixedOffset::west_opt(3 * 60 * 60).expect("valid Brazil UTC offset");
    let today = Utc::now()
        .with_timezone(&brazil)
        .format("%Y-%m-%d")
        .to_string();
    if observation.as_of > today {
        return Err(invalid_history());
    }
    Ok(())
}

pub(super) fn load(connection: &Connection) -> Result<Vec<PortfolioHistoryObservation>> {
    let mut statement =
        connection.prepare("SELECT month, payload FROM performance_history ORDER BY month")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut observations = Vec::new();
    for row in rows {
        let (month, payload) = row?;
        let observation: PortfolioHistoryObservation = serde_json::from_str(&payload)?;
        validate(&observation)?;
        if observation.month != month || observations.len() >= MAX_HISTORY_MONTHS {
            return Err(invalid_history());
        }
        observations.push(observation);
    }
    Ok(observations)
}

pub(super) fn parse_embedded(bytes: &[u8]) -> Result<Vec<PortfolioHistoryObservation>> {
    let root: Value =
        serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes))?;
    let Some(value) = root.get("performanceHistory") else {
        return Ok(Vec::new());
    };
    let observations: Vec<PortfolioHistoryObservation> = serde_json::from_value(value.clone())?;
    if observations.len() > MAX_HISTORY_MONTHS {
        return Err(invalid_history());
    }
    let mut months = HashSet::new();
    for observation in &observations {
        validate(observation)?;
        if !months.insert(&observation.month) {
            return Err(invalid_history());
        }
    }
    Ok(observations)
}

pub(super) fn store_imported(
    tx: &Transaction<'_>,
    observations: &[PortfolioHistoryObservation],
) -> Result<()> {
    for observation in observations {
        tx.execute(
            "INSERT INTO performance_history(month, payload) VALUES (?1, ?2)",
            params![observation.month, serde_json::to_string(observation)?],
        )?;
    }
    Ok(())
}
