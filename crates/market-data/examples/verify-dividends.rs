//! Optional live check of the public B3 feed, isolated from all user app data.
//! cargo run -p ivestfii-market-data --example verify-dividends -- MXRF11 VGHF11
use ivestfii_market_data::dividends::{income_events, DividendService};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), String> {
    let symbols: Vec<_> = std::env::args().skip(1).collect();
    if symbols.is_empty() {
        return Err(
            "Informe os tickers públicos que deseja validar, por exemplo MXRF11 VGHF11.".into(),
        );
    }
    let directory =
        tempfile::tempdir().map_err(|_| "Não foi possível criar a pasta temporária.")?;
    let service = DividendService::new(directory.path().join("dividends.json"))?;
    let result = service.refresh(&symbols).await?;
    let cached = service.read_cache(&symbols)?;
    let events = income_events(&cached);
    let summary: Vec<_> = result.items.iter().map(|item| {
        serde_json::json!({
            "symbol":item.symbol,"status":item.status,"error":item.error,
            "ordinaryEvents":item.observation.as_ref().map(|o|o.events.len()),
            "source":item.observation.as_ref().map(|o|o.source.as_str()),
            "incomePayments":events.iter().filter(|e|e.symbol==item.symbol).count(),
            "july2026":events.iter().filter(|e|e.symbol==item.symbol&&e.payment_date.starts_with("2026-07"))
                .map(|e|serde_json::json!({"perShare":e.amount_per_share,"recordDate":e.record_date,"paymentDate":e.payment_date})).collect::<Vec<_>>()
        })
    }).collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&summary)
            .map_err(|_| "Não foi possível serializar o resultado.")?
    );
    if !result.errors.is_empty() || result.items.iter().any(|item| item.observation.is_none()) {
        return Err(
            "Uma fonte não respondeu com eventos verificáveis. Confira os estados acima.".into(),
        );
    }
    Ok(())
}
