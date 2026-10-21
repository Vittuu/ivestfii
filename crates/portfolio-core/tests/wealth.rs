use ivestfii_core::{
    CashPocketInput, FundInput, RecordInput, Repository, WalletCatalog, WalletInput,
};

#[test]
fn wallet_summary_keeps_fii_cost_and_cash_balances_separate() {
    let temporary = tempfile::tempdir().unwrap();
    let catalog = WalletCatalog::open(temporary.path()).unwrap();
    let other = catalog
        .create(WalletInput {
            id: None,
            name: "Nubank".into(),
            institution: "Nubank".into(),
        })
        .unwrap();
    for (wallet, cost, balance) in [(None, "10", "1000"), (Some(other.id.as_str()), "20", "250")] {
        let repository = catalog.repository(wallet).unwrap();
        repository
            .upsert_fund(FundInput {
                id: None,
                ticker: "TEST11".into(),
                name: "Fundo de teste".into(),
                sector: "".into(),
            })
            .unwrap();
        let fund = repository.snapshot().unwrap().funds.remove(0);
        repository
            .save_monthly_record(RecordInput {
                id: None,
                fund_id: fund.id,
                month: "2026-01".into(),
                cotas_added: "1".into(),
                purchase_date: None,
                price_per_cota: cost.into(),
                dividend_per_cota: "0".into(),
                dividend_total: None,
                notes: "".into(),
            })
            .unwrap();
        repository
            .upsert_cash_pocket(CashPocketInput {
                id: None,
                name: "Reserva".into(),
                institution: "Instituição de teste".into(),
                balance: balance.into(),
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
    }
    let summary = catalog.wealth_overview().unwrap();
    assert_eq!(summary.wallets.len(), 2);
    assert_eq!(summary.totals.fii_invested, "30");
    assert_eq!(summary.totals.cash_balance, "1250");
    let nubank = summary
        .wallets
        .iter()
        .find(|item| item.wallet.id == other.id)
        .unwrap();
    assert_eq!(nubank.fii_invested, "20");
    assert_eq!(nubank.cash_balance, "250");
}

#[test]
fn an_empty_existing_database_cannot_silently_replace_a_registered_wallet() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("portfolio.sqlite");
    std::fs::write(&path, []).unwrap();
    assert!(Repository::open_existing(&path).is_err());
    assert_eq!(std::fs::metadata(path).unwrap().len(), 0);
}
