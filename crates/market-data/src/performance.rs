//! Read-only integration of ledger calculations and dated public observations.
use crate::{CdiHistorySnapshot, DividendPanel, QuotePanel};
use chrono::{DateTime, Datelike, Days, FixedOffset, NaiveDate};
use ivestfii_core::{
    historical_valuation::reconstruct_portfolio_evolution,
    performance::{
        calculate_fii_performance, calculate_fii_performance_with_income, DailyCdiRate,
        FiiPerformance, PerformanceQuote,
    },
    portfolio_benchmark::{calculate_portfolio_benchmark, PortfolioBenchmark},
    portfolio_evolution::{
        calculate_portfolio_evolution, latest_known_valuation, observe_portfolio,
        PortfolioEvolution, PortfolioHistoryObservation,
    },
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
    let dividends = crate::DividendService::new(directory.join("public-data/dividends.json"))?
        .read_cache(&symbols)
        .unwrap_or_else(|error| {
            errors.push(error);
            DividendPanel::default()
        });
    let report = build_report_with_income(
        snapshot,
        &panel,
        history.as_ref(),
        &dividends,
        today,
        errors,
    )?;
    Ok(attach_historical_cache(
        snapshot,
        directory,
        report,
        history.as_ref(),
        &dividends,
        today,
    ))
}

