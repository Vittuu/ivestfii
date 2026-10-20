//! Read-only totals for the selected wallet's ledger and reconciled cash pockets.
//! Gross deposits are acquisition funding, not the current principal balance.
//! Simulated CDI earnings and imported broker totals are deliberately excluded.
use super::{
    add, decimal_string, div, mul, parse_decimal, parse_nonnegative, performance::FiiPerformance,
    CashPocket, CoreError, PortfolioSnapshot, Result,
};
use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CashPerformanceItem {
    pub id: String,
    pub name: String,
    pub balance: String,
    pub invested: Option<String>,
    pub withdrawals: Option<String>,
    pub income: Option<String>,
    pub balance_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CashPerformance {
    pub balance: String,
    pub invested: Option<String>,
    pub income: Option<String>,
    pub withdrawals: Option<String>,
    pub known_invested: String,
    pub known_income: String,
    pub count: usize,
    pub missing_basis_count: usize,
    pub items: Vec<CashPerformanceItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioPerformance {
    pub status: String,
    pub invested: Option<String>,
    pub known_invested: String,
    pub income: Option<String>,
    pub known_income: String,
    pub current_value: Option<String>,
    pub known_current_value: String,
    pub total_result: Option<String>,
    pub known_result: String,
    pub total_return_percent: Option<String>,
    pub withdrawals: Option<String>,
    pub cash: CashPerformance,
    pub issues: Vec<String>,
}

fn subtract(left: Decimal, right: Decimal) -> Result<Decimal> {
    left.checked_sub(right).ok_or(CoreError::ArithmeticOverflow)
}

fn balance_date(value: &str) -> Result<()> {
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| CoreError::Validation("Data do saldo da caixinha inválida.".into()))?;
    if date.year() < 1 || date.format("%Y-%m-%d").to_string() != value {
        return Err(CoreError::Validation(
            "Data do saldo da caixinha inválida.".into(),
        ));
    }
    // The persistence layer validates today's boundary in Brasília. This pure
    // calculation has no clock and does not substitute updated_at for a date.
    Ok(())
}

fn fii_basis_completeness(
    snapshot: &PortfolioSnapshot,
    fii: &FiiPerformance,
) -> Result<(bool, bool)> {
    let cutoff = fii
        .as_of
        .as_deref()
        .map(|value| {
            NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .map(|date| date.format("%Y-%m").to_string())
                .map_err(|_| CoreError::Validation("Data dos FIIs inválida.".into()))
        })
        .transpose()?;
    let mut cost_complete = true;
    let mut future = false;
    let mut full_cost = Decimal::ZERO;
    let mut full_income = Decimal::ZERO;
    for fund in &snapshot.funds {
        full_income = add(
            full_income,
            parse_nonnegative(&fund.dividends_received, "Proventos do histórico")?,
        )?;
        for entry in &fund.entries {
            let quantity = parse_nonnegative(&entry.cotas_added, "Quantidade de cotas")?;
            let price = parse_nonnegative(&entry.price_per_cota, "Preço por cota")?;
            if quantity > Decimal::ZERO && price.is_zero() {
                cost_complete = false;
            }
            if cutoff.as_ref().is_some_and(|month| &entry.month > month) {
                future = true;
            }
            full_cost = add(full_cost, mul(quantity, price)?)?;
        }
    }
    // A calculation without common quotes may still have excluded future
    // records relative to its clock. Do not relabel a dated subset as a total.
    if !snapshot.funds.is_empty() && full_cost != parse_nonnegative(&fii.invested, "Custo de FIIs")?
    {
        cost_complete = false;
    }
    let income_complete = snapshot.funds.is_empty()
        || full_income == parse_nonnegative(&fii.dividends, "Proventos de FIIs")?;
    Ok((cost_complete && !future, income_complete && !future))
}

/// A known zero is explicit. An absent or partially supplied reconciliation is
/// unknown, while the manually recorded balance remains available independently.
pub fn calculate_cash_performance(pockets: &[CashPocket]) -> Result<CashPerformance> {
    let mut balance = Decimal::ZERO;
    let mut invested = Decimal::ZERO;
    let mut income = Decimal::ZERO;
    let mut withdrawals = Decimal::ZERO;
    let mut missing_basis_count = 0;
    let mut items = Vec::with_capacity(pockets.len());
    let mut seen = HashSet::new();
    for pocket in pockets {
        if !seen.insert(&pocket.id) {
            return Err(CoreError::Validation(
                "Há caixinhas duplicadas no cálculo da carteira.".into(),
            ));
        }
        let current = parse_nonnegative(&pocket.balance, "Saldo da caixinha")?;
        balance = add(balance, current)?;
        if let Some(value) = &pocket.balance_date {
            balance_date(value)?;
        }
        let reconciliation = match (
            &pocket.total_deposits,
            &pocket.total_withdrawals,
            &pocket.balance_date,
        ) {
            (Some(deposits), Some(withdrawn), Some(_)) => {
                let deposits = parse_nonnegative(deposits, "Total depositado na caixinha")?;
                let withdrawn = parse_nonnegative(withdrawn, "Total retirado da caixinha")?;
                let profit = subtract(add(current, withdrawn)?, deposits)?;
                invested = add(invested, deposits)?;
                income = add(income, profit)?;
                withdrawals = add(withdrawals, withdrawn)?;
                Some((deposits, withdrawn, profit))
            }
            _ => {
                missing_basis_count += 1;
                None
            }
        };
        items.push(CashPerformanceItem {
            id: pocket.id.clone(),
            name: pocket.name.clone(),
            balance: decimal_string(current),
            invested: reconciliation.map(|(deposits, _, _)| decimal_string(deposits)),
            withdrawals: reconciliation.map(|(_, withdrawn, _)| decimal_string(withdrawn)),
            income: reconciliation.map(|(_, _, profit)| decimal_string(profit)),
            balance_date: pocket.balance_date.clone(),
        });
    }
    let complete = missing_basis_count == 0;
    Ok(CashPerformance {
        balance: decimal_string(balance),
        invested: complete.then(|| decimal_string(invested)),
        income: complete.then(|| decimal_string(income)),
        withdrawals: complete.then(|| decimal_string(withdrawals)),
        known_invested: decimal_string(invested),
        known_income: decimal_string(income),
        count: pockets.len(),
        missing_basis_count,
        items,
    })
}

/// Sum component results, never a broker's aggregate snapshot, to avoid counting
/// imported positions, cash balances or reported fixed-income earnings twice.
/// Components retain their own observation dates; this is not a dated benchmark.
pub fn calculate_portfolio_performance(
    snapshot: &PortfolioSnapshot,
    fii: &FiiPerformance,
) -> Result<PortfolioPerformance> {
    let cash = calculate_cash_performance(&snapshot.cash_pockets)?;
    let empty_fii = fii.status == "empty";
    let (fii_cost_complete, fii_income_complete) = fii_basis_completeness(snapshot, fii)?;
    let fii_invested = if empty_fii {
        Decimal::ZERO
    } else {
        parse_nonnegative(&fii.invested, "Custo de FIIs")?
    };
    let fii_income = if empty_fii {
        Decimal::ZERO
    } else {
        parse_nonnegative(&fii.dividends, "Proventos de FIIs")?
    };
    let fii_market = if empty_fii {
        Some(Decimal::ZERO)
    } else {
        fii.market_value
            .as_deref()
            .map(|value| parse_nonnegative(value, "Valor de mercado dos FIIs"))
            .transpose()?
    };
    let fii_result = if empty_fii {
        Some(Decimal::ZERO)
    } else if !fii_cost_complete || !fii_income_complete {
        None
    } else {
        fii.total_result.as_deref().map(parse_decimal).transpose()?
    };
    let cash_balance = parse_nonnegative(&cash.balance, "Saldo das caixinhas")?;
    let cash_known_invested = parse_nonnegative(&cash.known_invested, "Depósitos conhecidos")?;
    let cash_known_income = parse_decimal(&cash.known_income)?;
    let cash_invested = cash.invested.as_deref().map(parse_decimal).transpose()?;
    let cash_income = cash.income.as_deref().map(parse_decimal).transpose()?;
    let known_invested = add(fii_invested, cash_known_invested)?;
    let known_income = add(fii_income, cash_known_income)?;
    let known_current_value = add(fii_market.unwrap_or(Decimal::ZERO), cash_balance)?;
    let known_result = add(fii_result.unwrap_or(Decimal::ZERO), cash_known_income)?;
    let invested = if fii_cost_complete {
        cash_invested
            .map(|value| add(fii_invested, value))
            .transpose()?
    } else {
        None
    };
    let income = if fii_income_complete {
        cash_income
            .map(|value| add(fii_income, value))
            .transpose()?
    } else {
        None
    };
    let current_value = fii_market
        .map(|value| add(value, cash_balance))
        .transpose()?;
    let total_result = match (fii_result, cash_income) {
        (Some(result), Some(profit)) => Some(add(result, profit)?),
        _ => None,
    };
    let total_return_percent = match (invested, total_result) {
        (Some(cost), Some(result)) if cost > Decimal::ZERO => {
            Some(mul(div(result, cost)?, Decimal::ONE_HUNDRED)?)
        }
        _ => None,
    };
    // Benchmark-specific failures stay in the FII chart. Complete wallet
    // accounting values do not become unavailable because CDI is missing.
    let mut issues = vec![];
    if !fii_cost_complete {
        issues.push("O custo completo dos FIIs não está disponível: há compras sem preço ou valores fora do período calculado.".into());
    }
    if !fii_income_complete {
        issues.push("Os proventos de FIIs calculados não cobrem todo o histórico; a parcela conhecida foi mantida separada.".into());
    }
    if !empty_fii && fii_result.is_none() {
        issues.push("O resultado dos FIIs ainda está incompleto. Consulte as cotações e a conciliação no painel de FIIs.".into());
    }
    if cash.missing_basis_count > 0 {
        issues.push(format!("Faltam os depósitos, as retiradas ou a data do saldo de {} caixinha(s). Os totais completos de custo e resultado permanecem indisponíveis.", cash.missing_basis_count));
    }
    let complete =
        invested.is_some() && income.is_some() && current_value.is_some() && total_result.is_some();
    let status = if empty_fii && cash.count == 0 {
        "empty"
    } else if complete {
        "available"
    } else {
        "incomplete"
    };
    let withdrawals = cash.withdrawals.clone();
    Ok(PortfolioPerformance {
        status: status.into(),
        invested: invested.map(decimal_string),
        known_invested: decimal_string(known_invested),
        income: income.map(decimal_string),
        known_income: decimal_string(known_income),
        current_value: current_value.map(decimal_string),
        known_current_value: decimal_string(known_current_value),
        total_result: total_result.map(decimal_string),
        known_result: decimal_string(known_result),
        total_return_percent: total_return_percent.map(decimal_string),
        withdrawals,
        cash,
        issues,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{calculate_fund, MonthlyEntry, PortfolioTotals};

    fn pocket(
        id: &str,
        balance: &str,
        deposits: Option<&str>,
        withdrawals: Option<&str>,
    ) -> CashPocket {
        CashPocket {
            id: id.into(),
            name: format!("Caixinha sintética {id}"),
            institution: "Teste".into(),
            balance: balance.into(),
            cdi_percent: "120".into(),
            annual_cdi_rate: Some("14.9".into()),
            rate_date: Some("2026-01-01".into()),
            rate_source: "manual".into(),
            rate_source_url: None,
            rate_fetched_at: None,
            notes: "Dados sintéticos".into(),
            created_at: "2026-01-01T12:00:00Z".into(),
            updated_at: "2026-10-20T12:00:00Z".into(),
            total_deposits: deposits.map(str::to_owned),
            total_withdrawals: withdrawals.map(str::to_owned),
            balance_date: deposits.zip(withdrawals).map(|_| "2026-10-20".into()),
        }
    }

    fn record(id: &str, month: &str, quantity: &str, price: &str, income: &str) -> MonthlyEntry {
        MonthlyEntry {
            id: id.into(),
            month: month.into(),
            cotas_added: quantity.into(),
            purchase_date: None,
            price_per_cota: price.into(),
            dividend_per_cota: "0".into(),
            dividend_total: Some(income.into()),
            notes: String::new(),
        }
    }

    fn snapshot(pockets: Vec<CashPocket>) -> PortfolioSnapshot {
        PortfolioSnapshot {
            funds: vec![calculate_fund(
                "fund-fixture".into(),
                "TEST11".into(),
                "Fundo sintético".into(),
                "Teste".into(),
                vec![record("r1", "2026-01", "10", "10", "5")],
            )
            .unwrap()],
            // These aggregate/cash fields must never be added to the separate
            // market result and pockets, where they would count values twice.
            totals: PortfolioTotals {
                invested: "999999".into(),
                cotas: "999999".into(),
                dividends_received: "999999".into(),
                estimated_monthly_income: "999999".into(),
            },
            record_count: 1,
            broker_position: None,
            cash_pockets: pockets,
            cash_balance: "999999".into(),
        }
    }

    fn fii() -> FiiPerformance {
        FiiPerformance {
            // A missing benchmark does not make the FII accounting unavailable.
            status: "incomplete".into(),
            as_of: Some("2026-10-20".into()),
            start_month: Some("2026-01".into()),
            invested: "100".into(),
            dividends: "5".into(),
            market_value: Some("90".into()),
            price_result: Some("-10".into()),
            total_result: Some("-5".into()),
            total_return_percent: Some("-5".into()),
            cdi: None,
            difference: None,
            issues: vec!["Comparação CDI indisponível: fixture sem histórico.".into()],
            assumptions: vec![],
        }
    }

    fn empty_fii() -> FiiPerformance {
        FiiPerformance {
            status: "empty".into(),
            invested: "0".into(),
            dividends: "0".into(),
            market_value: None,
            price_result: None,
            total_result: None,
            total_return_percent: None,
            issues: vec![],
            ..fii()
        }
    }

    #[test]
    fn selected_wallet_combines_known_cash_and_fii_without_requiring_the_fii_benchmark() {
        let original = snapshot(vec![pocket("one", "220", Some("200"), Some("10"))]);
        let bytes = serde_json::to_vec(&original).unwrap();
        let report = calculate_portfolio_performance(&original, &fii()).unwrap();
        assert_eq!(report.status, "available");
        assert_eq!(report.invested.as_deref(), Some("300"));
        assert_eq!(report.income.as_deref(), Some("35"));
        assert_eq!(report.current_value.as_deref(), Some("310"));
        assert_eq!(report.total_result.as_deref(), Some("25"));
        assert_eq!(report.withdrawals.as_deref(), Some("10"));
        assert_eq!(report.cash.income.as_deref(), Some("30"));
        assert_eq!(report.known_invested, "300");
        assert_eq!(report.known_income, "35");
        assert_eq!(report.known_current_value, "310");
        assert_eq!(report.known_result, "25");
        assert!(report.total_return_percent.is_some());
        assert!(report.issues.is_empty());
        assert_eq!(serde_json::to_vec(&original).unwrap(), bytes);
    }

    #[test]
    fn legacy_unknown_basis_never_becomes_zero_or_inferred_principal() {
        let original = snapshot(vec![
            pocket("known", "220", Some("200"), Some("10")),
            pocket("legacy", "50", None, None),
        ]);
        let report = calculate_portfolio_performance(&original, &fii()).unwrap();
        assert_eq!(report.status, "incomplete");
        assert!(
            report.invested.is_none() && report.income.is_none() && report.total_result.is_none()
        );
        assert!(report.total_return_percent.is_none() && report.withdrawals.is_none());
        assert_eq!(report.current_value.as_deref(), Some("360"));
        assert_eq!(report.known_invested, "300");
        assert_eq!(report.known_income, "35");
        assert_eq!(report.known_result, "25");
        assert_eq!(report.cash.balance, "270");
        assert_eq!(report.cash.missing_basis_count, 1);
        assert_eq!(report.cash.items[1].balance, "50");
        assert!(report.cash.items[1].income.is_none());
        assert!(report.cash.items[1].invested.is_none());
        assert!(report.cash.items[1].balance_date.is_none());
    }

    #[test]
    fn missing_market_preserves_known_cost_income_and_cash_result_without_partial_total() {
        let original = snapshot(vec![pocket("cash", "220", Some("200"), Some("10"))]);
        let unavailable = FiiPerformance {
            market_value: None,
            total_result: None,
            price_result: None,
            total_return_percent: None,
            as_of: None,
            ..fii()
        };
        let report = calculate_portfolio_performance(&original, &unavailable).unwrap();
        assert_eq!(report.invested.as_deref(), Some("300"));
        assert_eq!(report.income.as_deref(), Some("35"));
        assert!(report.current_value.is_none() && report.total_result.is_none());
        assert_eq!(report.known_current_value, "220");
        assert_eq!(report.known_result, "30");
        assert!(report.total_return_percent.is_none());
    }

    #[test]
    fn cash_only_wallet_needs_no_quotes_and_no_pockets_is_known_zero() {
        let mut original = snapshot(vec![pocket("only", "220", Some("200"), Some("10"))]);
        original.funds.clear();
        let report = calculate_portfolio_performance(&original, &empty_fii()).unwrap();
        assert_eq!(report.status, "available");
        assert_eq!(report.invested.as_deref(), Some("200"));
        assert_eq!(report.income.as_deref(), Some("30"));
        assert_eq!(report.current_value.as_deref(), Some("220"));
        assert_eq!(report.total_result.as_deref(), Some("30"));
        assert_eq!(report.total_return_percent.as_deref(), Some("15"));
        original.cash_pockets.clear();
        let empty = calculate_portfolio_performance(&original, &empty_fii()).unwrap();
        assert_eq!(empty.status, "empty");
        assert_eq!(empty.invested.as_deref(), Some("0"));
        assert_eq!(empty.income.as_deref(), Some("0"));
        assert_eq!(empty.current_value.as_deref(), Some("0"));
        assert_eq!(empty.total_result.as_deref(), Some("0"));
        assert_eq!(empty.cash.missing_basis_count, 0);
        assert!(empty.total_return_percent.is_none());
    }

    #[test]
    fn principal_redemptions_interest_withdrawals_and_closed_accounts_keep_the_same_gain() {
        for (balance, withdrawals) in [("1100", "0"), ("700", "400"), ("600", "500"), ("0", "1100")]
        {
            let cash = calculate_cash_performance(&[pocket(
                "cash",
                balance,
                Some("1000"),
                Some(withdrawals),
            )])
            .unwrap();
            assert_eq!(cash.income.as_deref(), Some("100"));
            assert_eq!(cash.invested.as_deref(), Some("1000"));
            assert_eq!(cash.withdrawals.as_deref(), Some(withdrawals));
            assert_eq!(cash.balance, balance);
        }
    }

    #[test]
    fn cash_loss_known_zero_and_missing_zero_balance_are_distinct() {
        let loss =
            calculate_cash_performance(&[pocket("loss", "90", Some("100"), Some("0"))]).unwrap();
        assert_eq!(loss.income.as_deref(), Some("-10"));
        let mut original = snapshot(vec![pocket("zero", "0", Some("0"), Some("0"))]);
        original.funds.clear();
        let zero = calculate_portfolio_performance(&original, &empty_fii()).unwrap();
        assert_eq!(zero.status, "available");
        assert_eq!(zero.income.as_deref(), Some("0"));
        assert!(zero.total_return_percent.is_none());
        let unknown = calculate_cash_performance(&[pocket("unknown", "0", None, None)]).unwrap();
        assert!(unknown.income.is_none());
        assert_eq!(unknown.missing_basis_count, 1);
    }

    #[test]
    fn incomplete_fii_cost_and_postdated_records_cannot_be_labelled_full_wallet_cost() {
        let mut original = snapshot(vec![pocket("cash", "220", Some("200"), Some("10"))]);
        original.funds[0]
            .entries
            .push(record("zero-cost", "2026-02", "2", "0", "0"));
        let report = calculate_portfolio_performance(&original, &fii()).unwrap();
        assert!(report.invested.is_none() && report.total_result.is_none());
        assert_eq!(report.known_invested, "300");
        assert_eq!(report.income.as_deref(), Some("35"));
        assert_eq!(report.known_result, "30");
        original.funds[0].entries.pop();
        original.funds[0]
            .entries
            .push(record("future", "2026-11", "2", "10", "2"));
        original.funds[0].dividends_received = "7".into();
        let future = calculate_portfolio_performance(&original, &fii()).unwrap();
        assert!(
            future.invested.is_none() && future.income.is_none() && future.total_result.is_none()
        );
        assert_eq!(future.known_income, "35");
        let no_quote_date = FiiPerformance {
            as_of: None,
            ..fii()
        };
        let undated = calculate_portfolio_performance(&original, &no_quote_date).unwrap();
        assert!(undated.invested.is_none() && undated.income.is_none());
    }

    #[test]
    fn cash_reference_rate_and_update_time_cannot_create_realized_income_or_a_balance_date() {
        let mut legacy = pocket("legacy", "100", None, None);
        legacy.annual_cdi_rate = Some("1000".into());
        legacy.cdi_percent = "500".into();
        let cash = calculate_cash_performance(&[legacy]).unwrap();
        assert_eq!(cash.balance, "100");
        assert!(cash.income.is_none() && cash.items[0].balance_date.is_none());
        let mut partial = pocket("partial", "100", Some("100"), None);
        partial.balance_date = Some("2026-10-20".into());
        let cash = calculate_cash_performance(&[partial]).unwrap();
        assert_eq!(cash.known_invested, "0");
        assert_eq!(cash.missing_basis_count, 1);
    }

    #[test]
    fn malformed_or_duplicate_inputs_and_checked_overflow_are_rejected() {
        let same = pocket("same", "100", Some("100"), Some("0"));
        assert!(calculate_cash_performance(&[same.clone(), same]).is_err());
        let mut invalid = pocket("bad", "100", Some("100"), Some("0"));
        invalid.balance_date = Some("2026-02-30".into());
        assert!(calculate_cash_performance(&[invalid]).is_err());
        let overflow = pocket(
            "overflow",
            "79228162514264337593543950335",
            Some("0"),
            Some("1"),
        );
        assert!(matches!(
            calculate_cash_performance(&[overflow]),
            Err(CoreError::ArithmeticOverflow)
        ));
        let mut original = snapshot(vec![pocket("one", "1", Some("1"), Some("0"))]);
        original.funds.clear();
        let enormous = FiiPerformance {
            invested: "79228162514264337593543950335".into(),
            ..fii()
        };
        assert!(matches!(
            calculate_portfolio_performance(&original, &enormous),
            Err(CoreError::ArithmeticOverflow)
        ));
    }
}
