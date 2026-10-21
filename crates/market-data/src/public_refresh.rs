//! Background public context only. The captured portfolio is never written here.
use std::{path::Path, time::Duration};

use crate::{
    CdiHistoryService, CvmFundRequest, CvmPanel, CvmService, DividendPanel, DividendService,
    EconomicPanel, EconomicService, NewsService, NewsSnapshot, QuotePanel, QuoteService,
    RefreshOutcome,
};
use chrono::{NaiveDate, SecondsFormat, Utc};
use ivestfii_core::PortfolioSnapshot;
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicDataRefresh {
    pub economics: Option<EconomicPanel>,
    pub news: Option<NewsSnapshot>,
    pub quotes: QuotePanel,
    pub cvm: CvmPanel,
    pub dividends: DividendPanel,
    pub sources: Vec<SourceRefresh>,
    pub checked_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRefresh {
    pub id: &'static str,
    pub status: String,
    pub message: Option<String>,
}

fn source<T>(id: &'static str, outcome: &RefreshOutcome<T>) -> SourceRefresh {
    SourceRefresh {
        id,
        status: outcome.status.clone(),
        message: outcome.message.clone(),
    }
}

fn unavailable<T>(data: Option<T>, message: String) -> RefreshOutcome<T> {
    let status = if data.is_some() {
        "cached"
    } else {
        "unavailable"
    };
    RefreshOutcome {
        data,
        status: status.into(),
        message: Some(message),
    }
}

/// Only existing cache services can write public-data files. All remote work is
/// bounded, independent sources run together, and one failure keeps the others.
pub struct PublicDataRequest<'a> {
    pub snapshot: &'a PortfolioSnapshot,
    pub symbols: &'a [String],
    pub funds: &'a [CvmFundRequest],
    pub key: Result<Option<String>, String>,
    pub today: NaiveDate,
}