/// Attach derived historical points without changing today's result or creating
/// a captured account observation. MCP and desktop reads use this same path.
pub fn attach_historical_cache(
    snapshot: &PortfolioSnapshot,
    directory: &std::path::Path,
    mut report: PerformanceReport,
    cdi: Option<&CdiHistorySnapshot>,
    dividends: &DividendPanel,
    today: NaiveDate,
) -> PerformanceReport {
    match crate::historical_performance::read_cache(snapshot, directory, today) {
        Ok((history, mut coverage)) => {
            let rates = cdi
                .map(|history| {
                    history
                        .observations
                        .iter()
                        .map(|row| DailyCdiRate {
                            date: row.date.clone(),
                            value: row.value.clone(),
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if let Err(error) = reconstruct_portfolio_evolution(
                snapshot,
                &mut report.portfolio_evolution,
                &history,
                &crate::dividends::income_events(dividends),
                &rates,
                today,
            ) {
                coverage.status = "partial".into();
                coverage.issues.push(error.to_string());
                report.portfolio_evolution.issues.push(error.to_string());
            }
            report.historical_data = Some(coverage);
        }
        Err(error) => {
            report.historical_data = Some(crate::historical_performance::HistoricalDataCoverage {
                status: "unavailable".into(),
                issues: vec![error.clone()],
                ..Default::default()
            });
            report.portfolio_evolution.issues.push(error);
        }
    }
    report
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DividendPerformanceSource {
    pub symbol: String,
    pub status: String,
    pub source: String,
    pub source_url: String,
    pub fetched_at: String,
    pub coverage_warning: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceReport {
    #[serde(flatten)]
    pub calculation: FiiPerformance,
    pub portfolio: PortfolioPerformance,
    pub portfolio_benchmark: PortfolioBenchmark,
    pub portfolio_evolution: PortfolioEvolution,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub historical_data: Option<crate::historical_performance::HistoricalDataCoverage>,
    #[serde(skip_serializing)]
    pub history_observation: Option<PortfolioHistoryObservation>,
    pub quote_source: Option<String>,
    pub quote_fetched_at: Option<String>,
    pub cdi_source: Option<String>,
    pub cdi_source_url: Option<String>,
    pub cdi_fetched_at: Option<String>,
    pub dividend_source: Option<String>,
    pub dividend_fetched_at: Option<String>,
    pub dividend_sources: Vec<DividendPerformanceSource>,
}

pub fn build_report(
    snapshot: &PortfolioSnapshot,
    panel: &QuotePanel,
    history: Option<&CdiHistorySnapshot>,
    today: NaiveDate,
    errors: Vec<String>,
) -> Result<PerformanceReport, String> {
    build_report_inner(snapshot, panel, history, None, today, errors)
}

/// Use the same saved public events and manual precedence as the monthly chart.
/// API income is derived for this report, never inserted into the ledger.
pub fn build_report_with_income(
    snapshot: &PortfolioSnapshot,
    panel: &QuotePanel,
    history: Option<&CdiHistorySnapshot>,
    dividends: &DividendPanel,
    today: NaiveDate,
    errors: Vec<String>,
) -> Result<PerformanceReport, String> {
    build_report_inner(snapshot, panel, history, Some(dividends), today, errors)
}

fn build_report_inner(
    snapshot: &PortfolioSnapshot,
    panel: &QuotePanel,
    history: Option<&CdiHistorySnapshot>,
    dividends: Option<&DividendPanel>,
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
    let events = dividends
        .map(crate::dividends::income_events)
        .unwrap_or_default();
    let mut calculation = match dividends {
        Some(_) => calculate_fii_performance_with_income(snapshot, &quotes, &rates, &events, today),
        None => calculate_fii_performance(snapshot, &quotes, &rates, today),
    }
    .map_err(|error| error.to_string())?;
    for error in errors
        .into_iter()
        .chain(panel.errors.iter().cloned())
        .chain(
            dividends
                .into_iter()
                .flat_map(|panel| panel.errors.iter().cloned()),
        )
    {
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
    let dividend_sources = dividends
        .into_iter()
        .flat_map(|panel| panel.items.iter())
        .filter(|item| {
            snapshot
                .funds
                .iter()
                .any(|fund| fund.ticker.eq_ignore_ascii_case(&item.symbol))
        })
        .filter_map(|item| {
            let observation = item.observation.as_ref()?;
            Some(DividendPerformanceSource {
                symbol: item.symbol.clone(),
                status: item.status.clone(),
                source: observation.source.clone(),
                source_url: observation.source_url.clone(),
                fetched_at: observation.fetched_at.clone(),
                coverage_warning: observation.coverage_warning.clone(),
            })
        })
        .collect::<Vec<_>>();
    let dividend_fetched_at = dividend_sources
        .iter()
        .map(|source| source.fetched_at.clone())
        .min();
    let dividend_source = dividend_sources.first().map(|source| source.source.clone());
    let portfolio =
        calculate_portfolio_performance(snapshot, &calculation).map_err(|e| e.to_string())?;
    let portfolio_benchmark =
        calculate_portfolio_benchmark(snapshot, &calculation, &portfolio, &rates, today)
            .map_err(|e| e.to_string())?;
    let history_observation = observe_portfolio(
        snapshot,
        &calculation,
        &portfolio,
        &portfolio_benchmark,
        &events,
        today,
    )
    .map_err(|error| error.to_string())?;
    let mut portfolio_evolution = calculate_portfolio_evolution(
        snapshot,
        &rates,
        &events,
        &snapshot.performance_history,
        history_observation.as_ref(),
        today,
    )
    .map_err(|error| error.to_string())?;
    portfolio_evolution.latest_known =
        latest_known_valuation(&calculation, &portfolio, &portfolio_benchmark, today)
            .map_err(|error| error.to_string())?;
    Ok(PerformanceReport {
        portfolio,
        portfolio_evolution,
        historical_data: None,
        history_observation,
        portfolio_benchmark,
        calculation,
        quote_source,
        quote_fetched_at,
        cdi_source: history.map(|h| h.source.clone()),
        cdi_source_url: history.map(|h| h.source_url.clone()),
        cdi_fetched_at: history.map(|h| h.fetched_at.clone()),
        dividend_source,
        dividend_fetched_at,
        dividend_sources,
    })
}

/// Share one bounded historical query across all portfolio components. Query
/// from the first flow's month, with a prior-week lookback when that flow is
/// early in the month, so weekend/holiday applications have a preceding rate.
pub fn cdi_range(report: &PerformanceReport) -> Result<Option<(NaiveDate, NaiveDate)>, String> {
    let mut range: Option<(NaiveDate, NaiveDate)> = None;
    for component in &report.portfolio_benchmark.components {
        let (Some(start), Some(end)) = (&component.start_date, &component.as_of) else {
            continue;
        };
        let start = NaiveDate::parse_from_str(start, "%Y-%m-%d")
            .map_err(|_| "Data inicial inválida na comparação.".to_string())?;
        let month_start = start
            .with_day(1)
            .ok_or("Mês inicial inválido na comparação.")?;
        let preceding_week = start.checked_sub_days(Days::new(7)).unwrap_or(start);
        let series_start = NaiveDate::from_ymd_opt(1986, 3, 6).expect("SGS 12 start date");
        let start = month_start.min(preceding_week).max(series_start);
        let end = NaiveDate::parse_from_str(end, "%Y-%m-%d")
            .map_err(|_| "Data final inválida na comparação.".to_string())?;
        range = Some(match range {
            Some((first, last)) => (first.min(start), last.max(end)),
            None => (start, end),
        });
    }
    if let (Some(start), Some(end)) = (
        &report.portfolio_evolution.start_date,
        &report.portfolio_evolution.as_of,
    ) {
        let start = NaiveDate::parse_from_str(start, "%Y-%m-%d")
            .map_err(|_| "Data inicial inválida na evolução.".to_string())?;
        let start = start
            .with_day(1)
            .ok_or("Mês inicial inválido na evolução.")?
            .min(start.checked_sub_days(Days::new(7)).unwrap_or(start))
            .max(NaiveDate::from_ymd_opt(1986, 3, 6).expect("SGS start"));
        let end = NaiveDate::parse_from_str(end, "%Y-%m-%d")
            .map_err(|_| "Data final inválida na evolução.".to_string())?;
        range = Some(match range {
            Some((first, last)) => (first.min(start), last.max(end)),
            None => (start, end),
        });
    }
    Ok(range)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn historical_fixture(directory: &std::path::Path) -> PortfolioSnapshot {
        let mut value = serde_json::to_value(snapshot()).unwrap();
        value["funds"][0]["entries"][0]["dividendTotal"] = json!("2");
        value["funds"][0]["entries"].as_array_mut().unwrap().push(json!({
            "id":"r2","month":"2026-09","cotasAdded":"0","pricePerCota":"0","dividendPerCota":"0","dividendTotal":"3","notes":""
        }));
        value["funds"][0]["dividendsReceived"] = json!("5");
        value["recordCount"] = json!(2);
        value["brokerPosition"] = json!({"referenceDate":"2026-10-19","institution":"Teste","importedAt":"2026-10-19T12:00:00Z",
            "sourcePath":"fixture.json","sourceHash":"a".repeat(64),"warnings":[],
            "overview":{"totalWithCash":"99999","invested":"99999","cash":"0","listedFunds":"99999","fixedIncome":"99999","pendingIncome":"0"},
            "listedFunds":{"marketValue":"90","costBasis":"100","unrealizedResult":"-10","dailyResult":"0",
                "holdings":[{"ticker":"TEST11","shares":"10","averagePrice":"10","price":"9","marketValue":"90","unrealizedResult":"-10","costBasis":"100"}]},
            "fixedIncome":{"name":"Pós-fixado sintético","nameTruncated":false,"balance":"120","netBalance":"119","reportedEarnings":"20","return12Months":"9999","redemptionConversion":"D+0","redemptionSettlement":"D+0",
                "latestObservation":{"referenceDate":"2026-10-19","name":"Pós-fixado sintético","category":"Renda fixa","riskLevel":"1","balance":"120","invested":"100","totalEarnings":"20","earnings12Months":"9999","return12Months":"9999","returnReferenceDate":"2026-10-19",
                    "contributions":[{"applicationDate":"2026-08-03","invested":"100","currentValue":"120","earnings":"20","returnPercent":"20"}],"sources":[],"importedAt":"2026-10-19T12:00:00Z","sourcePath":"fixture.json","sourceHash":"b".repeat(64)}}});
        let fetched = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let public = directory.join("public-data");
        std::fs::create_dir_all(&public).unwrap();
        let mut items = serde_json::Map::new();
        let mut nav_months = serde_json::Map::new();
        for (month, end, price, nav_rows) in [
            (
                "2026-08",
                "2026-08-31",
                "9",
                vec![("2026-08-03", "1"), ("2026-08-31", "1.1")],
            ),
            ("2026-09", "2026-09-30", "11", vec![("2026-09-30", "1.2")]),
        ] {
            items.insert(format!("TEST11:{month}"), json!({
                "request":{"symbol":"TEST11","month":month},"requestedThrough":end,
                "observation":{"symbol":"TEST11","tradeDate":end,"rawClose":price,"currency":"BRL","source":"Dados B3 · COTAHIST",
                    "sourceUrl":format!("https://dadosb3.com/fiis/TEST11/desempenho?de={month}-01&ate={end}"),"fetchedAt":fetched,"corporateActionsChecked":false},
                "checkedAt":fetched,"available":true,"error":null
            }));
            nav_months.insert(month.into(), json!({"fetchedAt":fetched,"issues":[],"observations":nav_rows.into_iter().map(|(date,nav)|json!({
                "cnpj":"00332266000131","date":date,"value":nav,"source":"CVM · Informe Diário de Fundos",
                "sourceUrl":format!("https://dados.cvm.gov.br/dados/FI/DOC/INF_DIARIO/DADOS/inf_diario_fi_{}.zip",month.replace('-' ,"")),"fetchedAt":fetched
            })).collect::<Vec<_>>()}));
        }
        std::fs::write(
            public.join("historical-prices.json"),
            serde_json::to_vec(&json!({"version":1,"items":items,"retryAfter":null})).unwrap(),
        )
        .unwrap();
        std::fs::write(public.join("fund-nav.json"), serde_json::to_vec(&json!({"version":1,"register":{"fetchedAt":fetched,"funds":{"00332266000131":["Pós-fixado sintético"]}},"funds":{"00332266000131":nav_months}})).unwrap()).unwrap();
        let mut day = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
        let mut observations = Vec::new();
        while day <= end {
            observations.push(json!({"date":day.to_string(),"value":"0"}));
            day = day.succ_opt().unwrap();
        }
        std::fs::write(public.join("cdi-history.json"), serde_json::to_vec(&json!({"version":1,"snapshot":{
            "seriesId":12,"unit":"% a.d.","source":"Banco Central do Brasil · SGS 12",
            "sourceUrl":"https://api.bcb.gov.br/dados/serie/bcdata.sgs.12/dados?formato=json&dataInicial=01/08/2026&dataFinal=19/10/2026",
            "requestedStartDate":"2026-08-01","requestedEndDate":"2026-10-19","observations":observations,"fetchedAt":fetched,"stale":false
        }})).unwrap()).unwrap();
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn cached_report_reconstructs_entire_wallet_months_without_capturing_balances_or_changing_sources(
    ) {
        let directory = tempfile::tempdir().unwrap();
        let snapshot = historical_fixture(directory.path());
        let original = serde_json::to_value(&snapshot).unwrap();
        let paths = [
            "historical-prices.json",
            "fund-nav.json",
            "cdi-history.json",
        ];
        let before = paths
            .map(|name| std::fs::read(directory.path().join("public-data").join(name)).unwrap());
        let report = cached_report(
            &snapshot,
            directory.path(),
            NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
        )
        .unwrap();
        assert!(report.history_observation.is_none());
        assert!(report.portfolio_evolution.latest_known.is_none());
        let points = &report.portfolio_evolution.points;
        assert_eq!(points[0].portfolio_result.as_deref(), Some("2"));
        assert_eq!(points[1].portfolio_result.as_deref(), Some("35"));
        assert_eq!(points[1].invested.as_deref(), Some("200"));
        assert_eq!(points[1].cdi_result.as_deref(), Some("0"));
        assert_eq!(points[1].difference.as_deref(), Some("35"));
        assert!(points[2].portfolio_result.is_none());
        assert_eq!(points[1].provenance.as_ref().unwrap().components.len(), 2);
        let coverage = report.historical_data.as_ref().unwrap();
        assert_eq!(coverage.price_observations, 2);
        assert_eq!(coverage.nav_observations, 3);
        assert_eq!(coverage.status, "ready");
        assert_eq!(serde_json::to_value(&snapshot).unwrap(), original);
        for (index, name) in paths.iter().enumerate() {
            assert_eq!(
                std::fs::read(directory.path().join("public-data").join(name)).unwrap(),
                before[index]
            );
        }
        assert_eq!(
            std::fs::read_dir(directory.path().join("public-data"))
                .unwrap()
                .count(),
            3
        );
        let repeat = cached_report(
            &snapshot,
            directory.path(),
            NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&repeat).unwrap(),
            serde_json::to_value(&report).unwrap()
        );
    }

    #[test]
    fn bad_historical_price_cache_keeps_valid_nav_and_current_accounting_available() {
        let directory = tempfile::tempdir().unwrap();
        let snapshot = historical_fixture(directory.path());
        std::fs::write(
            directory.path().join("public-data/historical-prices.json"),
            b"not-json",
        )
        .unwrap();
        let report = cached_report(
            &snapshot,
            directory.path(),
            NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
        )
        .unwrap();
        assert_eq!(report.portfolio.fixed_income.income.as_deref(), Some("20"));
        let coverage = report.historical_data.as_ref().unwrap();
        assert_eq!(coverage.nav_observations, 3);
        assert_eq!(coverage.price_observations, 0);
        assert_eq!(coverage.price_pending, 2);
        assert_eq!(coverage.status, "partial");
        assert!(!coverage.issues.is_empty());
        assert!(report.portfolio_evolution.points[0]
            .portfolio_result
            .is_none());
        assert_eq!(
            std::fs::read(directory.path().join("public-data/historical-prices.json")).unwrap(),
            b"not-json"
        );
    }

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
        let latest = report.portfolio_evolution.latest_known.as_ref().unwrap();
        assert_eq!(latest.from_date, "2026-10-19");
        assert_eq!(latest.to_date, "2026-10-19");
        assert_eq!(latest.portfolio_result, "-5");
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
        let (start, end) = cdi_range(&report).unwrap().unwrap();
        assert_eq!(start.to_string(), "2026-08-01");
        assert_eq!(end.to_string(), "2026-10-20");
        assert!(report.history_observation.is_none());
        assert!(report.portfolio_evolution.latest_known.is_none());
        assert!(report
            .portfolio_evolution
            .points
            .iter()
            .all(|point| point.portfolio_result.is_none()));
        assert_eq!(report.calculation.invested, "100");
    }

    #[test]
    fn monthly_evolution_keeps_real_history_and_independent_cdi_without_serializing_capture_inputs()
    {
        let mut snapshot = snapshot();
        snapshot.funds[0].entries[0].purchase_date = Some("2026-08-03".into());
        let september = build_report(
            &snapshot,
            &quote("BRL", "2026-09-30T21:00:00Z"),
            None,
            NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
            vec![],
        )
        .unwrap();
        // A common real valuation remains useful while CDI publication is
        // pending. The private fingerprint must not become part of the report
        // delivered to the web view or MCP model.
        let observed = september.history_observation.as_ref().unwrap();
        assert_eq!(observed.portfolio_result, "-5");
        let last = september.portfolio_evolution.points.last().unwrap();
        assert_eq!(last.portfolio_result.as_deref(), Some("-5"));
        assert!(last.cdi_result.is_none());
        let serialized = serde_json::to_value(&september).unwrap();
        assert!(serialized.get("historyObservation").is_none());
        assert!(!serialized.to_string().contains("basisFingerprint"));
        assert_eq!(
            serialized["portfolioEvolution"]["latestKnown"]["portfolioResult"],
            "-5"
        );
        snapshot.performance_history.push(observed.clone());
        let original = serde_json::to_value(&snapshot).unwrap();

        let mut observations = vec![];
        let mut day = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
        while day <= end {
            observations.push(crate::cdi_history::CdiDailyObservation {
                date: day.to_string(),
                value: if day.to_string() == "2026-09-01" {
                    "1"
                } else {
                    "0"
                }
                .into(),
            });
            day = day.succ_opt().unwrap();
        }
        let history = CdiHistorySnapshot {
            series_id: 12,
            unit: "% a.d.".into(),
            source: "BCB sintético".into(),
            source_url: "https://api.bcb.gov.br/dados/serie/bcdata.sgs.12/dados".into(),
            requested_start_date: "2026-08-01".into(),
            requested_end_date: "2026-10-19".into(),
            observations,
            fetched_at: "2026-10-20T12:00:00Z".into(),
            stale: false,
        };
        let october = build_report(
            &snapshot,
            &quote("BRL", "2026-10-19T21:00:00Z"),
            Some(&history),
            NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
            vec![],
        )
        .unwrap();
        let points = &october.portfolio_evolution.points;
        assert_eq!(points.len(), 3);
        // No August valuation exists; a real zero return must not be invented
        // to make the wallet line follow the fully covered CDI scenario.
        assert_eq!(points[0].month, "2026-08");
        assert!(points[0].portfolio_result.is_none());
        assert_eq!(points[0].cdi_result.as_deref(), Some("0"));
        assert_eq!(points[1].month, "2026-09");
        assert_eq!(points[1].as_of, "2026-09-30");
        assert_eq!(points[1].portfolio_result.as_deref(), Some("-5"));
        assert_eq!(points[1].cdi_result.as_deref(), Some("1"));
        assert_eq!(points[2].month, "2026-10");
        assert_eq!(points[2].as_of, "2026-10-19");
        assert_eq!(points[2].portfolio_result.as_deref(), Some("-5"));
        assert_eq!(points[2].cdi_result.as_deref(), Some("1"));
        assert_eq!(points[2].difference.as_deref(), Some("-6"));
        assert_eq!(serde_json::to_value(&snapshot).unwrap(), original);

        // The benchmark remains available from dated deposits even when no
        // current quote can value the wallet.
        let unavailable = build_report(
            &snapshot,
            &QuotePanel::default(),
            Some(&history),
            end,
            vec![],
        )
        .unwrap();
        assert!(unavailable.history_observation.is_none());
        assert!(unavailable.portfolio_evolution.points[0]
            .portfolio_result
            .is_none());
        assert_eq!(
            unavailable.portfolio_evolution.points[1]
                .portfolio_result
                .as_deref(),
            Some("-5")
        );
        assert!(unavailable
            .portfolio_evolution
            .points
            .iter()
            .all(|point| point.cdi_result.is_some()));
    }

    #[test]
    fn latest_known_result_is_serialized_separately_when_component_dates_differ() {
        let mut value = serde_json::to_value(snapshot()).unwrap();
        value["cashPockets"] = json!([{
            "id":"cash-a","name":"Reserva de teste","institution":"Teste","balance":"220",
            "totalDeposits":"200","totalWithdrawals":"10","balanceDate":"2026-10-19",
            "cashFlows":[{"date":"2026-08-10","kind":"deposit","amount":"200"},
                {"date":"2026-09-10","kind":"withdrawal","amount":"10"}],
            "cdiPercent":"120","annualCdiRate":null,"rateDate":null,"notes":"",
            "createdAt":"2026-08-01T12:00:00Z","updatedAt":"2026-10-19T12:00:00Z"
        }]);
        let original: PortfolioSnapshot = serde_json::from_value(value).unwrap();
        let before = serde_json::to_value(&original).unwrap();
        let report = build_report(
            &original,
            &quote("BRL", "2026-10-20T21:00:00Z"),
            None,
            NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
            vec![],
        )
        .unwrap();
        assert_eq!(report.portfolio.total_result.as_deref(), Some("25"));
        assert!(report.history_observation.is_none());
        assert!(report.portfolio_benchmark.cdi.is_none());
        assert!(report
            .portfolio_evolution
            .points
            .iter()
            .all(|point| point.portfolio_result.is_none()));
        let serialized = serde_json::to_value(&report).unwrap();
        let latest = &serialized["portfolioEvolution"]["latestKnown"];
        assert_eq!(latest["fromDate"], "2026-10-19");
        assert_eq!(latest["toDate"], "2026-10-20");
        assert_eq!(latest["invested"], "300");
        assert_eq!(latest["portfolioResult"], "25");
        assert_eq!(latest["components"][0]["kind"], "fiis");
        assert_eq!(latest["components"][0]["asOf"], "2026-10-20");
        assert_eq!(latest["components"][1]["kind"], "cash");
        assert_eq!(latest["components"][1]["asOf"], "2026-10-19");
        assert!(latest.get("cdiResult").is_none());
        assert!(latest.get("difference").is_none());
        assert!(serialized.get("historyObservation").is_none());
        assert!(!serialized.to_string().contains("basisFingerprint"));
        assert_eq!(serde_json::to_value(&original).unwrap(), before);
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

    #[test]
    fn cash_only_cached_report_uses_daily_history_and_requests_its_dated_flows() {
        let directory =
            std::env::temp_dir().join(format!("ivestfii-cash-cdi-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(directory.join("public-data")).unwrap();
        let value: PortfolioSnapshot = serde_json::from_value(json!({
            "funds":[], "totals":{"invested":"0","cotas":"0","dividendsReceived":"0","estimatedMonthlyIncome":"0"},
            "recordCount":0,"cashPockets":[{
                "id":"cash","name":"Reserva sintética","institution":"Teste","balance":"115",
                "totalDeposits":"100","totalWithdrawals":"10","balanceDate":"2026-10-20",
                "cashFlows":[{"date":"2026-10-19","kind":"deposit","amount":"100"},
                    {"date":"2026-10-20","kind":"withdrawal","amount":"10"}],
                "cdiPercent":"120","annualCdiRate":null,"rateDate":null,"rateSource":"manual","notes":"",
                "createdAt":"2026-10-19T12:00:00Z","updatedAt":"2026-10-20T12:00:00Z"
            }]
        })).unwrap();
        let before = serde_json::to_value(&value).unwrap();
        let history = json!({"version":1,"snapshot":{
            "seriesId":12,"unit":"% a.d.","source":"Banco Central do Brasil · SGS 12",
            "sourceUrl":"https://api.bcb.gov.br/dados/serie/bcdata.sgs.12/dados?formato=json&dataInicial=01/10/2026&dataFinal=20/10/2026",
            "requestedStartDate":"2026-10-01","requestedEndDate":"2026-10-20",
            "observations":[
                {"date":"2026-10-01","value":"0"},{"date":"2026-10-05","value":"0"},
                {"date":"2026-10-09","value":"0"},{"date":"2026-10-13","value":"0"},
                {"date":"2026-10-16","value":"0"},{"date":"2026-10-19","value":"1"},
                {"date":"2026-10-20","value":"2"}],
            "fetchedAt":"2026-10-20T23:00:00Z","stale":false
        }});
        std::fs::write(
            directory.join("public-data/cdi-history.json"),
            serde_json::to_vec(&history).unwrap(),
        )
        .unwrap();
        let report = cached_report(
            &value,
            &directory,
            NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
        )
        .unwrap();
        assert_eq!(
            report.portfolio_benchmark.status, "available",
            "{:?}",
            report.portfolio_benchmark
        );
        assert_eq!(report.portfolio_benchmark.result.as_deref(), Some("25"));
        assert_eq!(report.portfolio_benchmark.cdi.as_ref().unwrap().result, "2");
        assert_eq!(
            report.cdi_source.as_deref(),
            Some("Banco Central do Brasil · SGS 12")
        );
        let (start, end) = cdi_range(&report).unwrap().unwrap();
        assert_eq!(start.to_string(), "2026-10-01");
        assert_eq!(end.to_string(), "2026-10-20");
        assert_eq!(serde_json::to_value(&value).unwrap(), before);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn saved_b3_income_flows_into_full_result_cash_totals_and_cdi_without_manual_duplicates() {
        use chrono::{Days, Utc};
        let mut value = serde_json::to_value(snapshot()).unwrap();
        value["cashPockets"] = json!([{
            "id":"cash", "name":"Reserva", "institution":"Teste", "balance":"220",
            "totalDeposits":"200", "totalWithdrawals":"10", "balanceDate":"2026-10-17",
            "cdiPercent":"120", "annualCdiRate":null, "rateDate":null, "notes":"",
            "createdAt":"2026-08-01T12:00:00Z", "updatedAt":"2026-10-18T12:00:00Z"
        }]);
        let snapshot: PortfolioSnapshot = serde_json::from_value(value).unwrap();
        let original = serde_json::to_value(&snapshot).unwrap();
        let fetched = DateTime::parse_from_rfc3339("2026-10-20T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let observation = crate::dividends::parse_b3_dividends(
            &serde_json::to_vec(&json!({"code":"TEST","cashDividends":[
                {"isinCode":"BRTESTCTF000","label":"RENDIMENTO","rate":"9,99",
                    "lastDatePrior":"2026-08-01","paymentDate":"2026-08-15"},
                {"isinCode":"BRTESTCTF000","label":"RENDIMENTO","rate":"1,00",
                    "lastDatePrior":"2026-09-01","paymentDate":"2026-09-15"},
                {"isinCode":"BRTESTCTF000","label":"RENDIMENTO","rate":"2,00",
                    "lastDatePrior":"2026-10-01","paymentDate":"2026-10-20"}
            ]}))
            .unwrap(),
            "TEST11",
            fetched,
        )
        .unwrap();
        let source_url = observation.source_url.clone();
        let dividends = DividendPanel {
            items: vec![crate::dividends::DividendItem {
                symbol: "TEST11".into(),
                status: "cached".into(),
                observation: Some(observation),
                error: None,
            }],
            fetched_at: Some(fetched.to_rfc3339()),
            errors: vec![],
        };
        let today = NaiveDate::from_ymd_opt(2026, 10, 20).unwrap();
        let mut day = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let mut observations = vec![];
        while day < today {
            observations.push(crate::cdi_history::CdiDailyObservation {
                date: day.to_string(),
                value: "0".into(),
            });
            day = day.checked_add_days(Days::new(1)).unwrap();
        }
        let history = CdiHistorySnapshot {
            series_id: 12,
            unit: "% a.d.".into(),
            source: "BCB sintético".into(),
            source_url: "https://api.bcb.gov.br/dados/serie/bcdata.sgs.12/dados".into(),
            requested_start_date: "2026-08-01".into(),
            requested_end_date: "2026-10-19".into(),
            observations,
            fetched_at: fetched.to_rfc3339(),
            stale: false,
        };
        let report = build_report_with_income(
            &snapshot,
            &quote("BRL", "2026-10-19T21:00:00Z"),
            Some(&history),
            &dividends,
            today,
            vec![],
        )
        .unwrap();
        // August's local total wins over the API; October's payment is after
        // the common quote cutoff, so only September's 10 eligible shares count.
        assert_eq!(report.calculation.dividends, "15");
        assert_eq!(report.calculation.total_result.as_deref(), Some("5"));
        assert_eq!(report.calculation.difference.as_deref(), Some("5"));
        assert_eq!(report.calculation.cdi.as_ref().unwrap().result, "0");
        let income = report.calculation.income.as_ref().unwrap();
        assert_eq!(income.recorded_amount, "5");
        assert_eq!(income.api_amount, "10");
        assert_eq!(income.as_of, "2026-10-19");
        assert_eq!(report.portfolio.income.as_deref(), Some("45"));
        assert_eq!(report.portfolio.total_result.as_deref(), Some("35"));
        assert_eq!(report.dividend_source.as_deref(), Some("b3"));
        assert_eq!(report.dividend_sources[0].source_url, source_url);
        assert_eq!(
            report.dividend_sources[0].fetched_at,
            fetched.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        );
        assert_eq!(serde_json::to_value(&snapshot).unwrap(), original);
    }

    #[test]
    fn first_day_weekend_application_requests_a_prior_week_cdi_observation() {
        let snapshot: PortfolioSnapshot = serde_json::from_value(json!({
            "funds":[],"totals":{"invested":"0","cotas":"0","dividendsReceived":"0","estimatedMonthlyIncome":"0"},
            "recordCount":0,"cashPockets":[{
                "id":"cash","name":"Reserva sintética","institution":"Teste","balance":"101",
                "totalDeposits":"100","totalWithdrawals":"0","balanceDate":"2026-11-03",
                "cashFlows":[{"date":"2026-11-01","kind":"deposit","amount":"100"}],
                "cdiPercent":"120","annualCdiRate":null,"rateDate":null,"notes":"Sintético",
                "createdAt":"2026-11-01T12:00:00Z","updatedAt":"2026-11-03T12:00:00Z"
            }]
        })).unwrap();
        let report = build_report(
            &snapshot,
            &QuotePanel::default(),
            None,
            NaiveDate::from_ymd_opt(2026, 11, 3).unwrap(),
            vec![],
        )
        .unwrap();
        let (start, end) = cdi_range(&report).unwrap().unwrap();
        assert_eq!(start.to_string(), "2026-10-25");
        assert_eq!(end.to_string(), "2026-11-03");
        assert_eq!(
            report.portfolio_benchmark.start_date.as_deref(),
            Some("2026-11-01")
        );
        // The lookback is a query boundary, never an invented earlier deposit.
        assert_eq!(report.portfolio_benchmark.invested.as_deref(), Some("100"));
        let mut boundary = report;
        boundary.portfolio_benchmark.components[0].start_date = Some("1986-03-06".into());
        let (start, _) = cdi_range(&boundary).unwrap().unwrap();
        assert_eq!(start.to_string(), "1986-03-06");
    }

    #[test]
    fn whole_wallet_report_extends_cdi_to_fixed_applications_and_reads_history_without_fiis() {
        use chrono::{Days, SecondsFormat, Utc};
        let mut value = serde_json::to_value(snapshot()).unwrap();
        value["cashPockets"] = json!([{
            "id":"cash","name":"Reserva sintética","institution":"Teste","balance":"220",
            "totalDeposits":"200","totalWithdrawals":"10","balanceDate":"2026-10-19",
            "cashFlows":[{"date":"2026-08-10","kind":"deposit","amount":"200"},
                {"date":"2026-09-10","kind":"withdrawal","amount":"10"}],
            "cdiPercent":"120","annualCdiRate":null,"rateDate":null,"notes":"Sintético",
            "createdAt":"2026-08-01T12:00:00Z","updatedAt":"2026-10-19T12:00:00Z"
        }]);
        value["brokerPosition"] = json!({
            "referenceDate":"2026-10-19","institution":"Corretora sintética",
            "importedAt":"2026-10-19T12:00:00Z","sourcePath":"fixture.json","sourceHash":"a".repeat(64),
            "overview":{"totalWithCash":"99999","invested":"99999","cash":"99999","listedFunds":"99999","fixedIncome":"99999","pendingIncome":"99999"},
            "listedFunds":{"marketValue":"90","costBasis":"100","unrealizedResult":"-10","dailyResult":"0",
                "holdings":[{"ticker":"TEST11","shares":"10","averagePrice":"10","price":"9","marketValue":"90","unrealizedResult":"-10","costBasis":"100"}]},
            "fixedIncome":{"name":"Nome antigo sintético","nameTruncated":true,"balance":"99999","netBalance":"99999","reportedEarnings":"99999","return12Months":"99999","redemptionConversion":"D+0","redemptionSettlement":"D+0",
                "latestObservation":{"referenceDate":"2026-10-19","name":"Pós-fixado sintético","category":"Renda fixa","riskLevel":"1",
                    "balance":"180","invested":"150","totalEarnings":"30","earnings12Months":"99999","return12Months":"99999","returnReferenceDate":"2026-10-15",
                    "contributions":[
                        {"applicationDate":"2026-07-10","invested":"100","currentValue":"120","earnings":"20","returnPercent":"20"},
                        {"applicationDate":"2026-09-01","invested":"50","currentValue":"60","earnings":"10","returnPercent":"20"}],
                    "sources":[],"importedAt":"2026-10-19T12:00:00Z","sourcePath":"fixture.json","sourceHash":"b".repeat(64)}},
            "warnings":[]
        });
        let snapshot: PortfolioSnapshot = serde_json::from_value(value).unwrap();
        let before = serde_json::to_value(&snapshot).unwrap();
        let fetched = DateTime::parse_from_rfc3339("2026-10-20T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let observation = crate::dividends::parse_b3_dividends(
            &serde_json::to_vec(&json!({"code":"TEST","cashDividends":[
                {"isinCode":"BRTESTCTF000","label":"RENDIMENTO","rate":"99,99","lastDatePrior":"2026-08-01","paymentDate":"2026-08-15"},
                {"isinCode":"BRTESTCTF000","label":"RENDIMENTO","rate":"1,00","lastDatePrior":"2026-09-01","paymentDate":"2026-09-15"}
            ]})).unwrap(), "TEST11", fetched,
        ).unwrap();
        let dividends = DividendPanel {
            items: vec![crate::dividends::DividendItem {
                symbol: "TEST11".into(),
                status: "cached".into(),
                observation: Some(observation),
                error: None,
            }],
            fetched_at: Some(fetched.to_rfc3339()),
            errors: vec![],
        };
        let mut day = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
        let mut observations = vec![];
        while day <= end {
            observations.push(crate::cdi_history::CdiDailyObservation {
                date: day.to_string(),
                value: if day.to_string() == "2026-09-01" {
                    "1"
                } else {
                    "0"
                }
                .into(),
            });
            day = day.checked_add_days(Days::new(1)).unwrap();
        }
        let history = CdiHistorySnapshot {
            series_id:12, unit:"% a.d.".into(), source:"Banco Central do Brasil · SGS 12".into(),
            source_url:"https://api.bcb.gov.br/dados/serie/bcdata.sgs.12/dados?formato=json&dataInicial=01/07/2026&dataFinal=19/10/2026".into(),
            requested_start_date:"2026-07-01".into(), requested_end_date:"2026-10-19".into(),
            observations,fetched_at:fetched.to_rfc3339_opts(SecondsFormat::Secs,true),stale:false,
        };
        let today = NaiveDate::from_ymd_opt(2026, 10, 20).unwrap();
        let report = build_report_with_income(
            &snapshot,
            &quote("BRL", "2026-10-19T21:00:00Z"),
            Some(&history),
            &dividends,
            today,
            vec![],
        )
        .unwrap();
        let (start, end) = cdi_range(&report).unwrap().unwrap();
        assert_eq!(start.to_string(), "2026-07-01");
        assert_eq!(end.to_string(), "2026-10-19");
        assert_eq!(report.calculation.start_month.as_deref(), Some("2026-08"));
        let income = report.calculation.income.as_ref().unwrap();
        assert_eq!(income.recorded_amount, "5");
        assert_eq!(income.api_amount, "10");
        assert_eq!(report.portfolio.income.as_deref(), Some("75"));
        assert_eq!(report.portfolio_benchmark.status, "available");
        assert_eq!(report.portfolio_benchmark.invested.as_deref(), Some("450"));
        assert_eq!(report.portfolio_benchmark.result.as_deref(), Some("65"));
        assert_eq!(report.portfolio_benchmark.cdi.as_ref().unwrap().result, "4");
        assert_eq!(report.portfolio_benchmark.difference.as_deref(), Some("61"));
        let observed = report.history_observation.as_ref().unwrap();
        assert_eq!(observed.invested, "450");
        assert_eq!(observed.portfolio_result, "65");
        let current = report.portfolio_evolution.points.last().unwrap();
        assert_eq!(current.portfolio_result.as_deref(), Some("65"));
        assert_eq!(current.cdi_result.as_deref(), Some("4"));
        assert_eq!(current.difference.as_deref(), Some("61"));
        // August's manual amount masks its API event; September's API amount,
        // post-fixed gain and pocket gain each appear exactly once.
        assert!(report.portfolio_evolution.points[..3]
            .iter()
            .all(|point| point.portfolio_result.is_none()));
        assert_eq!(serde_json::to_value(&snapshot).unwrap(), before);

        let directory =
            std::env::temp_dir().join(format!("ivestfii-fixed-cdi-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(directory.join("public-data")).unwrap();
        std::fs::write(
            directory.join("public-data/cdi-history.json"),
            serde_json::to_vec(&json!({"version":1,"snapshot":history})).unwrap(),
        )
        .unwrap();
        let mut fixed_only = snapshot.clone();
        fixed_only.funds.clear();
        fixed_only.record_count = 0;
        fixed_only.cash_pockets.clear();
        fixed_only
            .broker_position
            .as_mut()
            .unwrap()
            .listed_funds
            .holdings
            .clear();
        let report = cached_report(&fixed_only, &directory, today).unwrap();
        assert_eq!(report.calculation.status, "empty");
        assert_eq!(
            report.portfolio_benchmark.status, "available",
            "{:?}",
            report.portfolio_benchmark
        );
        assert_eq!(report.portfolio_benchmark.components.len(), 1);
        assert_eq!(report.portfolio_benchmark.result.as_deref(), Some("30"));
        assert_eq!(report.portfolio_benchmark.cdi.as_ref().unwrap().result, "1");
        assert_eq!(
            report.cdi_source.as_deref(),
            Some("Banco Central do Brasil · SGS 12")
        );
        let (start, end) = cdi_range(&report).unwrap().unwrap();
        assert_eq!(start.to_string(), "2026-07-01");
        assert_eq!(end.to_string(), "2026-10-19");
        std::fs::remove_dir_all(directory).unwrap();
    }
}
