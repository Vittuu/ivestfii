//! Bounded manual check/backfill of public CVM NAV history. The input is a
//! FundNavRequest (public fund identity and dates only), never a wallet export.
use ivestfii_market_data::{CvmNavService, FundNavRequest};
use serde_json::json;
use std::{fs, path::PathBuf};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let option = |flag: &str| -> Option<String> {
        args.windows(2)
            .find(|pair| pair[0] == flag)
            .map(|pair| pair[1].clone())
    };
    let input = option("--input")
        .ok_or("Use --input public-request.json --cache public-nav.json [--max-batches 1..4].")?;
    let cache = option("--cache").ok_or("Informe --cache com o arquivo público de cotas.")?;
    let max_batches = option("--max-batches")
        .map(|v| v.parse::<usize>())
        .transpose()
        .map_err(|_| "Quantidade de lotes inválida.".to_string())?
        .unwrap_or(1);
    if !(1..=4).contains(&max_batches) {
        return Err("Use entre 1 e 4 lotes por execução.".into());
    }
    let metadata = fs::metadata(&input)
        .map_err(|_| "Não foi possível ler a solicitação pública.".to_string())?;
    if metadata.len() > 128 * 1024 {
        return Err("Solicitação pública excede 128 KiB.".into());
    }
    let request: FundNavRequest = serde_json::from_slice(
        &fs::read(input).map_err(|_| "Não foi possível ler a solicitação pública.".to_string())?,
    )
    .map_err(|_| "Solicitação pública inválida; use os campos FundNavRequest.".to_string())?;
    let service = CvmNavService::new(PathBuf::from(cache))?;
    let mut before = service.read_cache(&request)?.pending_months;
    for batch in 1..=max_batches {
        let data = service.refresh(&request).await?;
        println!(
            "{}",
            json!({
                "batch":batch,"identityResolved":data.cnpj.is_some(),
                "observations":data.observations.len(),
                "firstDate":data.observations.first().map(|v| &v.date),
                "lastDate":data.observations.last().map(|v| &v.date),
                "pendingMonths":data.pending_months,"issues":data.issues,
            })
        );
        if data.pending_months == 0 || data.pending_months >= before {
            break;
        }
        before = data.pending_months;
    }
    Ok(())
}
