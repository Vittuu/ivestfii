//! Shared, read-only income inputs for the overview, monthly chart and MCP.
//! Historical public evidence never becomes a financial ledger entry.
use std::{collections::BTreeSet, path::Path, time::Duration};

use chrono::{Months, NaiveDate};
use ivestfii_core::{income::IncomeEvent, PortfolioSnapshot};
use rust_decimal::Decimal;
use serde::Serialize;

use crate::{
    dividends::{self, DividendPanel},
    historical_dividends::{
        self, HistoricalDividendCoverage, HistoricalDividendPanel, HistoricalDividendRequest,
        HistoricalDividendService,
    },
    MarketSettingsStore,
};

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalIncomeCoverage {
    pub status: String,
    pub event_count: usize,
    pub pending_documents: usize,
    pub sources: Vec<HistoricalDividendCoverage>,
    pub issues: Vec<String>,
}

pub struct CachedIncome {
    pub events: Vec<IncomeEvent>,
    pub coverage: HistoricalIncomeCoverage,
    pub historical: HistoricalDividendPanel,
}

/// Only the public fund identity and a calendar interval reach Fundos.NET.
/// Include the previous month because the first payment can be announced before
/// the first eligible holding month. Amounts and purchase days stay local.
pub fn requests(
    snapshot: &PortfolioSnapshot,
    directory: &Path,
    today: NaiveDate,
) -> Result<(Vec<HistoricalDividendRequest>, Vec<String>), String> {
    let (mappings, _) = MarketSettingsStore::new(directory)?.mappings()?;
    let mut requests = Vec::new();
    let mut issues = Vec::new();
    for fund in &snapshot.funds {
        let mut first = None::<&str>;
        for entry in &fund.entries {
            let quantity = entry.cotas_added.parse::<Decimal>().map_err(|_| {
                "Quantidade inválida para consultar os proventos antigos.".to_string()
            })?;
            if quantity.is_sign_negative() {
                return Err("Quantidade negativa no histórico de compras.".into());
            }
            if !quantity.is_zero() && first.is_none_or(|value| entry.month.as_str() < value) {
                first = Some(&entry.month);
            }
        }
        let Some(first) = first else {
            continue;
        };
        let start = NaiveDate::parse_from_str(&format!("{first}-01"), "%Y-%m-%d")
            .map_err(|_| "Mês de compra inválido para consultar proventos antigos.".to_string())?;
        if start > today {
            continue;
        }
        let symbol = crate::quotes::normalize_symbol(&fund.ticker)?;
        let Some(cnpj) = mappings.get(&symbol) else {
            issues.push(format!(
                "{symbol}: falta o vínculo CNPJ para consultar os avisos oficiais antigos."
            ));
            continue;
        };
        requests.push(HistoricalDividendRequest {
            symbol,
            cnpj: cnpj.clone(),
            isin: None,
            start_date: start
                .checked_sub_months(Months::new(1))
                .unwrap_or(start)
                .to_string(),
            end_date: today.to_string(),
        });
    }
    requests.sort_by(|a, b| a.symbol.cmp(&b.symbol));
    Ok((requests, issues))
}

fn coverage(
    panel: &HistoricalDividendPanel,
    planning_issues: Vec<String>,
) -> HistoricalIncomeCoverage {
    let issues: BTreeSet<_> = panel
        .issues
        .iter()
        .cloned()
        .chain(planning_issues)
        .collect();
    let complete = panel.coverage.iter().all(|source| source.complete);
    HistoricalIncomeCoverage {
        status: if complete && issues.is_empty() && panel.pending_documents == 0 {
            "ready"
        } else {
            "partial"
        }
        .into(),
        event_count: usable_events(panel).len(),
        pending_documents: panel.pending_documents,
        sources: panel.coverage.clone(),
        issues: issues.into_iter().collect(),
    }
}

/// Preserve exact cached payments during an incomplete refresh. Missing notices
/// do not erase known money. The report separately withholds comparative
/// conclusions until the requested archive has been checked.
fn usable_events(panel: &HistoricalDividendPanel) -> Vec<IncomeEvent> {
    historical_dividends::income_events(panel)
}

