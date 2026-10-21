//! Explicit public-data probe. Does not open a portfolio, credential or chat DB.
use ivestfii_market_data::{HistoricalPriceRequest, HistoricalPriceService};
use std::path::PathBuf;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut cache = None;
    let mut month = None;
    let mut through_date = None;
    let mut input = None;
    let mut max_batches = 1usize;
    let mut symbols = Vec::new();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--cache" => {
                cache = Some(PathBuf::from(
                    args.next().ok_or("Informe o arquivo de cache.")?,
                ))
            }
            "--month" => month = Some(args.next().ok_or("Informe o mês YYYY-MM.")?),
            "--input" => input = Some(PathBuf::from(args.next().ok_or("Informe o JSON público de períodos.")?)),
            "--max-batches" => max_batches = args.next().ok_or("Informe o limite de lotes.")?
                .parse().map_err(|_| "Limite de lotes inválido.")?,
            "--through" => {
                through_date = Some(args.next().ok_or("Informe a data final YYYY-MM-DD.")?)
            }
            _ if !argument.starts_with('-') => symbols.push(argument),
            _ => return Err("Opção desconhecida no teste de cotações históricas.".into()),
        }
    }
    let cache = cache.ok_or("Informe --cache ARQUIVO para manter o cache deste teste separado.")?;
    if !(1..=4).contains(&max_batches) { return Err("Use de 1 a 4 lotes por execução.".into()); }
    let bulk = input.is_some();
    let requests = if let Some(path) = input {
        if std::fs::metadata(&path).map_err(|_| "Solicitação pública não encontrada.")?.len() > 512 * 1024 {
            return Err("Solicitação pública excede 512 KiB.".into());
        }
        serde_json::from_slice::<Vec<HistoricalPriceRequest>>(&std::fs::read(path)
            .map_err(|_| "Não foi possível ler os períodos públicos.")?)
            .map_err(|_| "JSON de períodos públicos inválido.")?
    } else {
        let month = month.ok_or("Informe --month YYYY-MM e os tickers públicos.")?;
        if symbols.is_empty() { return Err("Informe pelo menos um ticker público.".into()); }
        symbols
        .into_iter()
        .map(|symbol| HistoricalPriceRequest {
            symbol,
            month: month.clone(),
            through_date: through_date.clone(),
        })
        .collect::<Vec<_>>()
    };
    let service = HistoricalPriceService::new(cache)?;
    let mut previous = service.read_cache(&requests)?.pending_requests;
    for batch in 1..=max_batches {
        let report = service.refresh(&requests).await?;
        if bulk {
            println!("{}", serde_json::json!({"batch":batch,"observations":report.observations.len(),"pendingRequests":report.pending_requests,"issues":report.issues}));
        } else { println!(
        "{}",
        serde_json::to_string_pretty(&report)
            .map_err(|_| "Não foi possível mostrar a cobertura do histórico.")?
        ); }
        if report.pending_requests == 0 || report.pending_requests >= previous { break; }
        previous = report.pending_requests;
    }
    Ok(())
}
