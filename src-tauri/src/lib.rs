use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use ivestfii_assistant::AssistantStore;
use ivestfii_core::{
    CashPocketInput, CashSimulationInput, FundInput, InvestmentProfileInput, ProjectionInput,
    PurchaseConfirmationInput, PurchaseConfirmationReport, RecordInput, Repository, WalletCatalog,
    WalletInput, DEFAULT_WALLET_ID,
};
use ivestfii_market_data::{
    CdiHistoryService, CvmFundRequest, CvmPanel, CvmService, DividendPanel, DividendService,
    EconomicIndicator, EconomicPanel, EconomicService, MarketDataSettings, MarketSettingsStore,
    NewsService, NewsSnapshot, QuotePanel, QuoteService,
};
use ivestfii_runtime::{ChatRun, RunManager};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, State,
};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

mod operations;
pub mod performance;
pub mod public_data;
pub mod purchase_confirmation;
mod updates;

type CommandResult<T> = Result<T, String>;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    pub autostart: bool,
    pub start_minimized: bool,
    pub close_to_tray: bool,
    pub auto_update: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            autostart: !cfg!(debug_assertions),
            start_minimized: true,
            close_to_tray: true,
            auto_update: true,
        }
    }
}

fn read_settings(path: &Path) -> CommandResult<AppSettings> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("Configurações inválidas; arquivo preservado: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AppSettings::default()),
        Err(e) => Err(e.to_string()),
    }
}

fn persist_settings(path: &Path, settings: &AppSettings) -> CommandResult<()> {
    let bytes = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
    let temporary = path.with_extension("json.tmp");
    let mut file = fs::File::create(&temporary).map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    fs::rename(&temporary, path).map_err(|e| e.to_string())
}

struct AppState {
    wallets: Mutex<WalletCatalog>,
    assistant: Mutex<AssistantStore>,
    settings: Mutex<AppSettings>,
    settings_path: PathBuf,
    data_directory: PathBuf,
    legacy_path: Option<PathBuf>,
    migration: Mutex<Option<Value>>,
    position_path: Option<PathBuf>,
    position_import: Mutex<Option<Value>>,
    runs: Arc<RunManager>,
    economics: EconomicService,
    news: NewsService,
    updates: updates::UpdateManager,
    operations: operations::OperationGate,
    frontend_unsettled: std::sync::atomic::AtomicBool,
    last_window_action: Mutex<std::time::Instant>,
}

fn update_install_permit(app: tauri::AppHandle, manual: bool) -> updates::AcquireInstallPermit {
    Arc::new(move || {
        let state = app.try_state::<AppState>()?;
        let permit = state.operations.try_install()?;
        if state.runs.has_active_runs()
            || state
                .frontend_unsettled
                .load(std::sync::atomic::Ordering::SeqCst)
            || (!manual
                && (!state.settings.lock().ok().is_some_and(|s| s.auto_update)
                    || !state
                        .last_window_action
                        .lock()
                        .ok()
                        .is_some_and(|at| at.elapsed() >= std::time::Duration::from_secs(30))))
            || (!manual
                && app
                    .get_webview_window("main")
                    .is_none_or(|w| w.is_visible().unwrap_or(true)))
        {
            return None;
        }
        Some(Box::new(permit))
    })
}

#[tauri::command]
fn get_update_status(state: State<AppState>) -> CommandResult<updates::UpdateStatus> {
    state.updates.status()
}