pub fn read_cache(
    snapshot: &PortfolioSnapshot,
    directory: &Path,
    recent: &DividendPanel,
    today: NaiveDate,
) -> Result<CachedIncome, String> {
    let (requests, issues) = requests(snapshot, directory, today)?;
    let service =
        HistoricalDividendService::new(directory.join("public-data/historical-dividends.json"))?;
    let historical = service.read_cache(&requests)?;
    let status = coverage(&historical, issues);
    let mut events = dividends::income_events(recent);
    events.extend(usable_events(&historical));
    Ok(CachedIncome {
        events,
        coverage: status,
        historical,
    })
}

/// Continue missing public documents automatically, until complete, a provider
/// gate stops progress, or the bounded background budget expires. Each provider
/// batch saves its progress; re-opening the graph only reads the cache.
pub async fn refresh_if_due(
    snapshot: &PortfolioSnapshot,
    directory: &Path,
    today: NaiveDate,
) -> Result<HistoricalIncomeCoverage, String> {
    let (requests, issues) = requests(snapshot, directory, today)?;
    let service =
        HistoricalDividendService::new(directory.join("public-data/historical-dividends.json"))?;
    let work = async {
        for _ in 0..12 {
            let before = service.read_cache(&requests)?;
            let result = service.refresh_if_due(&requests).await;
            let Some(after) = result.data else {
                break;
            };
            let before_events: usize = before
                .items
                .iter()
                .filter_map(|item| item.observation.as_ref())
                .map(|item| item.events.len())
                .sum();
            let after_events: usize = after
                .items
                .iter()
                .filter_map(|item| item.observation.as_ref())
                .map(|item| item.events.len())
                .sum();
            if after.coverage.iter().all(|source| source.complete) && after.pending_documents == 0 {
                break;
            }
            if after_events <= before_events
                && after.pending_documents >= before.pending_documents
                && before.coverage == after.coverage
            {
                break;
            }
        }
        Ok::<(), String>(())
    };
    let result = tokio::time::timeout(Duration::from_secs(180), work).await;
    let panel = service.read_cache(&requests)?;
    let mut status = coverage(&panel, issues);
    match result {
        Ok(Ok(())) => {},
        Ok(Err(error)) => status.issues.push(error),
        Err(_) => status.issues.push("A consulta de proventos antigos atingiu o prazo. Os avisos já conferidos foram salvos; os demais podem continuar na próxima atualização.".into()),
    }
    if !status.issues.is_empty() {
        status.status = "partial".into();
    }
    Ok(status)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ivestfii_core::{FundInput, RecordInput, Repository};
    use serde_json::json;

    pub(crate) fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 20).unwrap()
    }

    fn repository(directory: &Path, shares: &str) -> Repository {
        let repo = Repository::open(directory.join("portfolio.sqlite")).unwrap();
        repo.upsert_fund(FundInput {
            id: None,
            ticker: "TEST11".into(),
            name: "Fundo sintético".into(),
            sector: "Teste".into(),
        })
        .unwrap();
        let id = repo.snapshot().unwrap().funds[0].id.clone();
        repo.save_monthly_record(RecordInput {
            id: None,
            fund_id: id,
            month: "2026-08".into(),
            purchase_date: Some("2026-08-03".into()),
            cotas_added: shares.into(),
            price_per_cota: "10".into(),
            dividend_per_cota: "0".into(),
            dividend_total: Some("2".into()),
            notes: "Nota privada que não pode integrar consultas públicas".into(),
        })
        .unwrap();
        repo
    }

    /// Synthetic public archive through the provider's persisted contract.
    /// Performance tests reuse it to exercise actual cache validation and
    /// reconciliation rather than constructing an already-combined report.
    pub(crate) fn archive(
        directory: &Path,
        snapshot: &PortfolioSnapshot,
        pending: bool,
    ) -> std::path::PathBuf {
        MarketSettingsStore::new(directory)
            .unwrap()
            .save_cvm_mapping("TEST11", Some("12345678000195"))
            .unwrap();
        let (planned, issues) = requests(snapshot, directory, day()).unwrap();
        assert!(issues.is_empty());
        let request = &planned[0];
        let fetched = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let make_event = |id: u64, rate: &str, record: &str, payment: &str| {
            json!({
                "eventId":format!("fundosnet:{id}:TEST11:income"),"symbol":"TEST11","perShare":rate,
                "kind":"income","label":"Rendimento","paymentDate":payment,"recordDate":record,
                "exDate":null,"declaredDate":record,"datePrecision":"exact","verified":true,
                "remarks":null,"relatedTo":null,"isinCode":"BRTESTCTF000",
                "sourceUrl":format!("https://fnet.bmfbovespa.com.br/fnet/publico/exibirDocumento?id={id}"),
                "lastObservedAt":fetched,"reconciliationPending":false
            })
        };
        let mut documents = vec![
            json!({"id":11,"version":1,"referenceDate":"01/08/2026","deliveredAt":"01/08/2026 18:00"}),
            json!({"id":12,"version":1,"referenceDate":"31/08/2026","deliveredAt":"31/08/2026 18:00"}),
        ];
        if pending {
            documents.push(json!({"id":13,"version":1,"referenceDate":"30/09/2026","deliveredAt":"30/09/2026 18:00"}));
        }
        let key = "TEST11:12345678000195";
        let path = directory.join("public-data/historical-dividends.json");
        std::fs::write(&path, serde_json::to_vec(&json!({"version":1,"funds":{key:{
            "request":request,
            "listing":{"fromDate":"2026-01-01","throughDate":"2026-10-20","checkedAt":fetched,
                "complete":true,"nextStart":documents.len(),"total":documents.len(),"documents":documents},
            "previousListing":null,
            "documents":{
                "11":{"version":1,"fetchedAt":fetched,"events":[make_event(11,"99","2026-08-01","2026-08-15")]},
                "12":{"version":1,"fetchedAt":fetched,"events":[make_event(12,"0.3","2026-08-31","2026-09-15")]}
            },"errors":{}
        }},"attempts":[],"retryAfter":null})).unwrap()).unwrap();
        path
    }

    pub(crate) fn recent_panel() -> DividendPanel {
        let fetched = chrono::Utc::now();
        let observation = dividends::parse_b3_dividends(&serde_json::to_vec(&json!({"code":"TEST","cashDividends":[
            {"isinCode":"BRTESTCTF000","label":"RENDIMENTO","rate":"0,3000","lastDatePrior":"2026-08-31","paymentDate":"2026-09-15"}
        ]})).unwrap(), "TEST11", fetched).unwrap();
        DividendPanel {
            items: vec![crate::dividends::DividendItem {
                symbol: "TEST11".into(),
                status: "cached".into(),
                observation: Some(observation),
                error: None,
            }],
            fetched_at: Some(fetched.to_rfc3339()),
            errors: vec![],
        }
    }

    #[test]
    fn planning_only_uses_public_identity_and_dates_with_large_holdings_and_reports_unknown_mappings(
    ) {
        let directory = tempfile::tempdir().unwrap();
        let repo = repository(directory.path(), "1001");
        let snapshot = repo.snapshot().unwrap();
        let absent = directory.path().join("not-created");
        let (none, issues) = requests(&snapshot, &absent, day()).unwrap();
        assert!(none.is_empty());
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("TEST11") && issues[0].contains("CNPJ"));
        assert!(!absent.exists());
        MarketSettingsStore::new(directory.path())
            .unwrap()
            .save_cvm_mapping("TEST11", Some("12345678000195"))
            .unwrap();
        let (planned, issues) = requests(&snapshot, directory.path(), day()).unwrap();
        assert!(issues.is_empty());
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].start_date, "2026-07-01");
        assert_eq!(planned[0].end_date, "2026-10-20");
        let fields = serde_json::to_value(&planned[0]).unwrap();
        assert_eq!(
            fields
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            ["cnpj", "endDate", "isin", "startDate", "symbol"]
                .into_iter()
                .collect()
        );
        assert!(!serde_json::to_string(&planned).unwrap().contains("1001"));
        assert!(!serde_json::to_string(&planned)
            .unwrap()
            .contains("Nota privada"));
    }

    #[test]
    fn local_missing_cache_creates_no_source_files_and_keeps_recent_public_income() {
        let directory = tempfile::tempdir().unwrap();
        let repo = repository(directory.path(), "10");
        let snapshot = repo.snapshot().unwrap();
        let absent = directory.path().join("not-created");
        let cached = read_cache(&snapshot, &absent, &recent_panel(), day()).unwrap();
        assert_eq!(cached.events.len(), 1);
        assert_eq!(cached.coverage.status, "partial");
        assert_eq!(cached.coverage.event_count, 0);
        assert!(!cached.coverage.issues.is_empty());
        assert!(!absent.exists());
    }

    #[test]
    fn complete_archive_and_recent_mirror_feed_one_read_only_calculation_with_manual_priority() {
        let directory = tempfile::tempdir().unwrap();
        let repo = repository(directory.path(), "10");
        let snapshot = repo.snapshot().unwrap();
        let path = archive(directory.path(), &snapshot, false);
        let before = serde_json::to_value(&snapshot).unwrap();
        let outbox = serde_json::to_value(repo.pending_changes().unwrap()).unwrap();
        let exported = repo.export_legacy_json().unwrap();
        let original = std::fs::read(&path).unwrap();
        let mappings =
            std::fs::read(directory.path().join("public-data/market-settings.json")).unwrap();
        for _ in 0..2 {
            let cached = read_cache(&snapshot, directory.path(), &recent_panel(), day()).unwrap();
            assert_eq!(cached.coverage.status, "ready");
            assert_eq!(cached.coverage.event_count, 2);
            assert_eq!(cached.coverage.pending_documents, 0);
            assert_eq!(
                cached.events.len(),
                3,
                "Economic reconciliation is shared in the core, not a lossy source/month filter"
            );
            let report = ivestfii_core::income::calculate_monthly_income(
                &snapshot,
                &cached.events,
                "2026-08",
                "2026-09",
                day(),
            )
            .unwrap();
            assert_eq!(report.months[0].total.as_deref(), Some("2"));
            assert_eq!(report.months[0].funds[0].status, "recorded");
            assert_eq!(report.months[1].total.as_deref(), Some("3"));
            assert_eq!(report.months[1].funds[0].events.len(), 1);
            assert_eq!(report.ignored_event_count, 1);
        }
        assert_eq!(
            serde_json::to_value(repo.snapshot().unwrap()).unwrap(),
            before
        );
        assert_eq!(
            serde_json::to_value(repo.pending_changes().unwrap()).unwrap(),
            outbox
        );
        assert_eq!(repo.export_legacy_json().unwrap(), exported);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(
            std::fs::read(directory.path().join("public-data/market-settings.json")).unwrap(),
            mappings
        );
        assert!(!directory
            .path()
            .join("public-data/historical-dividends.refresh-policy.json")
            .exists());
    }

    #[test]
    fn a_pending_document_preserves_verified_income_without_claiming_complete_archive_coverage()
    {
        let directory = tempfile::tempdir().unwrap();
        let repo = repository(directory.path(), "10");
        let snapshot = repo.snapshot().unwrap();
        archive(directory.path(), &snapshot, true);
        let cached = read_cache(
            &snapshot,
            directory.path(),
            &DividendPanel::default(),
            day(),
        )
        .unwrap();
        assert_eq!(
            cached.historical.items[0]
                .observation
                .as_ref()
                .unwrap()
                .events
                .len(),
            2
        );
        assert_eq!(cached.coverage.pending_documents, 1);
        assert_eq!(cached.coverage.event_count, 2);
        assert_eq!(cached.coverage.status, "partial");
        assert_eq!(cached.events.len(), 2);
        assert!(!cached.coverage.sources[0].complete);
        let report = ivestfii_core::income::calculate_monthly_income(
            &snapshot,
            &cached.events,
            "2026-08",
            "2026-09",
            day(),
        )
        .unwrap();
        assert_eq!(report.months[0].total.as_deref(), Some("2"));
        assert_eq!(report.months[1].total.as_deref(), Some("3"));
    }
}
