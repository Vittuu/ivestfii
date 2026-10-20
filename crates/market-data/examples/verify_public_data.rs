//! Explicit live verification using public data only. Normal tests never use HTTP.
use ivestfii_market_data::EconomicClient;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), String> {
    let panel = EconomicClient::new()?.indicators().await;
    println!(
        "{}",
        serde_json::to_string_pretty(&panel).map_err(|error| error.to_string())?
    );
    if panel.indicators.is_empty() {
        return Err("Nenhum indicador público foi recebido.".into());
    }
    Ok(())
}