#[tauri::command]
async fn check_for_updates(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<updates::UpdateStatus> {
    state.updates.check_and_download(&app).await
}

#[tauri::command]
fn install_pending_update(
    app: tauri::AppHandle,
    state: State<AppState>,
) -> CommandResult<updates::UpdateStatus> {
    state
        .updates
        .retry_install(&app, &update_install_permit(app.clone(), true))
}

#[tauri::command]
fn set_update_activity(state: State<AppState>, blocked: bool) {
    state
        .frontend_unsettled
        .store(blocked, std::sync::atomic::Ordering::SeqCst);
}

fn start_automatic_updates(app: tauri::AppHandle, manager: updates::UpdateManager) {
    if !manager.enabled() {
        return;
    }
    let permit = update_install_permit(app.clone(), false);
    tauri::async_runtime::spawn(async move {
        // Give the frontend time to report pending forms before an idle restart.
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        let mut last_check: Option<std::time::Instant> = None;
        loop {
            let enabled = app
                .try_state::<AppState>()
                .and_then(|state| state.settings.lock().ok().map(|s| s.auto_update))
                .unwrap_or(false);
            if enabled {
                if last_check.is_none_or(|last| last.elapsed() >= updates::CHECK_INTERVAL) {
                    let _ = manager.check_and_download(&app).await;
                    last_check = Some(std::time::Instant::now());
                }
                let _ = manager.install_pending(&app, &permit);
            }
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    });
}

fn as_json(value: impl Serialize) -> CommandResult<Value> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_economic_cache(state: State<AppState>) -> CommandResult<Option<EconomicPanel>> {
    let _operation = state.operations.begin()?;
    state.economics.read_cache()
}

#[tauri::command]
async fn refresh_economic_indicators(state: State<'_, AppState>) -> CommandResult<EconomicPanel> {
    let _operation = state.operations.begin()?;
    state.economics.refresh().await
}

#[tauri::command]
async fn fetch_cdi_reference(state: State<'_, AppState>) -> CommandResult<EconomicIndicator> {
    let _operation = state.operations.begin()?;
    state.economics.cdi_reference().await
}

#[tauri::command]
fn get_news_cache(state: State<AppState>) -> CommandResult<Option<NewsSnapshot>> {
    let _operation = state.operations.begin()?;
    state.news.read_cache()
}

#[tauri::command]
async fn refresh_news(state: State<'_, AppState>) -> CommandResult<NewsSnapshot> {
    let _operation = state.operations.begin()?;
    state.news.refresh().await
}

fn market_settings(state: &AppState) -> CommandResult<MarketSettingsStore> {
    MarketSettingsStore::new(&state.data_directory)
}

fn market_symbols(state: &AppState, wallet_id: Option<&str>) -> CommandResult<Vec<String>> {
    let snapshot = selected_repository(state, wallet_id)?
        .snapshot()
        .map_err(|e| e.to_string())?;
    snapshot_symbols(&snapshot)
}

fn snapshot_symbols(snapshot: &ivestfii_core::PortfolioSnapshot) -> CommandResult<Vec<String>> {
    let mut symbols = std::collections::BTreeSet::new();
    for fund in &snapshot.funds {
        symbols.insert(ivestfii_market_data::quotes::normalize_symbol(
            &fund.ticker,
        )?);
    }
    if let Some(position) = &snapshot.broker_position {
        for holding in &position.listed_funds.holdings {
            symbols.insert(ivestfii_market_data::quotes::normalize_symbol(
                &holding.ticker,
            )?);
        }
    }
    Ok(symbols.into_iter().collect())
}

fn cvm_requests(state: &AppState, wallet_id: Option<&str>) -> CommandResult<Vec<CvmFundRequest>> {
    let symbols = market_symbols(state, wallet_id)?;
    let (mappings, _) = market_settings(state)?.mappings()?;
    Ok(symbols
        .into_iter()
        .map(|symbol| CvmFundRequest {
            cnpj: mappings.get(&symbol).cloned(),
            symbol,
        })
        .collect())
}

#[tauri::command]
fn get_market_data_settings(state: State<AppState>) -> CommandResult<MarketDataSettings> {
    let _operation = state.operations.begin()?;
    market_settings(&state)?.read()
}

#[tauri::command]
fn save_brapi_key(state: State<AppState>, key: String) -> CommandResult<MarketDataSettings> {
    let _operation = state.operations.begin()?;
    market_settings(&state)?.save_brapi_key(&key)
}

#[tauri::command]
fn clear_brapi_key(state: State<AppState>) -> CommandResult<MarketDataSettings> {
    let _operation = state.operations.begin()?;
    market_settings(&state)?.clear_brapi_key()
}

#[tauri::command]
fn save_cvm_mapping(
    state: State<AppState>,
    symbol: String,
    cnpj: Option<String>,
) -> CommandResult<MarketDataSettings> {
    let _operation = state.operations.begin()?;
    market_settings(&state)?.save_cvm_mapping(&symbol, cnpj.as_deref())
}

#[tauri::command]
fn get_quote_cache(state: State<AppState>, wallet_id: Option<String>) -> CommandResult<QuotePanel> {
    let _operation = state.operations.begin()?;
    let symbols = market_symbols(&state, wallet_id.as_deref())?;
    let credential = market_settings(&state)?.brapi_key()?.0.is_some();
    let mut panel = QuoteService::new(state.data_directory.join("public-data/quotes.json"))?
        .read_cache(&symbols)
        .unwrap_or_else(|error| QuotePanel {
            items: symbols
                .iter()
                .map(|symbol| ivestfii_market_data::quotes::QuoteItem {
                    symbol: symbol.clone(),
                    status: "unavailable".into(),
                    observation: None,
                    error: Some(error.clone()),
                })
                .collect(),
            errors: vec![error],
            ..QuotePanel::default()
        });
    panel.set_credential_presence(credential);
    Ok(panel)
}

#[tauri::command]
async fn refresh_quotes(
    state: State<'_, AppState>,
    wallet_id: Option<String>,
) -> CommandResult<QuotePanel> {
    let _operation = state.operations.begin()?;
    let symbols = market_symbols(&state, wallet_id.as_deref())?;
    let (key, _) = market_settings(&state)?.brapi_key()?;
    let service = QuoteService::new(state.data_directory.join("public-data/quotes.json"))?;
    ivestfii_market_data::bounded_quotes(&service, &symbols, key.as_deref()).await
}

fn performance_today() -> chrono::NaiveDate {
    chrono::Utc::now()
        .with_timezone(&chrono::FixedOffset::west_opt(3 * 3600).unwrap())
        .date_naive()
}

#[tauri::command]
fn get_dividend_cache(
    state: State<AppState>,
    wallet_id: Option<String>,
) -> CommandResult<DividendPanel> {
    let _operation = state.operations.begin()?;
    let symbols = market_symbols(&state, wallet_id.as_deref())?;
    DividendService::new(state.data_directory.join("public-data/dividends.json"))?
        .read_cache(&symbols)
}

#[tauri::command]
fn get_monthly_income(
    state: State<AppState>,
    wallet_id: Option<String>,
    start_month: String,
    end_month: String,
) -> CommandResult<ivestfii_core::income::MonthlyIncomeReport> {
    let _operation = state.operations.begin()?;
    let snapshot = selected_repository(&state, wallet_id.as_deref())?
        .snapshot()
        .map_err(|error| error.to_string())?;
    let symbols = snapshot_symbols(&snapshot)?;
    let panel = DividendService::new(state.data_directory.join("public-data/dividends.json"))
        .and_then(|service| service.read_cache(&symbols))
        .unwrap_or_default();
    // Cached public events supply the rate/date. The calculation is ephemeral:
    // it never inserts a receipt or changes the user's recorded purchases.
    let events = ivestfii_market_data::income_history::read_cache(
        &snapshot,
        &state.data_directory,
        &panel,
        performance_today(),
    )
    .map(|data| data.events)
    .unwrap_or_else(|_| ivestfii_market_data::dividends::income_events(&panel));
    ivestfii_core::income::calculate_monthly_income(
        &snapshot,
        &events,
        &start_month,
        &end_month,
        performance_today(),
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_fii_performance(
    state: State<AppState>,
    wallet_id: Option<String>,
) -> CommandResult<performance::PerformanceReport> {
    let _operation = state.operations.begin()?;
    let repository = selected_repository(&state, wallet_id.as_deref())?;
    let snapshot = repository.snapshot().map_err(|e| e.to_string())?;
    let report = performance::cached_report(&snapshot, &state.data_directory, performance_today())?;
    Ok(remember_portfolio_observation(&repository, report))
}

/// Observations are derived chart measurements, separate from purchases,
/// received income and the sync outbox. A capture error does not hide results.
fn remember_portfolio_observation(
    repository: &Repository,
    mut report: performance::PerformanceReport,
) -> performance::PerformanceReport {
    if let Some(observation) = &report.history_observation {
        if let Err(error) = repository.record_performance_observation(observation) {
            report.portfolio_evolution.issues.push(format!(
                "Não foi possível guardar o registro mensal da carteira: {error}"
            ));
        }
    }
    report
}

#[tauri::command]
async fn refresh_fii_performance(
    state: State<'_, AppState>,
    wallet_id: Option<String>,
) -> CommandResult<performance::PerformanceReport> {
    let _operation = state.operations.begin()?;
    // Capture one immutable ledger snapshot; no mutex is held over network IO.
    let snapshot = selected_repository(&state, wallet_id.as_deref())?
        .snapshot()
        .map_err(|e| e.to_string())?;
    let symbols = snapshot
        .funds
        .iter()
        .map(|fund| fund.ticker.clone())
        .collect::<Vec<_>>();
    let quotes = QuoteService::new(state.data_directory.join("public-data/quotes.json"))?;
    let dividends = DividendService::new(state.data_directory.join("public-data/dividends.json"))?;
    let history_service =
        CdiHistoryService::new(state.data_directory.join("public-data/cdi-history.json"))?;
    let today = performance_today();
    let mut errors = Vec::new();
    let (key, _) = market_settings(&state)?.brapi_key()?;
    // Reuse the B3 cache and its own daily cadence; opening or refreshing the
    // result must not bypass the shared public-source request policy.
    let (quote_result, dividend_result, historical_result, income_result) = tokio::join!(
        ivestfii_market_data::bounded_quotes(&quotes, &symbols, key.as_deref()),
        tokio::time::timeout(
            std::time::Duration::from_secs(45),
            dividends.refresh_if_due(&symbols),
        ),
        ivestfii_market_data::historical_performance::refresh_if_due(
            &snapshot,
            &state.data_directory,
            today,
        ),
        ivestfii_market_data::income_history::refresh_if_due(
            &snapshot,
            &state.data_directory,
            today
        ),
    );
    match income_result {
        Ok(coverage) => errors.extend(coverage.issues),
        Err(error) => errors.push(error),
    }
    match historical_result {
        Ok(coverage) => errors.extend(coverage.issues),
        Err(error) => errors.push(error),
    }
    let panel = match quote_result {
        Ok(panel) => panel,
        Err(error) => {
            errors.push(error);
            quotes.read_cache(&symbols)?
        }
    };
    let dividend_panel = match dividend_result {
        Ok(result) => {
            if let Some(message) = result.message {
                errors.push(message);
            }
            result.data.unwrap_or_default()
        }
        Err(_) => {
            errors.push(
                "A consulta de proventos demorou demais. Os eventos salvos foram mantidos.".into(),
            );
            dividends.read_cache(&symbols).unwrap_or_else(|error| {
                errors.push(error);
                DividendPanel::default()
            })
        }
    };
    let mut history = history_service.read_cache().unwrap_or_else(|error| {
        errors.push(error);
        None
    });
    let preliminary = performance::build_report_with_income(
        &snapshot,
        &panel,
        history.as_ref(),
        &dividend_panel,
        today,
        Vec::new(),
    )?;
    if let Some((start, end)) = performance::cdi_range(&preliminary)? {
        match history_service.refresh(start, end).await {
            Ok(fetched) => history = Some(fetched),
            Err(error) => errors.push(error),
        }
    }
    let report = performance::build_report_with_public_history(
        &snapshot,
        &state.data_directory,
        &panel,
        history.as_ref(),
        &dividend_panel,
        today,
        errors,
    )?;
    let repository = selected_repository(&state, wallet_id.as_deref())?;
    Ok(remember_portfolio_observation(&repository, report))
}

#[tauri::command]
fn get_cvm_cache(state: State<AppState>, wallet_id: Option<String>) -> CommandResult<CvmPanel> {
    let _operation = state.operations.begin()?;
    CvmService::new(state.data_directory.join("public-data/cvm-funds.json"))?
        .read_cache(&cvm_requests(&state, wallet_id.as_deref())?)
}

#[tauri::command]
async fn refresh_cvm_data(
    state: State<'_, AppState>,
    wallet_id: Option<String>,
) -> CommandResult<CvmPanel> {
    let _operation = state.operations.begin()?;
    let funds = cvm_requests(&state, wallet_id.as_deref())?;
    let service = CvmService::new(state.data_directory.join("public-data/cvm-funds.json"))?;
    ivestfii_market_data::bounded_cvm(&service, &funds).await
}

#[tauri::command]
async fn refresh_public_data_if_due(
    state: State<'_, AppState>,
    wallet_id: Option<String>,
) -> CommandResult<public_data::PublicDataRefresh> {
    // An updater cannot restart the app while these background requests run.
    let _operation = state.operations.begin()?;
    let snapshot = selected_repository(&state, wallet_id.as_deref())?
        .snapshot()
        .map_err(|error| error.to_string())?;
    let symbols = snapshot_symbols(&snapshot)?;
    let settings = market_settings(&state)?;
    let (mappings, _) = settings.mappings()?;
    let funds = symbols
        .iter()
        .map(|symbol| CvmFundRequest {
            symbol: symbol.clone(),
            cnpj: mappings.get(symbol).cloned(),
        })
        .collect::<Vec<_>>();
    let key = settings.brapi_key().map(|(key, _)| key);
    public_data::refresh_if_due(
        &state.data_directory,
        &state.economics,
        &state.news,
        public_data::PublicDataRequest {
            snapshot: &snapshot,
            symbols: &symbols,
            funds: &funds,
            key,
            today: performance_today(),
        },
    )
    .await
}

fn public_source_url(value: &str) -> CommandResult<String> {
    let url = tauri::Url::parse(value).map_err(|_| "Link da fonte inválido.".to_string())?;
    if value.len() > 4096
        || url.scheme() != "https"
        || !matches!(
            url.host_str(),
            Some(
                "api.bcb.gov.br"
                    | "agenciabrasil.ebc.com.br"
                    | "brapi.dev"
                    | "dados.cvm.gov.br"
                    | "sistemas.cvm.gov.br"
                    | "fnet.bmfbovespa.com.br"
                    | "sistemaswebb3-listados.b3.com.br"
            )
        )
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|port| port != 443)
    {
        return Err("O link precisa usar HTTPS em uma das fontes públicas cadastradas.".into());
    }
    Ok(url.into())
}

#[tauri::command]
fn open_public_source(app: tauri::AppHandle, url: String) -> CommandResult<()> {
    app.opener()
        .open_url(public_source_url(&url)?, None::<&str>)
        .map_err(|error| format!("Não foi possível abrir o navegador: {error}"))
}

fn selected_repository(state: &AppState, wallet_id: Option<&str>) -> CommandResult<Repository> {
    state
        .wallets
        .lock()
        .map_err(|e| e.to_string())?
        .repository(wallet_id)
        .map_err(|e| e.to_string())
}

fn export_selected(state: &AppState, wallet_id: Option<&str>) -> CommandResult<String> {
    let wallet = state
        .wallets
        .lock()
        .map_err(|e| e.to_string())?
        .get(wallet_id)
        .map_err(|e| e.to_string())?;
    let mut content: Value = serde_json::from_str(
        &selected_repository(state, Some(&wallet.id))?
            .export_legacy_json()
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    content["wallet"] = json!({"name":wallet.name,"institution":wallet.institution});
    serde_json::to_string_pretty(&content).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_wallets(state: State<AppState>) -> CommandResult<Value> {
    let _operation = state.operations.begin()?;
    as_json(
        state
            .wallets
            .lock()
            .map_err(|e| e.to_string())?
            .list()
            .map_err(|e| e.to_string())?,
    )
}

#[tauri::command]
fn create_wallet(state: State<AppState>, input: WalletInput) -> CommandResult<Value> {
    let _operation = state.operations.begin()?;
    as_json(
        state
            .wallets
            .lock()
            .map_err(|e| e.to_string())?
            .create(input)
            .map_err(|e| e.to_string())?,
    )
}

#[tauri::command]
fn update_wallet(state: State<AppState>, input: WalletInput) -> CommandResult<Value> {
    let _operation = state.operations.begin()?;
    as_json(
        state
            .wallets
            .lock()
            .map_err(|e| e.to_string())?
            .update(input)
            .map_err(|e| e.to_string())?,
    )
}

#[tauri::command]
fn get_wealth_overview(state: State<AppState>) -> CommandResult<Value> {
    let _operation = state.operations.begin()?;
    as_json(
        state
            .wallets
            .lock()
            .map_err(|e| e.to_string())?
            .wealth_overview()
            .map_err(|e| e.to_string())?,
    )
}

#[tauri::command]
fn get_investment_profile(
    state: State<AppState>,
    wallet_id: Option<String>,
) -> CommandResult<Value> {
    let _operation = state.operations.begin()?;
    as_json(
        selected_repository(&state, wallet_id.as_deref())?
            .investment_profile()
            .map_err(|error| error.to_string())?,
    )
}

#[tauri::command]
fn save_investment_profile(
    state: State<AppState>,
    wallet_id: Option<String>,
    input: InvestmentProfileInput,
) -> CommandResult<Value> {
    let _operation = state.operations.begin()?;
    as_json(
        selected_repository(&state, wallet_id.as_deref())?
            .save_investment_profile(input)
            .map_err(|error| error.to_string())?,
    )
}

#[tauri::command]
fn get_cash_pockets(state: State<AppState>, wallet_id: Option<String>) -> CommandResult<Value> {
    let _operation = state.operations.begin()?;
    as_json(
        selected_repository(&state, wallet_id.as_deref())?
            .cash_pockets()
            .map_err(|e| e.to_string())?,
    )
}

#[tauri::command]
fn get_cash_performance(
    state: State<AppState>,
    wallet_id: Option<String>,
) -> CommandResult<ivestfii_core::portfolio_performance::CashPerformance> {
    let _operation = state.operations.begin()?;
    let pockets = selected_repository(&state, wallet_id.as_deref())?
        .cash_pockets()
        .map_err(|e| e.to_string())?;
    ivestfii_core::portfolio_performance::calculate_cash_performance(&pockets)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn save_cash_pocket(
    state: State<AppState>,
    wallet_id: Option<String>,
    input: CashPocketInput,
) -> CommandResult<Value> {
    let _operation = state.operations.begin()?;
    as_json(
        selected_repository(&state, wallet_id.as_deref())?
            .upsert_cash_pocket(input)
            .map_err(|e| e.to_string())?,
    )
}

#[tauri::command]
fn simulate_cash_pocket(
    state: State<AppState>,
    wallet_id: Option<String>,
    input: CashSimulationInput,
) -> CommandResult<Value> {
    let _operation = state.operations.begin()?;
    as_json(
        selected_repository(&state, wallet_id.as_deref())?
            .simulate_cash_pocket(input)
            .map_err(|e| e.to_string())?,
    )
}

#[tauri::command]
fn get_portfolio(state: State<AppState>, wallet_id: Option<String>) -> CommandResult<Value> {
    let _operation = state.operations.begin()?;
    as_json(
        selected_repository(&state, wallet_id.as_deref())?
            .snapshot()
            .map_err(|e| e.to_string())?,
    )
}

#[tauri::command]
fn import_legacy(
    state: State<AppState>,
    path: String,
    wallet_id: Option<String>,
) -> CommandResult<Value> {
    let _operation = state.operations.begin()?;
    let receipt = selected_repository(&state, wallet_id.as_deref())?
        .import_legacy_file(Path::new(&path))
        .map_err(|e| e.to_string())?;
    let receipt = as_json(receipt)?;
    if wallet_id
        .as_deref()
        .is_none_or(|id| id == DEFAULT_WALLET_ID)
    {
        *state.migration.lock().map_err(|e| e.to_string())? = Some(receipt.clone());
    }
    Ok(receipt)
}

#[tauri::command]
fn import_broker_position(
    state: State<AppState>,
    path: String,
    wallet_id: Option<String>,
) -> CommandResult<Value> {
    let _operation = state.operations.begin()?;
    let position = selected_repository(&state, wallet_id.as_deref())?
        .import_broker_position_file(Path::new(&path))
        .map_err(|e| e.to_string())?;
    let result = as_json(position)?;
    if wallet_id
        .as_deref()
        .is_none_or(|id| id == DEFAULT_WALLET_ID)
    {
        *state.position_import.lock().map_err(|e| e.to_string())? = Some(json!({"status":"ok"}));
    }
    Ok(result)
}

#[tauri::command]
fn export_portfolio(state: State<AppState>, wallet_id: Option<String>) -> CommandResult<String> {
    let _operation = state.operations.begin()?;
    export_selected(&state, wallet_id.as_deref())
}

#[tauri::command]
async fn choose_import_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<Option<String>> {
    let _operation = state.operations.begin()?;
    let selected = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Carteira JSON", &["json"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    selected
        .map(|file| {
            file.into_path()
                .map(|p| p.to_string_lossy().to_string())
                .map_err(|e| e.to_string())
        })
        .transpose()
}

#[tauri::command]
async fn export_portfolio_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    wallet_id: Option<String>,
) -> CommandResult<Option<String>> {
    let _operation = state.operations.begin()?;
    let content = export_selected(&state, wallet_id.as_deref())?;
    let filename = format!(
        "ivestfii-{}-backup-{}.json",
        wallet_id.as_deref().unwrap_or(DEFAULT_WALLET_ID),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs()
    );
    let selected = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Carteira JSON", &["json"])
            .set_file_name(filename)
            .blocking_save_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(selected) = selected else {
        return Ok(None);
    };
    let path = selected.into_path().map_err(|e| e.to_string())?;
    // A backup never overwrites the legacy source or an existing export.
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                "O arquivo já existe. Escolha outro nome para preservar as cópias anteriores."
                    .to_string()
            } else {
                e.to_string()
            }
        })?;
    file.write_all(content.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(Some(path.to_string_lossy().to_string()))
}

#[tauri::command]
fn upsert_fund(
    state: State<AppState>,
    input: FundInput,
    wallet_id: Option<String>,
) -> CommandResult<()> {
    let _operation = state.operations.begin()?;
    selected_repository(&state, wallet_id.as_deref())?
        .upsert_fund(input)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn save_record(
    state: State<AppState>,
    input: RecordInput,
    wallet_id: Option<String>,
) -> CommandResult<()> {
    let _operation = state.operations.begin()?;
    selected_repository(&state, wallet_id.as_deref())?
        .save_monthly_record(input)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn preview_simulation_purchases(
    state: State<AppState>,
    wallet_id: String,
    conversation_id: String,
    input: PurchaseConfirmationInput,
) -> CommandResult<PurchaseConfirmationReport> {
    let _operation = state.operations.begin()?;
    let repository = selected_repository(&state, Some(&wallet_id))?;
    let assistant = state.assistant.lock().map_err(|e| e.to_string())?;
    purchase_confirmation::preview(
        &assistant,
        &repository,
        &wallet_id,
        &conversation_id,
        input,
        chrono::Local::now().date_naive(),
    )
}

#[tauri::command]
fn confirm_simulation_purchases(
    state: State<AppState>,
    wallet_id: String,
    conversation_id: String,
    input: PurchaseConfirmationInput,
    preview_token: String,
) -> CommandResult<PurchaseConfirmationReport> {
    let _operation = state.operations.begin()?;
    let repository = selected_repository(&state, Some(&wallet_id))?;
    let assistant = state.assistant.lock().map_err(|e| e.to_string())?;
    purchase_confirmation::confirm(
        &assistant,
        &repository,
        &wallet_id,
        &conversation_id,
        input,
        chrono::Local::now().date_naive(),
        &preview_token,
    )
}

#[tauri::command]
fn get_simulation_purchase_confirmations(
    state: State<AppState>,
    wallet_id: String,
    conversation_id: String,
    source_tool_id: String,
) -> CommandResult<PurchaseConfirmationReport> {
    let _operation = state.operations.begin()?;
    let repository = selected_repository(&state, Some(&wallet_id))?;
    let assistant = state.assistant.lock().map_err(|e| e.to_string())?;
    purchase_confirmation::confirmations(
        &assistant,
        &repository,
        &wallet_id,
        &conversation_id,
        &source_tool_id,
    )
}

#[tauri::command]
fn project_income(
    state: State<AppState>,
    input: ProjectionInput,
    wallet_id: Option<String>,
) -> CommandResult<Value> {
    let _operation = state.operations.begin()?;
    as_json(
        selected_repository(&state, wallet_id.as_deref())?
            .project(input)
            .map_err(|e| e.to_string())?,
    )
}

#[tauri::command]
fn get_settings(app: tauri::AppHandle, state: State<AppState>) -> CommandResult<AppSettings> {
    let _operation = state.operations.begin()?;
    let mut settings = state.settings.lock().map_err(|e| e.to_string())?.clone();
    settings.autostart = app.autolaunch().is_enabled().map_err(|e| e.to_string())?;
    Ok(settings)
}

#[tauri::command]
fn save_settings(
    app: tauri::AppHandle,
    state: State<AppState>,
    settings: AppSettings,
) -> CommandResult<AppSettings> {
    let _operation = state.operations.begin()?;
    let mut current = state.settings.lock().map_err(|e| e.to_string())?;
    let was_enabled = app.autolaunch().is_enabled().map_err(|e| e.to_string())?;
    if settings.autostart {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    }
    .map_err(|e| e.to_string())?;
    if let Err(error) = persist_settings(&state.settings_path, &settings) {
        let rollback = if was_enabled {
            app.autolaunch().enable()
        } else {
            app.autolaunch().disable()
        };
        return Err(format!(
            "{error}; restauração da inicialização: {}",
            if rollback.is_ok() { "ok" } else { "falhou" }
        ));
    }
    *current = settings.clone();
    Ok(settings)
}

#[tauri::command]
fn get_app_info(state: State<AppState>, wallet_id: Option<String>) -> CommandResult<Value> {
    let _operation = state.operations.begin()?;
    let wallet = state
        .wallets
        .lock()
        .map_err(|e| e.to_string())?
        .get(wallet_id.as_deref())
        .map_err(|e| e.to_string())?;
    let pending = selected_repository(&state, Some(&wallet.id))?
        .pending_change_count()
        .map_err(|e| e.to_string())?;
    let database_path = if wallet.is_default {
        state.data_directory.join("portfolio.sqlite")
    } else {
        state
            .data_directory
            .join("wallets")
            .join(&wallet.id)
            .join("portfolio.sqlite")
    };
    Ok(json!({
        "dataDirectory": state.data_directory.to_string_lossy(),
        "databasePath": database_path.to_string_lossy(),
        "wallet":wallet,
        "legacyPath": if wallet.is_default {state.legacy_path.as_ref().map(|p| p.to_string_lossy().to_string())} else {None},
        "migration": if wallet.is_default {state.migration.lock().map_err(|e| e.to_string())?.clone()} else {None},
        "positionPath": if wallet.is_default {state.position_path.as_ref().map(|p| p.to_string_lossy().to_string())} else {None},
        "positionImport": if wallet.is_default {state.position_import.lock().map_err(|e| e.to_string())?.clone()} else {None},
        "sync": {"status": "not_configured", "pendingChanges": pending}
    }))
}

#[tauri::command]
async fn get_providers() -> CommandResult<Value> {
    let providers = tauri::async_runtime::spawn_blocking(ivestfii_runtime::probe_providers)
        .await
        .map_err(|e| e.to_string())?;
    as_json(providers)
}

#[tauri::command]
fn start_chat_turn(
    app: tauri::AppHandle,
    state: State<AppState>,
    conversation_id: String,
    provider: String,
    model: Option<String>,
    content: String,
    wallet_id: Option<String>,
) -> CommandResult<ChatRun> {
    let _operation = state.operations.begin()?;
    let wallet = state
        .wallets
        .lock()
        .map_err(|e| e.to_string())?
        .get(wallet_id.as_deref())
        .map_err(|e| e.to_string())?;
    state.runs.start_for_wallet(
        conversation_id,
        provider,
        model,
        content,
        Some(wallet.id),
        Arc::new(move |run| {
            let _ = app.emit("chat-run", run);
        }),
    )
}

#[tauri::command]
fn get_chat_run(state: State<AppState>, conversation_id: String) -> CommandResult<Option<ChatRun>> {
    let _operation = state.operations.begin()?;
    state.runs.latest(&conversation_id)
}

#[tauri::command]
fn cancel_chat_turn(state: State<AppState>, run_id: String) -> CommandResult<()> {
    let _operation = state.operations.begin()?;
    state.runs.cancel(&run_id)
}

#[tauri::command]
fn get_tool_events(state: State<AppState>, conversation_id: String) -> CommandResult<Value> {
    let _operation = state.operations.begin()?;
    as_json(
        state
            .assistant
            .lock()
            .map_err(|e| e.to_string())?
            .tool_events(&conversation_id)
            .map_err(|e| e.to_string())?,
    )
}

#[tauri::command]
fn list_conversations(state: State<AppState>) -> CommandResult<Value> {
    let _operation = state.operations.begin()?;
    as_json(
        state
            .assistant
            .lock()
            .map_err(|e| e.to_string())?
            .list_conversations()
            .map_err(|e| e.to_string())?,
    )
}

#[tauri::command]
fn create_conversation(state: State<AppState>, title: String) -> CommandResult<Value> {
    let _operation = state.operations.begin()?;
    as_json(
        state
            .assistant
            .lock()
            .map_err(|e| e.to_string())?
            .create_conversation(&title)
            .map_err(|e| e.to_string())?,
    )
}

#[tauri::command]
fn get_messages(state: State<AppState>, conversation_id: String) -> CommandResult<Value> {
    let _operation = state.operations.begin()?;
    as_json(
        state
            .assistant
            .lock()
            .map_err(|e| e.to_string())?
            .messages(&conversation_id)
            .map_err(|e| e.to_string())?,
    )
}

#[tauri::command]
fn append_message(
    state: State<AppState>,
    conversation_id: String,
    role: String,
    content: String,
    provider: Option<String>,
    model: Option<String>,
) -> CommandResult<Value> {
    let _operation = state.operations.begin()?;
    as_json(
        state
            .assistant
            .lock()
            .map_err(|e| e.to_string())?
            .append_message(
                &conversation_id,
                &role,
                &content,
                provider.as_deref(),
                model.as_deref(),
            )
            .map_err(|e| e.to_string())?,
    )
}

#[tauri::command]
fn build_chat_context(
    state: State<AppState>,
    conversation_id: String,
    max_chars: usize,
) -> CommandResult<Value> {
    let _operation = state.operations.begin()?;
    as_json(
        state
            .assistant
            .lock()
            .map_err(|e| e.to_string())?
            .build_context(&conversation_id, max_chars)
            .map_err(|e| e.to_string())?,
    )
}

fn find_legacy_file() -> Option<PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(index) = args.iter().position(|arg| arg == "--legacy-file") {
        return args
            .get(index + 1)
            .map(PathBuf::from)
            .filter(|p| p.is_file());
    }
    let mut candidates = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("data/fiis_data.json"));
        candidates.push(cwd.join("../data/fiis_data.json"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join("data/fiis_data.json"));
            // A development executable can be opened outside the checkout's
            // working directory. Only recognize ancestors with the legacy
            // application's structure; never bundle personal data in the exe.
            for ancestor in parent.ancestors().take(4) {
                if ancestor.join("fiis_tracker").is_dir() {
                    candidates.push(ancestor.join("data/fiis_data.json"));
                }
            }
        }
    }
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .and_then(|p| p.canonicalize().ok())
}

/// Discover an optional local statement without including personal files in the build.
/// Once imported, SQLite is authoritative; startup never resets a saved position.
fn find_position_file(directory: &Path, legacy: Option<&Path>) -> Option<PathBuf> {
    let mut directories = vec![directory.to_path_buf()];
    if let Some(parent) = legacy.and_then(Path::parent) {
        if parent != directory {
            directories.push(parent.to_path_buf());
        }
    }
    for candidate_directory in directories {
        let Ok(entries) = fs::read_dir(candidate_directory) else {
            continue;
        };
        let mut candidates: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file()
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("posicao_") && name.ends_with(".json"))
            })
            .collect();
        candidates.sort();
        if let Some(path) = candidates.last().and_then(|path| path.canonicalize().ok()) {
            return Some(path);
        }
    }
    None
}

