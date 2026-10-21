//! Read-only reconstruction inputs. Public caches are shared by the app and MCP;
//! no price, NAV, or derived result is written into the investor's ledger.
use crate::{
    fund_nav::{CvmNavService, FundNavPanel, FundNavRequest},
    historical_prices::{HistoricalPricePanel, HistoricalPriceRequest, HistoricalPriceService},
};
use chrono::{Datelike, NaiveDate};
use ivestfii_core::{
    historical_valuation::{
        FundNavObservation, HistoricalFixedIncomeSeries, HistoricalMarketPrice,
        HistoricalValuationData,
    },
    PortfolioSnapshot,
};
use serde::Serialize;
use std::{collections::BTreeSet, path::Path, time::Duration};

fn parse_nonnegative(value: &str, label: &str) -> Result<rust_decimal::Decimal, String> {
    let parsed = value
        .parse::<rust_decimal::Decimal>()
        .map_err(|_| format!("{label}: valor decimal inválido para consultar o histórico."))?;
    if parsed.is_sign_negative() {
        return Err(format!("{label}: o valor não pode ser negativo."));
    }
    Ok(parsed)
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalDataCoverage {
    pub status: String,
    pub price_observations: usize,
    pub price_pending: usize,
    pub nav_observations: usize,
    pub nav_pending_months: usize,
    pub issues: Vec<String>,
}

fn closed_month_end(today: NaiveDate) -> Result<NaiveDate, String> {
    today
        .with_day(1)
        .and_then(|day| day.pred_opt())
        .ok_or_else(|| "Périodo inválido para buscar o histórico.".into())
}

/// A purchase's calendar month suffices for end-of-month quantities. Its unknown
/// day is still kept unknown in the ledger and the matched-flow CDI assumption.
pub fn price_requests(
    snapshot: &PortfolioSnapshot,
    today: NaiveDate,
) -> Result<Vec<HistoricalPriceRequest>, String> {
    let end = closed_month_end(today)?;
    let last = end.year() * 12 + end.month0() as i32;
    let mut requests = Vec::new();
    for fund in &snapshot.funds {
        let first = fund
            .entries
            .iter()
            .filter_map(
                |entry| match parse_nonnegative(&entry.cotas_added, "Cotas compradas") {
                    Ok(quantity) if !quantity.is_zero() => Some(Ok(entry.month.as_str())),
                    Ok(_) => None,
                    Err(error) => Some(Err(error.to_string())),
                },
            )
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .min();
        let Some(first) = first else {
            continue;
        };
        let first = NaiveDate::parse_from_str(&format!("{first}-01"), "%Y-%m-%d")
            .map_err(|_| "Mês de compra inválido para buscar o histórico.".to_string())?;
        let start = first.year() * 12 + first.month0() as i32;
        if last - start > 1199 {
            return Err("O histórico solicitado excede o período suportado.".into());
        }
        for index in start..=last {
            requests.push(HistoricalPriceRequest {
                symbol: fund.ticker.clone(),
                month: format!("{:04}-{:02}", index / 12, index % 12 + 1),
                through_date: None,
            });
        }
    }
    requests.sort_by(|left, right| {
        right
            .month
            .cmp(&left.month)
            .then_with(|| left.symbol.cmp(&right.symbol))
    });
    requests.dedup_by(|left, right| left.symbol == right.symbol && left.month == right.month);
    if requests.len() > 2000 {
        return Err("A carteira excede o limite de consultas históricas por lote.".into());
    }
    Ok(requests)
}

pub fn nav_request(
    snapshot: &PortfolioSnapshot,
    today: NaiveDate,
) -> Result<Option<FundNavRequest>, String> {
    let Some(fixed) = snapshot
        .broker_position
        .as_ref()
        .and_then(|position| position.fixed_income.latest_observation.as_ref())
    else {
        return Ok(None);
    };
    let end = closed_month_end(today)?;
    let mut anchors = BTreeSet::new();
    for contribution in &fixed.contributions {
        let date = NaiveDate::parse_from_str(&contribution.application_date, "%Y-%m-%d").map_err(
            |_| "Data da aplicação inválida para consultar cotas históricas.".to_string(),
        )?;
        if date <= end
            && !parse_nonnegative(&contribution.invested, "Valor aplicado")
                .map_err(|error| error.to_string())?
                .is_zero()
        {
            anchors.insert(date.to_string());
        }
    }
    let Some(start) = anchors.first().cloned() else {
        return Ok(None);
    };
    Ok(Some(FundNavRequest {
        fund_name: fixed.name.clone(),
        cnpj: None,
        start_date: start,
        end_date: end.to_string(),
        anchor_dates: anchors.into_iter().collect(),
    }))
}

fn coverage(prices: &HistoricalPricePanel, nav: &FundNavPanel) -> HistoricalDataCoverage {
    let issues: BTreeSet<_> = prices
        .issues
        .iter()
        .chain(nav.issues.iter())
        .cloned()
        .collect();
    let pending = prices.pending_requests + nav.pending_months;
    HistoricalDataCoverage {
        status: if pending == 0 && issues.is_empty() {
            "ready"
        } else {
            "partial"
        }
        .into(),
        price_observations: prices.observations.len(),
        price_pending: prices.pending_requests,
        nav_observations: nav.observations.len(),
        nav_pending_months: nav.pending_months,
        issues: issues.into_iter().collect(),
    }
}

pub fn read_cache(
    snapshot: &PortfolioSnapshot,
    directory: &Path,
    today: NaiveDate,
) -> Result<(HistoricalValuationData, HistoricalDataCoverage), String> {
    let price_requests = price_requests(snapshot, today)?;
    let prices = HistoricalPriceService::new(directory.join("public-data/historical-prices.json"))?
        .read_cache(&price_requests)
        .unwrap_or_else(|error| HistoricalPricePanel {
            pending_requests: price_requests.len(),
            issues: vec![error],
            ..Default::default()
        });
    let nav = if let Some(request) = nav_request(snapshot, today)? {
        CvmNavService::new(directory.join("public-data/fund-nav.json"))?
            .read_cache(&request)
            .unwrap_or_else(|error| FundNavPanel {
                issues: vec![error],
                ..Default::default()
            })
    } else {
        FundNavPanel::default()
    };
    let status = coverage(&prices, &nav);
    let fixed_income = match (&nav.cnpj, &nav.fund_name) {
        (Some(cnpj), Some(fund_name)) => Some(HistoricalFixedIncomeSeries {
            cnpj: cnpj.clone(),
            fund_name: fund_name.clone(),
            observations: nav
                .observations
                .iter()
                .map(|row| FundNavObservation {
                    cnpj: row.cnpj.clone(),
                    date: row.date.clone(),
                    value: row.value.clone(),
                    source: row.source.clone(),
                    source_url: row.source_url.clone(),
                    fetched_at: row.fetched_at.clone(),
                })
                .collect(),
        }),
        _ => None,
    };
    let data = HistoricalValuationData {
        prices: prices
            .observations
            .into_iter()
            .map(|row| HistoricalMarketPrice {
                symbol: row.symbol,
                date: row.trade_date,
                raw_close: row.raw_close,
                source: row.source,
                source_url: row.source_url,
                fetched_at: row.fetched_at,
            })
            .collect(),
        fixed_income,
    };
    Ok((data, status))
}

/// Backfill the latest months first, then continue older missing batches. Each
/// service commits partial public results. Repeated failures or source limits
/// stop progress; no repeated request loop tries to evade a provider's budget.
pub async fn refresh_if_due(
    snapshot: &PortfolioSnapshot,
    directory: &Path,
    today: NaiveDate,
) -> Result<HistoricalDataCoverage, String> {
    let price_requests = price_requests(snapshot, today)?;
    let nav_request = nav_request(snapshot, today)?;
    let prices = HistoricalPriceService::new(directory.join("public-data/historical-prices.json"))?;
    let nav = CvmNavService::new(directory.join("public-data/fund-nav.json"))?;
    let work = async {
        let prices_work = async {
            for _ in 0..4 {
                let before = prices.read_cache(&price_requests)?;
                let outcome = prices.refresh_if_due(&price_requests).await;
                let Some(after) = outcome.data else {
                    break;
                };
                if after.pending_requests == 0 || after.pending_requests >= before.pending_requests
                {
                    break;
                }
            }
            Ok::<(), String>(())
        };
        let nav_work = async {
            if let Some(request) = &nav_request {
                for _ in 0..4 {
                    let before = nav.read_cache(request)?;
                    let outcome = nav.refresh_if_due(request).await;
                    let Some(after) = outcome.data else {
                        break;
                    };
                    if after.cnpj.is_none()
                        || after.pending_months == 0
                        || (before.cnpj.is_some() && after.pending_months >= before.pending_months)
                    {
                        break;
                    }
                }
            }
            Ok::<(), String>(())
        };
        let (left, right) = tokio::join!(prices_work, nav_work);
        left?;
        right?;
        Ok::<(), String>(())
    };
    let outcome = tokio::time::timeout(Duration::from_secs(180), work).await;
    let (_, mut status) = read_cache(snapshot, directory, today)?;
    match outcome {
        Ok(Ok(())) => {},
        Ok(Err(error)) => status.issues.push(error),
        Err(_) => status.issues.push("O histórico está sendo preenchido em lotes. Os dados já consultados foram salvos; os meses pendentes poderão continuar na próxima atualização.".into()),
    }
    if !status.issues.is_empty() {
        status.status = "partial".into();
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snapshot() -> PortfolioSnapshot {
        serde_json::from_value(json!({"funds":[{"id":"a","ticker":"ABCD11","name":"Teste","sector":"",
            "entries":[{"id":"a0","month":"2026-08","cotasAdded":"2000","pricePerCota":"10","dividendPerCota":"0","notes":""},
                {"id":"a1","month":"2026-09","cotasAdded":"0","pricePerCota":"0","dividendPerCota":"0","notes":""},
                {"id":"a2","month":"2026-12","cotasAdded":"5","pricePerCota":"10","dividendPerCota":"0","notes":""}],
            "totalCotas":"2005","totalInvested":"20050","averagePrice":"10","dividendsReceived":"0","estimatedMonthlyIncome":"0"}],
            "totals":{"invested":"20050","cotas":"2005","dividendsReceived":"0","estimatedMonthlyIncome":"0"},"recordCount":3})).unwrap()
    }

    #[test]
    fn price_planner_carries_holdings_through_closed_months_without_using_future_buys() {
        let original = snapshot();
        let before = serde_json::to_value(&original).unwrap();
        let requests =
            price_requests(&original, NaiveDate::from_ymd_opt(2026, 10, 21).unwrap()).unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].month, "2026-09");
        assert_eq!(requests[1].month, "2026-08");
        assert!(requests
            .iter()
            .all(|request| request.symbol == "ABCD11" && request.through_date.is_none()));
        assert_eq!(serde_json::to_value(&original).unwrap(), before);
    }

    #[test]
    fn missing_cache_read_preserves_filesystem_and_reports_backfill_pending() {
        let directory =
            std::env::temp_dir().join(format!("ivestfii-historical-read-{}", uuid::Uuid::new_v4()));
        let (data, status) = read_cache(
            &snapshot(),
            &directory,
            NaiveDate::from_ymd_opt(2026, 10, 21).unwrap(),
        )
        .unwrap();
        assert!(data.prices.is_empty());
        assert!(data.fixed_income.is_none());
        assert_eq!(status.price_pending, 2);
        assert_eq!(status.status, "partial");
        assert!(!directory.exists());
    }

    #[test]
    fn planner_rejects_negative_financial_inputs_without_provider_requests() {
        let mut original = snapshot();
        original.funds[0].entries[0].cotas_added = "-1".into();
        assert!(price_requests(&original, NaiveDate::from_ymd_opt(2026, 10, 21).unwrap()).is_err());
    }
}
