//! Read-only integration of ledger calculations and dated public observations.
use crate::{CdiHistorySnapshot, QuotePanel};
use chrono::{DateTime, FixedOffset, NaiveDate};
use ivestfii_core::{
    performance::{calculate_fii_performance, DailyCdiRate, FiiPerformance, PerformanceQuote},
    portfolio_performance::{calculate_portfolio_performance, PortfolioPerformance},
    PortfolioSnapshot,
};
use serde::Serialize;

/// Read the same public observations as the desktop, without network or ledger writes.
pub fn cached_report(
    snapshot: &PortfolioSnapshot,
    directory: &std::path::Path,
    today: NaiveDate,
) -> Result<PerformanceReport, String> {
    if snapshot.funds.is_empty() {
        return build_report(snapshot, &QuotePanel::default(), None, today, vec![]);
    }
    let symbols = snapshot
        .funds
        .iter()
        .map(|fund| fund.ticker.clone())
        .collect::<Vec<_>>();
    let mut errors = Vec::new();
    let panel = crate::QuoteService::new(directory.join("public-data/quotes.json"))?
        .read_cache(&symbols)
        .unwrap_or_else(|error| {
            errors.push(error);
            QuotePanel::default()
        });
    let history = crate::CdiHistoryService::new(directory.join("public-data/cdi-history.json"))?
        .read_cache()
        .unwrap_or_else(|error| {
            errors.push(error);
            None
        });
    build_report(snapshot, &panel, history.as_ref(), today, errors)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceReport {
    #[serde(flatten)]
    pub calculation: FiiPerformance,
    pub portfolio: PortfolioPerformance,
    pub quote_source: Option<String>,
    pub quote_fetched_at: Option<String>,
    pub cdi_source: Option<String>,
    pub cdi_source_url: Option<String>,
    pub cdi_fetched_at: Option<String>,
}

pub fn build_report(
    snapshot: &PortfolioSnapshot,
    panel: &QuotePanel,
    history: Option<&CdiHistorySnapshot>,
    today: NaiveDate,
    errors: Vec<String>,
) -> Result<PerformanceReport, String> {
    let offset = FixedOffset::west_opt(3 * 60 * 60).expect("Brasília offset");
    let quotes = panel
        .items
        .iter()
        .filter_map(|item| {
            let quote = item.observation.as_ref()?;
            if quote.currency != "BRL" || quote.symbol != item.symbol {
                return None;
            }
            let as_of = DateTime::parse_from_rfc3339(quote.quoted_at.as_deref()?)
                .ok()?
                .with_timezone(&offset)
                .date_naive()
                .to_string();
            Some(PerformanceQuote {
                ticker: quote.symbol.clone(),
                price: quote.price.clone(),
                as_of,
            })
        })
        .collect::<Vec<_>>();
    let rates = history
        .map(|h| {
            h.observations
                .iter()
                .map(|o| DailyCdiRate {
                    date: o.date.clone(),
                    value: o.value.clone(),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut calculation = calculate_fii_performance(snapshot, &quotes, &rates, today)
        .map_err(|error| error.to_string())?;
    for error in errors.into_iter().chain(panel.errors.iter().cloned()) {
        if !calculation.issues.contains(&error) {
            calculation.issues.push(error);
        }
    }
    let used = panel
        .items
        .iter()
        .filter_map(|i| i.observation.as_ref())
        .filter(|q| {
            quotes.iter().any(|accepted| accepted.ticker == q.symbol)
                && snapshot
                    .funds
                    .iter()
                    .any(|f| f.ticker.eq_ignore_ascii_case(&q.symbol))
        });
    let quote_fetched_at = used.clone().map(|q| q.fetched_at.clone()).min();
    let quote_source = used.map(|q| q.source.clone()).next();
    Ok(PerformanceReport {
        portfolio: calculate_portfolio_performance(snapshot, &calculation)
            .map_err(|e| e.to_string())?,
        calculation,
        quote_source,
        quote_fetched_at,
        cdi_source: history.map(|h| h.source.clone()),
        cdi_source_url: history.map(|h| h.source_url.clone()),
        cdi_fetched_at: history.map(|h| h.fetched_at.clone()),
    })
}

/// Query from the beginning of the first purchase month so that a purchase
/// estimated on a weekend still has a preceding published CDI observation.
/// The core accrues interest only after each modelled month-end purchase.
pub fn cdi_range(report: &PerformanceReport) -> Result<Option<(NaiveDate, NaiveDate)>, String> {
    if report.calculation.total_result.is_none() {
        return Ok(None);
    }
    let (Some(start), Some(end)) = (&report.calculation.start_month, &report.calculation.as_of)
    else {
        return Ok(None);
    };
    let start = NaiveDate::parse_from_str(&format!("{start}-01"), "%Y-%m-%d")
        .map_err(|_| "Mês inicial inválido na comparação.".to_string())?;
    let end = NaiveDate::parse_from_str(end, "%Y-%m-%d")
        .map_err(|_| "Data final inválida na comparação.".to_string())?;
    Ok(Some((start, end)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snapshot() -> PortfolioSnapshot {
        serde_json::from_value(json!({
            "funds": [{"id":"f", "ticker":"TEST11", "name":"Teste", "sector":"",
                "entries":[{"id":"r", "month":"2026-08", "cotasAdded":"10", "pricePerCota":"10",
                    "dividendPerCota":"0", "dividendTotal":"5", "notes":""}],
                "totalCotas":"10", "totalInvested":"100", "averagePrice":"10",
                "dividendsReceived":"5", "estimatedMonthlyIncome":"0"}],
            "totals":{"invested":"100","cotas":"10","dividendsReceived":"5","estimatedMonthlyIncome":"0"},
            "recordCount":1
        })).unwrap()
    }

    fn quote(currency: &str, time: &str) -> QuotePanel {
        serde_json::from_value(json!({"items":[{"symbol":"TEST11","status":"cached","error":null,
            "observation":{"symbol":"TEST11","price":"9","currency":currency,"quotedAt":time,
                "fetchedAt":"2026-10-20T12:00:00Z","source":"brapi","sourceUrl":"https://brapi.dev/","delayMinutes":30}}],
            "fetchedAt":"2026-10-20T12:00:00Z","credentialConfigured":false,"errors":[]
        })).unwrap()
    }

    #[test]
    fn market_date_uses_brasilia_not_fetch_time_and_keeps_fii_without_cdi() {
        let panel = quote("BRL", "2026-10-20T01:00:00Z");
        let report = build_report(
            &snapshot(),
            &panel,
            None,
            NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
            vec![],
        )
        .unwrap();
        assert_eq!(report.calculation.as_of.as_deref(), Some("2026-10-19"));
        assert_eq!(report.calculation.total_result.as_deref(), Some("-5"));
        assert!(report.calculation.cdi.is_none());
        assert_eq!(
            report.quote_fetched_at.as_deref(),
            Some("2026-10-20T12:00:00Z")
        );
        let (start, end) = cdi_range(&report).unwrap().unwrap();
        assert_eq!(start.to_string(), "2026-08-01");
        assert_eq!(end.to_string(), "2026-10-19");
    }

    #[test]
    fn foreign_currency_cannot_be_added_to_brl_purchase_cost() {
        let panel = quote("USD", "2026-10-19T21:00:00Z");
        let report = build_report(
            &snapshot(),
            &panel,
            None,
            NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
            vec![],
        )
        .unwrap();
        assert!(report.calculation.market_value.is_none());
        assert!(report.calculation.total_result.is_none());
        assert!(cdi_range(&report).unwrap().is_none());
        assert_eq!(report.calculation.invested, "100");
    }

    #[test]
    fn cash_profit_combines_with_complete_fii_result_even_without_cdi_history() {
        let mut value = serde_json::to_value(snapshot()).unwrap();
        value["cashPockets"] = json!([{
            "id":"cash", "name":"Reserva", "institution":"Teste", "balance":"220",
            "totalDeposits":"200", "totalWithdrawals":"10", "balanceDate":"2026-10-17",
            "cdiPercent":"120", "annualCdiRate":null, "rateDate":null, "notes":"",
            "createdAt":"2026-08-01T12:00:00Z", "updatedAt":"2026-10-18T12:00:00Z"
        }]);
        let snapshot = serde_json::from_value(value).unwrap();
        let report = build_report(
            &snapshot,
            &quote("BRL", "2026-10-19T21:00:00Z"),
            None,
            NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
            vec![],
        )
        .unwrap();
        assert!(report.calculation.cdi.is_none());
        assert_eq!(report.portfolio.invested.as_deref(), Some("300"));
        assert_eq!(report.portfolio.income.as_deref(), Some("35"));
        assert_eq!(report.portfolio.current_value.as_deref(), Some("310"));
        assert_eq!(report.portfolio.total_result.as_deref(), Some("25"));
        assert_eq!(
            report.portfolio.cash.items[0].balance_date.as_deref(),
            Some("2026-10-17")
        );
    }
}