/// Startup must not replace a manually imported position, including when its
/// acquisition ledger is still empty and a complete legacy backup is nearby.
fn import_legacy_on_startup(
    repository: &Repository,
    legacy: Option<&Path>,
) -> ivestfii_core::Result<Option<Value>> {
    let snapshot = repository.snapshot()?;
    if snapshot.broker_position.is_some() || !snapshot.cash_pockets.is_empty() {
        return Ok(None);
    }
    Ok(
        legacy.map(|path| match repository.import_legacy_file(path) {
            Ok(receipt) => serde_json::to_value(receipt)
                .unwrap_or(json!({"error":"Não foi possível ler recibo"})),
            Err(error) => json!({"error":error.to_string()}),
        }),
    )
}

fn show_window(app: &tauri::AppHandle) {
    let _operation = if let Some(state) = app.try_state::<AppState>() {
        let permit = match state.operations.begin() {
            Ok(permit) => permit,
            Err(_) => return,
        };
        if let Ok(mut at) = state.last_window_action.lock() {
            *at = std::time::Instant::now();
        }
        Some(permit)
    } else {
        None
    };
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Verifies the actual native executable's storage path without launching a UI,
/// touching autostart, contacting providers, or writing to the legacy source.
fn verify_storage(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let value = |name: &str| -> Result<PathBuf, Box<dyn std::error::Error>> {
        let index = args
            .iter()
            .position(|a| a == name)
            .ok_or(format!("Falta {name}"))?;
        Ok(PathBuf::from(
            args.get(index + 1)
                .ok_or(format!("Falta valor de {name}"))?,
        ))
    };
    let directory = value("--data-dir")?;
    fs::create_dir_all(&directory)?;
    let source = value("--legacy-file")?;
    let repository = Repository::open(directory.join("portfolio.sqlite"))?;
    let receipt = repository.import_legacy_file(source)?;
    if args.iter().any(|arg| arg == "--position-file") {
        repository.import_broker_position_file(value("--position-file")?)?;
    }
    let snapshot = repository.snapshot()?;
    fs::write(
        directory.join("verified-snapshot.json"),
        serde_json::to_vec_pretty(&snapshot)?,
    )?;
    fs::write(
        directory.join("verified-export.json"),
        repository.export_legacy_json()?,
    )?;
    let assistant = AssistantStore::open(directory.join("assistant.sqlite"))?;
    println!(
        "{}",
        json!({"status":"ok","fundCount":snapshot.funds.len(),"recordCount":snapshot.record_count,"hasBrokerPosition":snapshot.broker_position.is_some(),"migration":receipt,"conversations":assistant.list_conversations()?.len()})
    );
    Ok(())
}

pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    // The packaged desktop executable also hosts its MCP transport. Providers
    // launch this mode with pipes; no sidecar install, GUI or autostart needed.
    if args.get(1).is_some_and(|arg| arg == "--mcp") {
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            if ![4, 6].contains(&args.len())
                || args[2] != "--data-dir"
                || (args.len() == 6 && args[4] != "--wallet-id")
            {
                return Err("Uso: ivestfii-desktop --mcp --data-dir <diretório>".into());
            }
            let mut server = ivestfii_mcp::McpServer::open_for_wallet(
                Path::new(&args[3]),
                args.get(5).map(String::as_str),
            )?;
            ivestfii_mcp::run(
                &mut server,
                std::io::stdin().lock(),
                std::io::stdout().lock(),
            )?;
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("Falha no MCP local: {error}");
            std::process::exit(1);
        }
        return;
    }
    if args.iter().any(|arg| arg == "--verify-storage") {
        if let Err(error) = verify_storage(&args) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_window(app)
        }))
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .args(["--autostart"])
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .setup(|app| {
            let directory = match std::env::var_os("IVESTFII_DATA_DIR") {
                Some(path) => PathBuf::from(path),
                None => app.path().app_local_data_dir()?,
            };
            fs::create_dir_all(&directory)?;
            let settings_path = directory.join("settings.json");
            let first_run = !settings_path.exists();
            let settings = read_settings(&settings_path).map_err(std::io::Error::other)?;
            let wallets = WalletCatalog::open(&directory)?;
            let repository = wallets.repository(None)?;
            let legacy = find_legacy_file();
            // A startup import failure stays visible to the UI; it must never
            // replace an existing portfolio or turn invalid data into empty data.
            let migration = import_legacy_on_startup(&repository, legacy.as_deref())?;
            let position_path = find_position_file(&directory, legacy.as_deref());
            let position_import = if repository.snapshot()?.broker_position.is_none() {
                position_path.as_ref().map(|path| {
                    match repository.import_broker_position_file(path) {
                        Ok(_) => json!({"status":"ok"}),
                        Err(error) => json!({"error":error.to_string()}),
                    }
                })
            } else {
                None
            };
            let assistant = AssistantStore::open(directory.join("assistant.sqlite"))?;
            let economics = EconomicService::new(directory.join("public-data/economics.json"))
                .map_err(std::io::Error::other)?;
            let news = NewsService::new(directory.join("public-data/news.json"))
                .map_err(std::io::Error::other)?;
            let runs = RunManager::new(
                directory.clone(),
                std::env::current_exe()?,
                vec![
                    "--mcp".into(),
                    "--data-dir".into(),
                    directory.to_string_lossy().into_owned(),
                ],
            )
            .map_err(std::io::Error::other)?;
            if first_run {
                if settings.autostart {
                    app.autolaunch().enable()?;
                }
                persist_settings(&settings_path, &settings).map_err(std::io::Error::other)?;
            }
            let start_hidden =
                std::env::args().any(|a| a == "--autostart") && settings.start_minimized;
            let updater = updates::UpdateManager::new(
                app.package_info().version.to_string(),
                updates::is_installed_release(&std::env::current_exe()?),
            );
            // Moving from a development executable to the installed stable
            // path must not leave an enabled startup entry pointing at target/.
            if updater.enabled() && settings.autostart {
                app.autolaunch().enable()?;
            }
            app.manage(AppState {
                wallets: Mutex::new(wallets),
                assistant: Mutex::new(assistant),
                settings: Mutex::new(settings),
                settings_path,
                data_directory: directory,
                legacy_path: legacy,
                migration: Mutex::new(migration),
                position_path,
                position_import: Mutex::new(position_import),
                runs,
                economics,
                news,
                updates: updater.clone(),
                operations: operations::OperationGate::default(),
                frontend_unsettled: std::sync::atomic::AtomicBool::new(true),
                last_window_action: Mutex::new(std::time::Instant::now()),
            });
            start_automatic_updates(app.handle().clone(), updater);

            let open = MenuItem::with_id(app, "open", "Abrir IvestFII", true, None::<&str>)?;
            let configure =
                MenuItem::with_id(app, "settings", "Configurações", true, None::<&str>)?;
            let sync = MenuItem::with_id(
                app,
                "sync",
                "Sincronização ainda não configurada",
                false,
                None::<&str>,
            )?;
            let quit = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &configure, &sync, &quit])?;
            let icon = tauri::image::Image::new_owned(tray_rgba(), 32, 32);
            TrayIconBuilder::with_id("ivestfii-tray")
                .icon(icon)
                .tooltip("IvestFII · carteira local")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_window(app),
                    "settings" => {
                        show_window(app);
                        let _ = app.emit("navigate", "settings");
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                    ) {
                        show_window(tray.app_handle());
                    }
                })
                .build(app)?;
            if !start_hidden {
                show_window(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                if let Ok(mut at) = state.last_window_action.lock() {
                    *at = std::time::Instant::now();
                }
                if state
                    .settings
                    .lock()
                    .map(|s| s.close_to_tray)
                    .unwrap_or(false)
                {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_portfolio,
            get_economic_cache,
            refresh_economic_indicators,
            fetch_cdi_reference,
            get_news_cache,
            refresh_news,
            get_market_data_settings,
            get_update_status,
            check_for_updates,
            install_pending_update,
            set_update_activity,
            save_brapi_key,
            clear_brapi_key,
            save_cvm_mapping,
            get_quote_cache,
            refresh_quotes,
            get_dividend_cache,
            get_monthly_income,
            get_fii_performance,
            refresh_fii_performance,
            get_cvm_cache,
            refresh_cvm_data,
            refresh_public_data_if_due,
            open_public_source,
            get_wallets,
            create_wallet,
            update_wallet,
            get_wealth_overview,
            get_investment_profile,
            save_investment_profile,
            get_cash_pockets,
            get_cash_performance,
            save_cash_pocket,
            simulate_cash_pocket,
            import_legacy,
            import_broker_position,
            export_portfolio,
            choose_import_file,
            export_portfolio_file,
            upsert_fund,
            save_record,
            preview_simulation_purchases,
            confirm_simulation_purchases,
            get_simulation_purchase_confirmations,
            project_income,
            get_settings,
            save_settings,
            get_app_info,
            get_providers,
            list_conversations,
            create_conversation,
            get_messages,
            append_message,
            build_chat_context,
            start_chat_turn,
            get_chat_run,
            cancel_chat_turn,
            get_tool_events
        ])
        .build(tauri::generate_context!())
        .expect("Não foi possível iniciar o IvestFII")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                if let Some(state) = app.try_state::<AppState>() {
                    state.runs.shutdown();
                }
            }
        });
}

