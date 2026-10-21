//! Bounded public-identity-only historical distribution backfill.
use ivestfii_market_data::historical_dividends::{
    HistoricalDividendRequest, HistoricalDividendService,
};
use std::{fs, path::PathBuf};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), String> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let get = |flag: &str| {
        arguments
            .iter()
            .position(|arg| arg == flag)
            .and_then(|index| arguments.get(index + 1))
            .cloned()
    };
    let input = get("--input").ok_or(
        "Use --input public-requests.json --cache public-history.json [--max-batches 1..12]",
    )?;
    let cache = PathBuf::from(get("--cache").ok_or("Informe --cache.")?);
    let batches = get("--max-batches")
        .unwrap_or("1".into())
        .parse::<usize>()
        .map_err(|_| "Número de lotes inválido.")?;
    if !(1..=12).contains(&batches) {
        return Err("Use entre 1 e 12 lotes limitados.".into());
    }
    let bytes = fs::read(input).map_err(|_| "Não foi possível ler as identidades públicas.")?;
    if bytes.len() > 64 * 1024 {
        return Err("Arquivo de consulta excedeu o limite seguro.".into());
    }
    let requests: Vec<HistoricalDividendRequest> =
        serde_json::from_slice(&bytes).map_err(|_| "Consulta pública inválida.")?;
    let service = HistoricalDividendService::new(cache)?;
    for batch in 1..=batches {
        let before = service.read_cache(&requests)?;
        let result = service.refresh(&requests).await?;
        println!(
            "{}",
            serde_json::json!({"batch":batch,"events":result.items.iter().filter_map(|item|item.observation.as_ref()).map(|observation|observation.events.len()).sum::<usize>(),"pendingDocuments":result.pending_documents,"coverage":result.coverage,"issues":result.issues})
        );
        if result.pending_documents == 0 {
            break;
        }
        if before.items == result.items && before.coverage == result.coverage {
            break;
        }
    }
    Ok(())
}
