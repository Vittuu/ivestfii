//! Explicit live verification. Only fund symbols/CNPJs are accepted, no balances
//! or financial records. Example: cargo run -p ivestfii-market-data --example
//! verify_cvm -- FVPQ11 00.332.266/0001-31 VGIA11 41.081.088/0001-09
//! FII/FIAGRO must produce a real dated report; unsupported is not a success.
use ivestfii_market_data::cvm::{CvmFundRequest, CvmService};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), String> {
    let mut args: Vec<_> = std::env::args().skip(1).collect();
    let data_dir = if args.first().is_some_and(|arg| arg == "--data-dir") {
        if args.len() < 2 {
            return Err("Informe a pasta após --data-dir.".into());
        }
        let path = std::path::PathBuf::from(args.remove(1));
        args.remove(0);
        Some(path)
    } else {
        None
    };
    if args.is_empty() || args.len() % 2 != 0 {
        return Err(
            "Informe pares TICKER CNPJ_OFICIAL_DO_FUNDO. Nenhum dado de carteira é necessário."
                .into(),
        );
    }
    let requests: Vec<_> = args
        .chunks_exact(2)
        .map(|pair| CvmFundRequest {
            symbol: pair[0].to_uppercase(),
            cnpj: Some(pair[1].clone()),
        })
        .collect();
    let folder = tempfile::tempdir()
        .map_err(|_| "Não foi possível criar um cache de verificação isolado.".to_string())?;
    let path = data_dir
        .map(|path| path.join("cvm-funds.json"))
        .unwrap_or_else(|| folder.path().join("cvm-verification.json"));
    let service = CvmService::new(path)?;
    let panel = service.refresh(&requests).await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&panel).map_err(|error| error.to_string())?
    );
    if !panel.errors.is_empty()
        || panel.items.iter().any(|item| {
            item.report
                .as_ref()
                .is_none_or(|report| !matches!(report.fund_type.as_str(), "FII" | "FIAGRO"))
        })
    {
        return Err(
            "Um ou mais informes não foram validados; veja os resultados por fundo.".into(),
        );
    }
    let cached = service.read_cache(&requests)?;
    if cached
        .items
        .iter()
        .zip(&panel.items)
        .any(|(cached, fetched)| {
            cached.report != fetched.report
                || cached.cnpj != fetched.cnpj
                || cached.symbol != fetched.symbol
                || (cached.report.is_some() && cached.status != "cached")
        })
    {
        return Err("Leitura do cache difere do informe público validado.".into());
    }
    // A second refresh must reuse the dated archives and keep the same result.
    if service.refresh(&requests).await? != panel {
        return Err("Reutilização do cache CVM produziu um resultado diferente.".into());
    }
    Ok(())
}