fn tray_rgba() -> Vec<u8> {
    let mut rgba = vec![0_u8; 32 * 32 * 4];
    for y in 0..32 {
        for x in 0..32 {
            let bar = (7..11).contains(&x) && (19..26).contains(&y)
                || (14..18).contains(&x) && (13..26).contains(&y)
                || (21..25).contains(&x) && (6..26).contains(&y);
            let color = if bar {
                [54, 211, 175, 255]
            } else {
                [12, 24, 39, 255]
            };
            rgba[(y * 32 + x) * 4..(y * 32 + x) * 4 + 4].copy_from_slice(&color);
        }
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn displayed_real_result_is_captured_once_without_changing_financial_rows() {
        let root = tempfile::tempdir().unwrap();
        let repository = Repository::open(root.path().join("portfolio.sqlite")).unwrap();
        let source = root.path().join("synthetic.json");
        fs::write(&source, r#"{"fiis":[{"id":"f","ticker":"TEST11","name":"Fundo sintético","sector":"Teste","entries":[{"id":"e","month":"2020-05","purchase_date":"2020-05-04","cotas_added":"10","price_per_cota":"10","dividend_per_cota":"0","dividend_total":"5","notes":""}]}]}"#).unwrap();
        repository.import_legacy_file(&source).unwrap();
        let before = repository.snapshot().unwrap();
        let pending = serde_json::to_value(repository.pending_changes().unwrap()).unwrap();
        let panel: QuotePanel = serde_json::from_value(json!({"items":[{
            "symbol":"TEST11","status":"cached","error":null,
            "observation":{"symbol":"TEST11","price":"12","currency":"BRL",
                "quotedAt":"2020-06-30T21:00:00Z","fetchedAt":"2020-07-01T12:00:00Z",
                "source":"brapi","sourceUrl":"https://brapi.dev/","delayMinutes":30}
        }],"fetchedAt":"2020-07-01T12:00:00Z","credentialConfigured":false,"errors":[]}))
        .unwrap();
        for _ in 0..2 {
            let snapshot = repository.snapshot().unwrap();
            let report = performance::build_report(
                &snapshot,
                &panel,
                None,
                chrono::NaiveDate::from_ymd_opt(2020, 7, 1).unwrap(),
                vec![],
            )
            .unwrap();
            let report = remember_portfolio_observation(&repository, report);
            assert!(report.history_observation.is_some());
        }
        let after = repository.snapshot().unwrap();
        assert_eq!(after.performance_history.len(), 1);
        assert_eq!(after.performance_history[0].portfolio_result, "25");
        assert_eq!(
            serde_json::to_value(after.funds).unwrap(),
            serde_json::to_value(before.funds).unwrap()
        );
        assert_eq!(
            serde_json::to_value(after.totals).unwrap(),
            serde_json::to_value(before.totals).unwrap()
        );
        assert_eq!(
            serde_json::to_value(repository.pending_changes().unwrap()).unwrap(),
            pending
        );
    }

    #[test]
    fn startup_keeps_a_cash_only_wallet_authoritative() {
        let root = tempfile::tempdir().unwrap();
        let repository = Repository::open(root.path().join("portfolio.sqlite")).unwrap();
        repository
            .upsert_cash_pocket(CashPocketInput {
                id: None,
                name: "Reserva".into(),
                institution: "Instituição de teste".into(),
                balance: "1000".into(),
                total_deposits: None,
                total_withdrawals: None,
                balance_date: None,
                cash_flows: None,
                clear_reconciliation: false,
                cdi_percent: "120".into(),
                annual_cdi_rate: None,
                rate_date: None,
                rate_source: None,
                rate_source_url: None,
                rate_fetched_at: None,
                notes: "".into(),
            })
            .unwrap();
        let legacy = root.path().join("fiis_data.json");
        fs::write(
            &legacy,
            br#"{"fiis":[{"ticker":"TEST11","name":"Arquivo antigo","entries":[]}]}"#,
        )
        .unwrap();
        let before = repository.export_legacy_json().unwrap();
        assert!(import_legacy_on_startup(&repository, Some(&legacy))
            .unwrap()
            .is_none());
        assert_eq!(repository.export_legacy_json().unwrap(), before);
    }
    #[test]
    fn old_settings_keep_new_defaults() {
        let settings: AppSettings = serde_json::from_str("{\"autostart\":false}").unwrap();
        assert!(!settings.autostart);
        assert!(settings.close_to_tray);
        assert!(settings.start_minimized);
        assert!(settings.auto_update);
    }

    #[test]
    fn public_source_links_cannot_open_files_commands_or_other_hosts() {
        for url in [
            "https://api.bcb.gov.br/dados/serie/bcdata.sgs.4389/dados/ultimos/1?formato=json",
            "https://agenciabrasil.ebc.com.br/economia/noticia/2026-10/exemplo",
            "https://sistemaswebb3-listados.b3.com.br/fundsListedProxy/Search/GetEventsCorporateActions/exemplo",
        ] {
            assert_eq!(public_source_url(url).unwrap(), url);
        }
        for url in [
            "http://api.bcb.gov.br/",
            "file:///C:/Windows/System32/cmd.exe",
            "javascript:alert(1)",
            "mailto:teste@example.com",
            "https://example.com/?fonte=api.bcb.gov.br",
            "https://api.bcb.gov.br.example.com/",
            "https://sistemaswebb3-listados.b3.com.br.example.com/",
            "https://api.bcb.gov.br@evil.example/",
            "https://user:secret@api.bcb.gov.br/",
            "https://api.bcb.gov.br:8080/",
            "https://127.0.0.1/",
        ] {
            assert!(public_source_url(url).is_err(), "{url}");
        }
    }

    #[test]
    fn position_discovery_prefers_local_data_and_ignores_backups() {
        let root = tempfile::tempdir().unwrap();
        let local = root.path().join("app-data");
        let legacy = root.path().join("legacy");
        fs::create_dir_all(&local).unwrap();
        fs::create_dir_all(&legacy).unwrap();
        let legacy_file = legacy.join("fiis_data.json");
        fs::write(&legacy_file, "{}").unwrap();
        fs::write(legacy.join("fiis_backup_2026.json"), "{}").unwrap();
        assert!(find_position_file(&local, Some(&legacy_file)).is_none());
        let discovered = legacy.join("posicao_corretora_2026-01-01.json");
        fs::write(&discovered, "{}").unwrap();
        assert_eq!(
            find_position_file(&local, Some(&legacy_file)),
            Some(discovered.canonicalize().unwrap())
        );
        let local_position = local.join("posicao_corretora_2026-02-01.json");
        fs::write(&local_position, "{}").unwrap();
        assert_eq!(
            find_position_file(&local, Some(&legacy_file)),
            Some(local_position.canonicalize().unwrap())
        );
    }

    #[test]
    fn startup_preserves_saved_position_without_ledger() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("app-data/portfolio.sqlite");
        let repository = Repository::open(&database).unwrap();
        let position = json!({
            "referenceDate":"2025-06-30","institution":"Corretora manual",
            "importedAt":"2025-06-30T12:00:00Z","sourcePath":"synthetic-position.json",
            "sourceHash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "overview":{
                "updatedTime":"09:00","capturedTime":"09:01","totalWithCash":"170",
                "invested":"160","cash":"10","listedFunds":"100","fixedIncome":"60","pendingIncome":"0"
            },
            "listedFunds":{
                "capturedTime":"09:02","marketValue":"100","costBasis":"110",
                "unrealizedResult":"-10","dailyResult":"0","holdings":[{
                    "ticker":"TEST11","shares":"10","averagePrice":"11","price":"10",
                    "marketValue":"100","unrealizedResult":"-10","costBasis":"110"
                }]
            },
            "fixedIncome":{
                "name":"Fundo exemplo","nameTruncated":false,"balance":"60","netBalance":"59",
                "reportedEarnings":"5","return12Months":"10","redemptionConversion":"D+0","redemptionSettlement":"D+1"
            },
            "warnings":[]
        });
        let position_file = root.path().join("manual-position.json");
        fs::write(&position_file, serde_json::to_vec(&position).unwrap()).unwrap();
        repository
            .import_broker_position_file(&position_file)
            .unwrap();
        let before = repository.export_legacy_json().unwrap();
        let pending = repository.pending_change_count().unwrap();
        let mut older_position = position.clone();
        older_position["institution"] = json!("Corretora do backup antigo");
        let legacy = json!({
            "fiis":[{"ticker":"TEST11","name":"Histórico de teste","entries":[{
                "month":"2025-01","cotas_added":10,"price_per_cota":11,
                "dividend_per_cota":0,"dividend_total":0
            }]}],
            "broker_position":older_position
        });
        let legacy_file = root.path().join("fiis_data.json");
        let original = serde_json::to_vec(&legacy).unwrap();
        fs::write(&legacy_file, &original).unwrap();
        drop(repository);
        // Reopening matches the startup path, rather than relying on memory.
        let repository = Repository::open(&database).unwrap();
        assert!(import_legacy_on_startup(&repository, Some(&legacy_file))
            .unwrap()
            .is_none());
        assert_eq!(repository.export_legacy_json().unwrap(), before);
        assert!(repository.snapshot().unwrap().funds.is_empty());
        assert_eq!(repository.pending_change_count().unwrap(), pending);
        assert_eq!(fs::read(&legacy_file).unwrap(), original);
        assert_eq!(
            fs::read_dir(database.parent().unwrap().join("backups"))
                .unwrap()
                .count(),
            1
        );

        // Explicit manual restoration still works; only startup is guarded.
        let receipt = repository.import_legacy_file(&legacy_file).unwrap();
        assert_eq!(receipt.record_count, 1);
        assert_eq!(
            repository
                .snapshot()
                .unwrap()
                .broker_position
                .unwrap()
                .institution,
            "Corretora do backup antigo"
        );

        // A fresh installation still imports the same complete legacy backup.
        let fresh = Repository::open(root.path().join("fresh/portfolio.sqlite")).unwrap();
        let receipt = import_legacy_on_startup(&fresh, Some(&legacy_file))
            .unwrap()
            .unwrap();
        assert_eq!(receipt["recordCount"], 1);
        assert_eq!(
            fresh
                .snapshot()
                .unwrap()
                .broker_position
                .unwrap()
                .institution,
            "Corretora do backup antigo"
        );
    }
}
