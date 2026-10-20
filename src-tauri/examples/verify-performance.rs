//! Opt-in read-only verification against a local ledger and public CDI data.
//! Does not read the brapi credential, change financial rows or launch the app.
use chrono::{FixedOffset, Utc};
use ivestfii_core::Repository;
use ivestfii_market_data::{CdiHistoryService, QuoteService};
use std::path::PathBuf;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let directory = PathBuf::from(args.next().ok_or("Informe o diretório de dados.")?);
    let refresh = args.next().is_some_and(|arg| arg == "--refresh-cdi");
    if args.next().is_some() {
        return Err("Argumentos extras não permitidos.".into());
    }
    let repository = Repository::open_existing(directory.join("portfolio.sqlite"))?;
    let snapshot = repository.snapshot()?;
    let symbols = snapshot
        .funds
        .iter()
        .map(|f| f.ticker.clone())
        .collect::<Vec<_>>();
    let quotes =
        QuoteService::new(directory.join("public-data/quotes.json"))?.read_cache(&symbols)?;
    let service = CdiHistoryService::new(directory.join("public-data/cdi-history.json"))?;
    let mut history = service.read_cache()?;
    let today = Utc::now()
        .with_timezone(&FixedOffset::west_opt(3 * 3600).unwrap())
        .date_naive();
    let before = ivestfii_app::performance::build_report(
        &snapshot,
        &quotes,
        history.as_ref(),
        today,
        vec![],
    )?;
    if refresh {
        let (start, end) =
            ivestfii_app::performance::cdi_range(&before)?.ok_or("Sem datas comparáveis.")?;
        history = Some(service.refresh(start, end).await?);
    }
    let report = ivestfii_app::performance::build_report(
        &snapshot,
        &quotes,
        history.as_ref(),
        today,
        vec![],
    )?;
    assert!(
        report.calculation.total_result.is_some(),
        "Resultado completo indisponível: {:?}",
        report.calculation.issues
    );
    assert!(
        report.calculation.cdi.is_some(),
        "CDI indisponível: {:?}",
        report.calculation.issues
    );
    assert_eq!(
        history.as_ref().unwrap().observations.last().unwrap().date,
        report.calculation.as_of.as_deref().unwrap()
    );
    println!(
        "{}",
        serde_json::json!({
            "status": report.calculation.status,
            "fundCount": snapshot.funds.len(),
            "recordCount": snapshot.record_count,
            "asOf": report.calculation.as_of,
            "cdiDailyObservations": history.as_ref().unwrap().observations.len(),
            "seriesId": history.as_ref().unwrap().series_id,
            "hasFullResult": report.calculation.total_result.is_some(),
            "hasComparableCdi": report.calculation.cdi.is_some(),
            "issues": report.calculation.issues,
        })
    );
    Ok(())
}
