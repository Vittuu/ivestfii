//! Opt-in native startup probe. Use a backup directory for offline checks.
//! Prints source coverage and integrity only, never credentials or balances.
use ivestfii_core::WalletCatalog;
use ivestfii_market_data::{CvmFundRequest, EconomicService, MarketSettingsStore, NewsService};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

fn policies(directory: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut files = BTreeMap::new();
    let path = directory.join("public-data");
    if !path.exists() {
        return Ok(files);
    }
    for item in fs::read_dir(path).map_err(|error| error.to_string())? {
        let item = item.map_err(|error| error.to_string())?;
        let name = item.file_name().to_string_lossy().into_owned();
        if name.ends_with(".refresh-policy.json") {
            files.insert(
                name,
                fs::read(item.path()).map_err(|error| error.to_string())?,
            );
        }
    }
    Ok(files)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), String> {
    let mut arguments = std::env::args().skip(1);
    let mut directory = None;
    let mut wallet = None;
    let mut live = false;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--data-dir" => directory = arguments.next().map(PathBuf::from),
            "--wallet-id" => wallet = arguments.next(),
            "--live" => live = true,
            _ => return Err("Use --data-dir EXISTENTE [--wallet-id ID] [--live].".into()),
        }
    }
    let directory = directory.ok_or("Informe --data-dir com um diretório existente.")?;
    if !directory.join("wallets.sqlite").is_file() {
        return Err("O diretório precisa conter um catálogo existente.".into());
    }
    if !live {
        // Isolated process: any accidental HTTP fails locally without reaching a source.
        for name in ["HTTPS_PROXY", "HTTP_PROXY", "ALL_PROXY"] {
            std::env::set_var(name, "http://127.0.0.1:1");
        }
        std::env::set_var("NO_PROXY", "");
    }
    let catalog = WalletCatalog::open(&directory).map_err(|error| error.to_string())?;
    let repository = catalog
        .repository(wallet.as_deref())
        .map_err(|error| error.to_string())?;
    let before = repository.snapshot().map_err(|error| error.to_string())?;
    let original = serde_json::to_value(&before).map_err(|error| error.to_string())?;
    let mut symbols = before
        .funds
        .iter()
        .map(|fund| fund.ticker.clone())
        .collect::<BTreeSet<_>>();
    if let Some(position) = &before.broker_position {
        symbols.extend(
            position
                .listed_funds
                .holdings
                .iter()
                .map(|fund| fund.ticker.clone()),
        );
    }
    let symbols = symbols.into_iter().collect::<Vec<_>>();
    let settings = MarketSettingsStore::new(&directory)?;
    let (mappings, _) = settings.mappings()?;
    let requests = symbols
        .iter()
        .map(|symbol| CvmFundRequest {
            symbol: symbol.clone(),
            cnpj: mappings.get(symbol).cloned(),
        })
        .collect::<Vec<_>>();
    let economics = EconomicService::new(directory.join("public-data/economics.json"))?;
    let news = NewsService::new(directory.join("public-data/news.json"))?;
    let today = chrono::Utc::now()
        .with_timezone(&chrono::FixedOffset::west_opt(3 * 3600).unwrap())
        .date_naive();
    let first = ivestfii_app::public_data::refresh_if_due(
        &directory,
        &economics,
        &news,
        ivestfii_app::public_data::PublicDataRequest {
            snapshot: &before,
            symbols: &symbols,
            funds: &requests,
            key: settings.brapi_key().map(|(key, _)| key),
            today,
        },
    )
    .await?;
    let reservations = policies(&directory)?;
    let second = ivestfii_app::public_data::refresh_if_due(
        &directory,
        &economics,
        &news,
        ivestfii_app::public_data::PublicDataRequest {
            snapshot: &before,
            symbols: &symbols,
            funds: &requests,
            key: settings.brapi_key().map(|(key, _)| key),
            today,
        },
    )
    .await?;
    if policies(&directory)? != reservations {
        return Err("A segunda abertura reservou outra consulta antes do intervalo.".into());
    }
    let after = serde_json::to_value(repository.snapshot().map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    if original != after {
        return Err("Os dados financeiros foram alterados durante a consulta pública.".into());
    }
    println!(
        "{}",
        json!({"portfolioUnchanged":true,"secondPassMadeNoRequests":true,
        "live":live,"sources":first.sources.iter().map(|source| json!({"id":source.id,"status":source.status})).collect::<Vec<_>>(),
        "repeatSources":second.sources.iter().map(|source| json!({"id":source.id,"status":source.status})).collect::<Vec<_>>(),
        "indicatorCount":first.economics.as_ref().map_or(0, |panel|panel.indicators.len()),
        "newsCount":first.news.as_ref().map_or(0,|news|news.items.len()),
        "quotedFunds":first.quotes.items.iter().filter(|item|item.observation.is_some()).count(),
        "reportedFunds":first.cvm.items.iter().filter(|item|item.report.is_some()).count()})
    );
    Ok(())
}
