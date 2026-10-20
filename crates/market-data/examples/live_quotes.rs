//! Explicit opt-in network probe. Prints coverage only, never positions or prices.
use ivestfii_market_data::{MarketSettingsStore, QuoteService};
use std::path::PathBuf;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), String> {
    let mut arguments = std::env::args().skip(1);
    let mut directory = None;
    let mut symbols = Vec::new();
    while let Some(argument) = arguments.next() {
        if argument == "--data-dir" {
            directory = Some(PathBuf::from(
                arguments
                    .next()
                    .ok_or("Informe o diretório depois de --data-dir.")?,
            ));
        } else {
            symbols.push(argument);
        }
    }
    if symbols.is_empty() {
        return Err("Informe os tickers: cargo run -p ivestfii-market-data --example live_quotes -- MXRF11 XPML11".into());
    }
    let token = match directory.as_ref() {
        Some(path) => MarketSettingsStore::new(path)?.brapi_key()?.0,
        None => std::env::var("BRAPI_API_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                std::env::var("BRAPI_TOKEN")
                    .ok()
                    .filter(|s| !s.trim().is_empty())
            }),
    };
    let cache = std::env::var_os("IVESTFII_QUOTE_PROBE_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            directory
                .map(|path| path.join("public-data/quotes.json"))
                .unwrap_or_else(|| std::env::temp_dir().join("ivestfii-quote-coverage-probe.json"))
        });
    let service = QuoteService::new(cache)?;
    let panel = service.refresh(&symbols, token.as_deref()).await?;
    println!("credential_configured={}", panel.credential_configured);
    for item in panel.items {
        println!(
            "{} status={} has_quote={} has_market_time={}",
            item.symbol,
            item.status,
            item.observation.is_some(),
            item.observation
                .as_ref()
                .is_some_and(|x| x.quoted_at.is_some())
        );
    }
    Ok(())
}