pub async fn refresh_if_due(
    directory: &Path,
    economics: &EconomicService,
    news: &NewsService,
    request: PublicDataRequest<'_>,
) -> Result<PublicDataRefresh, String> {
    let PublicDataRequest {
        snapshot,
        symbols,
        funds,
        key,
        today,
    } = request;
    let quotes = QuoteService::new(directory.join("public-data/quotes.json"));
    let cvm = CvmService::new(directory.join("public-data/cvm-funds.json"));
    let dividends = DividendService::new(directory.join("public-data/dividends.json"));
    let quote_refresh = async {
        if symbols.is_empty() {
            return RefreshOutcome {
                data: Some(QuotePanel::default()),
                status: "cached".into(),
                message: None,
            };
        }
        let quotes = match &quotes {
            Ok(service) => service,
            Err(error) => return unavailable(None, error.clone()),
        };
        match key.as_ref() {
            Ok(key) => tokio::time::timeout(
                Duration::from_secs(45), quotes.refresh_if_due(symbols, key.as_deref()),
            ).await.unwrap_or_else(|_| unavailable(
                quotes.read_cache(symbols).ok(),
                "A consulta de cotações demorou demais. As cotações salvas foram mantidas.".into(),
            )),
            Err(_) => unavailable(
                quotes.read_cache(symbols).ok(),
                "Não foi possível acessar a chave brapi neste computador. Confira as configurações.".into(),
            ),
        }
    };
    let cvm_refresh = async {
        if funds.is_empty() {
            return RefreshOutcome {
                data: Some(CvmPanel::default()),
                status: "cached".into(),
                message: None,
            };
        }
        let cvm = match &cvm {
            Ok(service) => service,
            Err(error) => return unavailable(None, error.clone()),
        };
        tokio::time::timeout(Duration::from_secs(45), cvm.refresh_if_due(funds))
            .await
            .unwrap_or_else(|_| {
                unavailable(
                    cvm.read_cache(funds).ok(),
                    "A consulta CVM demorou demais. Os informes salvos foram mantidos.".into(),
                )
            })
    };
    let dividend_refresh = async {
        if symbols.is_empty() {
            return RefreshOutcome {
                data: Some(DividendPanel::default()),
                status: "cached".into(),
                message: None,
            };
        }
        let service = match &dividends {
            Ok(service) => service,
            Err(error) => return unavailable(None, error.clone()),
        };
        tokio::time::timeout(Duration::from_secs(45), service.refresh_if_due(symbols))
            .await
            .unwrap_or_else(|_| {
                unavailable(
                    service.read_cache(symbols).ok(),
                    "A consulta de proventos demorou demais. Os eventos salvos foram mantidos."
                        .into(),
                )
            })
    };
    let historical_refresh = async {
        let before = crate::historical_performance::read_cache(snapshot, directory, today)
            .ok()
            .map(|(_, coverage)| coverage);
        match crate::historical_performance::refresh_if_due(snapshot, directory, today).await {
            Ok(after) => {
                let changed = before.as_ref().is_none_or(|before| {
                    before.price_observations != after.price_observations
                        || before.nav_observations != after.nav_observations
                        || before.price_pending != after.price_pending
                        || before.nav_pending_months != after.nav_pending_months
                });
                let pending = after.price_pending + after.nav_pending_months;
                let message = if !after.issues.is_empty() {
                    Some(after.issues.join(" "))
                } else if pending > 0 {
                    Some(format!("Histórico em preenchimento: {} cotações e {} meses de cotas ainda pendentes. O progresso consultado foi salvo.", after.price_pending, after.nav_pending_months))
                } else {
                    None
                };
                SourceRefresh {
                    id: "historicalValuations",
                    status: if changed {
                        "updated"
                    } else if after.price_observations + after.nav_observations > 0
                        || after.status == "ready"
                    {
                        "cached"
                    } else {
                        "unavailable"
                    }
                    .into(),
                    message,
                }
            }
            Err(error) => SourceRefresh {
                id: "historicalValuations",
                status: "unavailable".into(),
                message: Some(error),
            },
        }
    };
    let income_refresh = async {
        match crate::income_history::refresh_if_due(snapshot, directory, today).await {
            Ok(coverage) => SourceRefresh {
                id: "historicalIncome",
                status: if coverage.event_count > 0 || coverage.status == "ready" { "cached" } else { "unavailable" }.into(),
                message: if !coverage.issues.is_empty() {
                    Some(coverage.issues.join(" "))
                } else if coverage.pending_documents > 0 {
                    Some(format!("Proventos antigos: {} avisos oficiais ainda aguardam consulta. Os dados já conferidos foram salvos.", coverage.pending_documents))
                } else { None },
            },
            Err(error) => SourceRefresh { id: "historicalIncome", status: "unavailable".into(), message: Some(error) },
        }
    };
    let (economics, news, mut quotes, cvm, dividends, historical, income) = tokio::join!(
        economics.refresh_if_due(),
        news.refresh_if_due(),
        quote_refresh,
        cvm_refresh,
        dividend_refresh,
        historical_refresh,
        income_refresh,
    );
    if let Some(panel) = quotes.data.as_mut() {
        panel.set_credential_presence(key.as_ref().is_ok_and(|key| key.is_some()));
    }
    let mut sources = vec![
        source("economics", &economics),
        source("news", &news),
        source("quotes", &quotes),
        source("cvm", &cvm),
        source("dividends", &dividends),
        historical,
        income,
    ];
    let quote_panel = quotes.data.unwrap_or_default();
    let dividend_panel = dividends.data.unwrap_or_default();
    // One CDI range covers dated FII purchases, fixed-income applications and
    // cash movements. Reuse quotes without spending a second brapi batch.
    let range = crate::performance::build_report_with_income(
        snapshot,
        &quote_panel,
        None,
        &dividend_panel,
        today,
        vec![],
    )
    .and_then(|report| crate::performance::cdi_range(&report));
    match range {
        Ok(Some((start, end))) => {
            let result =
                match CdiHistoryService::new(directory.join("public-data/cdi-history.json")) {
                    Ok(history) => history.refresh_if_due(start, end).await,
                    Err(error) => unavailable(None, error),
                };
            sources.push(source("cdiHistory", &result));
        }
        Ok(None) => sources.push(SourceRefresh {
            id: "cdiHistory",
            status: "cached".into(),
            // The performance panel explains any missing inputs itself.
            message: None,
        }),
        Err(error) => sources.push(SourceRefresh {
            id: "cdiHistory",
            status: "unavailable".into(),
            message: Some(error),
        }),
    }
    Ok(PublicDataRefresh {
        economics: economics.data,
        news: news.data,
        quotes: quote_panel,
        cvm: cvm.data.unwrap_or_default(),
        dividends: dividend_panel,
        sources,
        checked_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
    })
}
