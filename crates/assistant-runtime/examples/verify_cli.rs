//! Live, opt-in subscription smoke test. Only a disposable synthetic portfolio is
//! exposed to providers. Never opens the application's actual financial data.
use ivestfii_assistant::AssistantStore;
use ivestfii_runtime::RunManager;
use serde_json::json;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "--mcp") {
        if ![4, 6].contains(&args.len())
            || args[2] != "--data-dir"
            || (args.len() == 6 && args[4] != "--wallet-id")
        {
            return Err("Invalid MCP arguments".into());
        }
        let mut server = ivestfii_mcp::McpServer::open_for_wallet(
            std::path::Path::new(&args[3]),
            args.get(5).map(String::as_str),
        )?;
        ivestfii_mcp::run(
            &mut server,
            std::io::stdin().lock(),
            std::io::stdout().lock(),
        )?;
        return Ok(());
    }
    let providers: Vec<&str> = args.get(1).map(|a| a.split(',').collect()).unwrap_or(vec![
        "codex",
        "claude",
        "antigravity",
    ]);
    let directory = tempfile::tempdir()?;
    let data_dir = directory.path().canonicalize()?;
    let _portfolio = ivestfii_core::Repository::open(data_dir.join("portfolio.sqlite"))?;
    let wallet_id = if args.iter().any(|arg| arg == "--wallet-smoke") {
        let catalog = ivestfii_core::WalletCatalog::open(&data_dir)?;
        let wallet = catalog.create(ivestfii_core::WalletInput {
            id: None,
            name: "Carteira sintética separada".into(),
            institution: "Instituição de teste".into(),
        })?;
        catalog.repository(Some(&wallet.id))?.upsert_cash_pocket(
            ivestfii_core::CashPocketInput {
                id: None,
                name: "Reserva sintética".into(),
                institution: "Instituição de teste".into(),
                balance: "10000".into(),
                total_deposits: None,
                total_withdrawals: None,
                balance_date: None,
                clear_reconciliation: false,
                cdi_percent: "120".into(),
                annual_cdi_rate: Some("10".into()),
                rate_date: Some("2026-01-01".into()),
                rate_source: None,
                rate_source_url: None,
                rate_fetched_at: None,
                notes: "Somente cenário sintético; não é taxa de mercado".into(),
            },
        )?;
        Some(wallet.id)
    } else {
        None
    };
    let store = AssistantStore::open(data_dir.join("assistant.sqlite"))?;
    let conversation = store.create_conversation("Validação sintética das CLIs")?;
    let manager = RunManager::new(
        data_dir.clone(),
        std::env::current_exe()?,
        vec![
            "--mcp".into(),
            "--data-dir".into(),
            data_dir.to_string_lossy().into_owned(),
        ],
    )?;
    println!(
        "{}",
        json!({"fixture":if wallet_id.is_some(){"scoped-synthetic-wallet-with-cdi-pocket"}else{"empty-disposable-portfolio"},"providers":providers})
    );
    let mut failures = 0;
    for (index, provider) in providers.into_iter().enumerate() {
        let content = if wallet_id.is_some() {
            "Teste técnico. Guarde CONTEXTO-7462. Use carteira_snapshot sem walletId para consultar a carteira selecionada neste turno. Em seguida use projetar_renda com cashPocketId da caixinha encontrada e businessDays=252, sem campos de FIIs. Responda somente o código, saldo informado da caixinha e saldo bruto simulado. O CDI é uma premissa hipotética. Não use ferramentas externas."
        } else if index == 0 {
            "Teste técnico. Guarde o código CONTEXTO-7462 para as próximas mensagens. Use a ferramenta MCP carteira_snapshot agora e diga apenas o código e a quantidade de fundos da carteira. Não use comandos, arquivos ou ferramentas externas."
        } else {
            "Continuamos a mesma conversa em outro provedor. Recupere do histórico o código informado pelo usuário antes, use a ferramenta MCP carteira_snapshot agora e responda apenas aquele código e a quantidade atual de fundos. Não use comandos, arquivos ou ferramentas externas."
        };
        let run = manager.start_for_wallet(
            conversation.id.clone(),
            provider.into(),
            None,
            content.into(),
            wallet_id.clone(),
            Arc::new(|_| {}),
        )?;
        let deadline = Instant::now() + Duration::from_secs(150);
        loop {
            let snapshot = manager.latest(&conversation.id)?.ok_or("Run missing")?;
            if snapshot.status != "running" {
                let tools = store
                    .tool_events(&conversation.id)?
                    .into_iter()
                    .filter(|t| t.task_id.as_deref() == Some(&run.id))
                    .collect::<Vec<_>>();
                let used_mcp = tools
                    .iter()
                    .any(|t| t.status == "completed" && t.tool_name.contains("carteira_snapshot"));
                let continuity = snapshot.text.contains("CONTEXTO-7462");
                let scoped_cash = wallet_id.is_none()
                    || tools.iter().any(|tool| {
                        tool.status == "completed"
                            && tool.tool_name.contains("projetar_renda")
                            && tool
                                .output
                                .as_ref()
                                .is_some_and(|output| output.to_string().contains("11211.65"))
                    });
                let success =
                    snapshot.status == "completed" && used_mcp && continuity && scoped_cash;
                println!(
                    "{}",
                    json!({"provider":provider,"status":snapshot.status,"directMcp":used_mcp,"historyRetained":continuity,"scopedCashSimulation":if wallet_id.is_some(){Some(scoped_cash)}else{None},"success":success,"error":snapshot.error,"response":snapshot.text,"tools":tools.iter().map(|t|json!({"name":t.tool_name,"status":t.status})).collect::<Vec<_>>()})
                );
                if !success {
                    failures += 1;
                }
                break;
            }
            if Instant::now() > deadline {
                manager.cancel(&run.id)?;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    manager.shutdown();
    if failures > 0 {
        return Err(
            format!("{failures} provider(s) did not pass the live MCP/history check").into(),
        );
    }
    Ok(())
}
