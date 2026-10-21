//! Monthly matched-flow results using real, dated whole-wallet observations.
//! Missing valuations are gaps, never interpolated from the current balance.
use crate::{
    add, decimal_string,
    income::IncomeEvent,
    mul, normalize_purchase_date, parse_decimal, parse_nonnegative,
    performance::{DailyCdiRate, FiiPerformance},
    portfolio_benchmark::{add_flow, calculate_flow_cdi, Flow, PortfolioBenchmark},
    portfolio_performance::PortfolioPerformance,
    CoreError, PortfolioSnapshot, Result,
};
use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioHistoryObservation {
    pub month: String,
    pub as_of: String,
    pub invested: String,
    pub portfolio_result: String,
    pub basis_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioEvolutionPoint {
    pub month: String,
    pub as_of: String,
    pub portfolio_result: Option<String>,
    pub cdi_result: Option<String>,
    pub invested: Option<String>,
    pub difference: Option<String>,
    pub status: String,
    pub issues: Vec<String>,
    pub estimated: bool,
    pub is_partial_month: bool,
    /// Public historical prices and declared movements, never a saved account balance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<crate::historical_valuation::HistoricalMonthlyProvenance>,
}

/// Latest real values, retaining each investment's own source date. This is
/// not a same-day monthly observation and has no matched-date CDI result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioLatestValuation {
    pub from_date: String,
    pub to_date: String,
    pub invested: String,
    pub portfolio_result: String,
    pub components: Vec<PortfolioLatestValuationComponent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioLatestValuationComponent {
    pub kind: String,
    pub id: Option<String>,
    pub name: String,
    pub as_of: String,
    pub result: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioEvolution {
    pub status: String,
    pub start_date: Option<String>,
    pub as_of: Option<String>,
    pub points: Vec<PortfolioEvolutionPoint>,
    pub issues: Vec<String>,
    pub assumptions: Vec<String>,
    #[serde(default)]
    pub latest_known: Option<PortfolioLatestValuation>,
}

fn invalid(message: &str) -> CoreError {
    CoreError::Validation(message.into())
}

fn date(value: &str) -> Result<NaiveDate> {
    let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| invalid("Data inválida no histórico da evolução da carteira."))?;
    if parsed.year() < 1 || parsed.format("%Y-%m-%d").to_string() != value {
        return Err(invalid(
            "Data inválida no histórico da evolução da carteira.",
        ));
    }
    Ok(parsed)
}

fn month_start(value: &str) -> Result<NaiveDate> {
    let parsed = date(&format!("{value}-01"))?;
    if parsed.format("%Y-%m").to_string() != value {
        return Err(invalid(
            "Mês inválido no histórico da evolução da carteira.",
        ));
    }
    Ok(parsed)
}

fn month_end(day: NaiveDate) -> Result<NaiveDate> {
    let next = if day.month() == 12 {
        NaiveDate::from_ymd_opt(day.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(day.year(), day.month() + 1, 1)
    };
    next.and_then(|next| next.pred_opt())
        .ok_or_else(|| invalid("Mês fora do intervalo suportado."))
}

/// Canonical storage validation; persistence also enforces its current-date
/// boundary. A fingerprint describes financial inputs, not a signed receipt.
pub fn validate_history_observation(value: &PortfolioHistoryObservation) -> Result<()> {
    month_start(&value.month)?;
    let observed = date(&value.as_of)?;
    if observed.format("%Y-%m").to_string() != value.month {
        return Err(invalid("A data do saldo não pertence ao mês do histórico."));
    }
    parse_nonnegative(&value.invested, "Valor aplicado no histórico")?;
    parse_decimal(&value.portfolio_result)?;
    if value.basis_fingerprint.len() != 64
        || !value
            .basis_fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid("A referência dos dados do histórico é inválida."));
    }
    Ok(())
}

fn purchase_day(entry: &crate::MonthlyEntry, cutoff: NaiveDate) -> Result<Option<NaiveDate>> {
    let quantity = parse_nonnegative(&entry.cotas_added, "Cotas compradas")?;
    if quantity.is_zero() {
        return Ok(None);
    }
    let month = month_start(&entry.month)?;
    if month > cutoff {
        return Ok(None);
    }
    let day = match normalize_purchase_date(entry.purchase_date.as_deref(), &entry.month, quantity)?
    {
        Some(value) => date(&value)?,
        None => month_end(month)?.min(cutoff),
    };
    Ok((day <= cutoff).then_some(day))
}

/// Ignore names, notes, cache timestamps and future movements. A retroactive
/// acquisition or income correction invalidates earlier derived observations.
fn basis_fingerprint(
    snapshot: &PortfolioSnapshot,
    events: &[IncomeEvent],
    cutoff: NaiveDate,
) -> Result<String> {
    let mut economic = vec!["portfolio-evolution-basis-v1".to_string()];
    let cutoff_month = cutoff.format("%Y-%m").to_string();
    let mut first_income_month: Option<String> = None;
    for fund in &snapshot.funds {
        let symbol = fund.ticker.to_uppercase();
        for entry in &fund.entries {
            month_start(&entry.month)?;
            if entry.month > cutoff_month {
                continue;
            }
            let purchase = purchase_day(entry, cutoff)?;
            let manual = entry.dividend_total.is_some()
                || !parse_nonnegative(&entry.dividend_per_cota, "Rendimento por cota")?.is_zero();
            if purchase.is_none() && !manual {
                continue;
            }
            if first_income_month
                .as_ref()
                .is_none_or(|month| entry.month < *month)
            {
                first_income_month = Some(entry.month.clone());
            }
            let quantity = purchase.map(|_| entry.cotas_added.as_str()).unwrap_or("0");
            let price = purchase
                .map(|_| entry.price_per_cota.as_str())
                .unwrap_or("0");
            economic.push(serde_json::to_string(&(
                "fii",
                &symbol,
                &entry.month,
                purchase.map(|value| value.to_string()),
                decimal_string(parse_nonnegative(quantity, "Cotas compradas")?),
                decimal_string(parse_nonnegative(price, "Preço de compra")?),
                decimal_string(parse_nonnegative(
                    &entry.dividend_per_cota,
                    "Rendimento por cota",
                )?),
                entry
                    .dividend_total
                    .as_deref()
                    .map(parse_decimal)
                    .transpose()?
                    .map(decimal_string),
            ))?);
        }
    }
    if let Some(position) = &snapshot.broker_position {
        if let Some(observation) = &position.fixed_income.latest_observation {
            let invested =
                parse_nonnegative(&observation.invested, "Valor aplicado no pós-fixado")?;
            if observation.contributions.is_empty()
                && (!invested.is_zero()
                    || !parse_nonnegative(&observation.balance, "Saldo do pós-fixado")?.is_zero())
            {
                // An undated position may already have existed at this cutoff.
                // Its current value is not a historical acquisition date.
                economic.push(serde_json::to_string(&(
                    "fixedIncome-unknown",
                    decimal_string(invested),
                ))?);
            }
            for contribution in &observation.contributions {
                let day = date(&contribution.application_date)?;
                if day <= cutoff {
                    economic.push(serde_json::to_string(&(
                        "fixedIncome",
                        day.to_string(),
                        decimal_string(parse_nonnegative(
                            &contribution.invested,
                            "Aporte no pós-fixado",
                        )?),
                    ))?);
                }
            }
        } else if !parse_nonnegative(&position.fixed_income.balance, "Saldo do pós-fixado")?
            .is_zero()
        {
            economic.push("fixedIncome-unknown".into());
        }
    }
    for pocket in &snapshot.cash_pockets {
        match &pocket.cash_flows {
            Some(flows) => {
                for flow in flows {
                    let day = date(&flow.date)?;
                    if day <= cutoff {
                        economic.push(serde_json::to_string(&(
                            "cash",
                            day.to_string(),
                            &flow.kind,
                            decimal_string(parse_nonnegative(
                                &flow.amount,
                                "Movimentação da caixinha",
                            )?),
                        ))?);
                    }
                }
            }
            None if pocket
                .total_deposits
                .as_deref()
                .map(parse_decimal)
                .transpose()?
                == Some(Decimal::ZERO)
                && pocket
                    .total_withdrawals
                    .as_deref()
                    .map(parse_decimal)
                    .transpose()?
                    == Some(Decimal::ZERO) => {}
            None => economic.push("cash-unknown".into()),
        }
    }
    // Hash resolved economic income, rather than provider identifiers. This
    // preserves manual precedence and conflict/duplicate handling exactly as
    // the result calculator while remaining portable across provider renames.
    if let Some(first) = first_income_month {
        let income = crate::income::calculate_monthly_income(
            snapshot,
            events,
            &first,
            &cutoff_month,
            cutoff,
        )?;
        for month in income.months {
            for fund in month.funds {
                if fund.status == "not_held" {
                    continue;
                }
                economic.push(serde_json::to_string(&(
                    "income",
                    &month.month,
                    fund.symbol.to_uppercase(),
                    fund.status,
                    fund.amount,
                ))?);
            }
        }
    }
    economic.sort();
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&economic)?)
    ))
}

/// Capture only a real, complete valuation of all active components on one
/// common date. Historical CDI publication can lag without losing that value.
pub fn observe_portfolio(
    snapshot: &PortfolioSnapshot,
    _fii: &FiiPerformance,
    portfolio: &PortfolioPerformance,
    benchmark: &PortfolioBenchmark,
    events: &[IncomeEvent],
    today: NaiveDate,
) -> Result<Option<PortfolioHistoryObservation>> {
    let (Some(as_of), Some(invested), Some(result)) = (
        benchmark.as_of.as_deref(),
        portfolio.invested.as_deref(),
        portfolio.total_result.as_deref(),
    ) else {
        return Ok(None);
    };
    let observed = date(as_of)?;
    if portfolio.status != "available"
        || observed > today
        || benchmark.components.is_empty()
        || benchmark.components.iter().any(|component| {
            component.as_of.as_deref() != Some(as_of)
                || component.result.is_none()
                || component.invested.is_none()
        })
    {
        return Ok(None);
    }
    let output = PortfolioHistoryObservation {
        month: observed.format("%Y-%m").to_string(),
        as_of: as_of.into(),
        invested: invested.into(),
        portfolio_result: result.into(),
        basis_fingerprint: basis_fingerprint(snapshot, events, observed)?,
    };
    validate_history_observation(&output)?;
    Ok(Some(output))
}

/// Show the wallet's already calculated result even when its latest real
/// component valuations have different dates. No rate, date or balance is
/// inferred; historical observation capture keeps its common-date rule.
pub fn latest_known_valuation(
    fii: &FiiPerformance,
    portfolio: &PortfolioPerformance,
    benchmark: &PortfolioBenchmark,
    today: NaiveDate,
) -> Result<Option<PortfolioLatestValuation>> {
    let (Some(invested), Some(result)) = (
        portfolio.invested.as_deref(),
        portfolio.total_result.as_deref(),
    ) else {
        return Ok(None);
    };
    if portfolio.status != "available" {
        return Ok(None);
    }
    // Require exactly the active components from the accounting calculation.
    // An absent fixed-income position is not an unknown zero position. Cash
    // pockets with known zero balances still retain their reconciliation date.
    let mut expected = BTreeMap::new();
    if fii.status != "empty" {
        expected.insert(
            ("fiis".to_string(), None),
            (
                Some(fii.invested.as_str()),
                fii.total_result.as_deref(),
                fii.as_of.as_deref(),
            ),
        );
    }
    if portfolio.fixed_income.status != "empty" {
        let fixed = &portfolio.fixed_income;
        expected.insert(
            ("fixedIncome".to_string(), None),
            (
                fixed.invested.as_deref(),
                fixed.result.as_deref(),
                fixed.reference_date.as_deref(),
            ),
        );
    }
    for cash in &portfolio.cash.items {
        if expected
            .insert(
                ("cash".to_string(), Some(cash.id.clone())),
                (
                    cash.invested.as_deref(),
                    cash.income.as_deref(),
                    cash.balance_date.as_deref(),
                ),
            )
            .is_some()
        {
            return Ok(None);
        }
    }
    if expected.is_empty() || expected.len() != benchmark.components.len() {
        return Ok(None);
    }
    let invested = parse_nonnegative(invested, "Valor aplicado na carteira")?;
    let result = parse_decimal(result)?;
    let mut component_invested = Decimal::ZERO;
    let mut component_result = Decimal::ZERO;
    let mut dates = BTreeSet::new();
    let mut components = Vec::with_capacity(expected.len());
    for component in &benchmark.components {
        let Some((Some(source_invested), Some(source_result), Some(source_date))) =
            expected.remove(&(component.kind.clone(), component.id.clone()))
        else {
            return Ok(None);
        };
        let (Some(cost), Some(gain), Some(as_of)) = (
            component.invested.as_deref(),
            component.result.as_deref(),
            component.as_of.as_deref(),
        ) else {
            return Ok(None);
        };
        let Some(observed) = date(as_of).ok().filter(|day| *day <= today) else {
            return Ok(None);
        };
        let cost = parse_nonnegative(cost, "Valor aplicado no investimento")?;
        let gain = parse_decimal(gain)?;
        if source_date != as_of
            || parse_nonnegative(source_invested, "Valor aplicado no investimento")? != cost
            || parse_decimal(source_result)? != gain
        {
            return Ok(None);
        }
        component_invested = add(component_invested, cost)?;
        component_result = add(component_result, gain)?;
        dates.insert(observed);
        components.push(PortfolioLatestValuationComponent {
            kind: component.kind.clone(),
            id: component.id.clone(),
            name: component.name.clone(),
            as_of: observed.to_string(),
            result: decimal_string(gain),
        });
    }
    if !expected.is_empty() || component_invested != invested || component_result != result {
        return Ok(None);
    }
    Ok(Some(PortfolioLatestValuation {
        from_date: dates
            .first()
            .expect("nonempty complete components")
            .to_string(),
        to_date: dates
            .last()
            .expect("nonempty complete components")
            .to_string(),
        invested: decimal_string(invested),
        portfolio_result: decimal_string(result),
        components,
    }))
}

#[derive(Default)]
struct MovementComponent {
    flows: BTreeMap<NaiveDate, Flow>,
    invested: Decimal,
    withdrawn: Decimal,
}

#[derive(Default)]
struct MovementBasis {
    flows: BTreeMap<NaiveDate, Flow>,
    components: BTreeMap<String, MovementComponent>,
    invested: Decimal,
    withdrawn: Decimal,
    issues: Vec<String>,
    estimated: bool,
}

fn record_flow(
    output: &mut MovementBasis,
    component: &str,
    day: NaiveDate,
    deposit: Decimal,
    withdrawal: Decimal,
) -> Result<()> {
    add_flow(&mut output.flows, day, deposit, withdrawal)?;
    output.invested = add(output.invested, deposit)?;
    output.withdrawn = add(output.withdrawn, withdrawal)?;
    let part = output.components.entry(component.into()).or_default();
    add_flow(&mut part.flows, day, deposit, withdrawal)?;
    part.invested = add(part.invested, deposit)?;
    part.withdrawn = add(part.withdrawn, withdrawal)?;
    Ok(())
}

fn component_cdi_result(
    basis: &MovementBasis,
    rates: &[DailyCdiRate],
    cutoff: NaiveDate,
    today: NaiveDate,
) -> Result<String> {
    let mut result = Decimal::ZERO;
    for component in basis.components.values() {
        let cdi = calculate_flow_cdi(
            &component.flows,
            rates,
            cutoff,
            today,
            component.invested,
            component.withdrawn,
        )?;
        result = add(result, parse_decimal(&cdi.result)?)?;
    }
    Ok(decimal_string(result))
}

fn movements(snapshot: &PortfolioSnapshot, cutoff: NaiveDate) -> Result<MovementBasis> {
    let mut output = MovementBasis::default();
    for fund in &snapshot.funds {
        for entry in &fund.entries {
            let Some(day) = purchase_day(entry, cutoff)? else {
                continue;
            };
            let quantity = parse_nonnegative(&entry.cotas_added, "Cotas compradas")?;
            let price = parse_nonnegative(&entry.price_per_cota, "Preço de compra")?;
            if price.is_zero() {
                output.issues.push(format!(
                    "{}: informe o preço de compra para comparar com o CDI.",
                    fund.ticker
                ));
                continue;
            }
            let cost = mul(quantity, price)?;
            record_flow(&mut output, "fiis", day, cost, Decimal::ZERO)?;
            output.estimated |= entry.purchase_date.is_none();
        }
    }
    if let Some(position) = &snapshot.broker_position {
        if let Some(observation) = &position.fixed_income.latest_observation {
            let contribution_total = observation.contributions.iter().try_fold(
                Decimal::ZERO,
                |total, contribution| {
                    add(
                        total,
                        parse_nonnegative(&contribution.invested, "Aporte no pós-fixado")?,
                    )
                },
            )?;
            if contribution_total
                != parse_nonnegative(&observation.invested, "Valor aplicado no pós-fixado")?
            {
                output.issues.push("Os aportes com data do pós-fixado não conferem com o valor aplicado informado.".into());
            }
            for contribution in &observation.contributions {
                let day = date(&contribution.application_date)?;
                if day > cutoff {
                    continue;
                }
                let amount = parse_nonnegative(&contribution.invested, "Aporte no pós-fixado")?;
                if !amount.is_zero() {
                    record_flow(&mut output, "fixedIncome", day, amount, Decimal::ZERO)?;
                }
            }
            if observation.contributions.is_empty()
                && !parse_nonnegative(&observation.invested, "Aporte no pós-fixado")?.is_zero()
            {
                output.issues.push(
                    "Informe as datas dos aportes do pós-fixado para comparar com o CDI.".into(),
                );
            }
        } else if !parse_nonnegative(&position.fixed_income.balance, "Saldo do pós-fixado")?
            .is_zero()
        {
            output
                .issues
                .push("Faltam os aportes com data do pós-fixado para comparar com o CDI.".into());
        }
    }
    for pocket in &snapshot.cash_pockets {
        if let Some(flows) = &pocket.cash_flows {
            let mut deposits = Decimal::ZERO;
            let mut withdrawals = Decimal::ZERO;
            for flow in flows {
                let amount = parse_nonnegative(&flow.amount, "Movimentação da caixinha")?;
                match flow.kind.as_str() {
                    "deposit" => deposits = add(deposits, amount)?,
                    "withdrawal" => withdrawals = add(withdrawals, amount)?,
                    _ => return Err(invalid("Tipo inválido na movimentação da caixinha.")),
                }
            }
            if pocket
                .total_deposits
                .as_deref()
                .map(parse_decimal)
                .transpose()?
                != Some(deposits)
                || pocket
                    .total_withdrawals
                    .as_deref()
                    .map(parse_decimal)
                    .transpose()?
                    != Some(withdrawals)
            {
                output.issues.push(format!("{}: os movimentos com data não conferem com os totais depositados e retirados.", pocket.name));
            }
            for movement in flows {
                let day = date(&movement.date)?;
                if day > cutoff {
                    continue;
                }
                let amount = parse_nonnegative(&movement.amount, "Movimentação da caixinha")?;
                if amount.is_zero() {
                    return Err(invalid(
                        "Uma movimentação da caixinha precisa de valor positivo.",
                    ));
                }
                match movement.kind.as_str() {
                    "deposit" => {
                        record_flow(
                            &mut output,
                            &format!("cash:{}", pocket.id),
                            day,
                            amount,
                            Decimal::ZERO,
                        )?;
                    }
                    "withdrawal" => {
                        record_flow(
                            &mut output,
                            &format!("cash:{}", pocket.id),
                            day,
                            Decimal::ZERO,
                            amount,
                        )?;
                    }
                    _ => return Err(invalid("Tipo inválido na movimentação da caixinha.")),
                }
            }
        } else if pocket.total_deposits.as_deref() != Some("0")
            || pocket.total_withdrawals.as_deref() != Some("0")
        {
            output.issues.push(format!(
                "{}: informe as datas dos depósitos e retiradas para comparar com o CDI.",
                pocket.name
            ));
        }
    }
    Ok(output)
}

/// Reuse precisely the same matched-flow CDI model for reconstructed month ends.
pub(crate) fn reconstructed_month_cdi(
    snapshot: &PortfolioSnapshot,
    rates: &[DailyCdiRate],
    cutoff: NaiveDate,
    today: NaiveDate,
) -> Result<(Option<String>, Vec<String>)> {
    let basis = movements(snapshot, cutoff)?;
    let mut issues = basis.issues.clone();
    if basis.estimated {
        issues.push("CDI: compras antigas sem dia informado usam o fim do mês; essa convenção não altera as cotas no fechamento mensal.".into());
    }
    let calculate = || -> Result<String> {
        // A completed calendar month can close after the last published CDI
        // observation (for example Easter Friday followed by a weekend). Keep
        // every flow at its original date; only published observations earn
        // interest. Live/partial balances retain their strict coverage rule.
        let expected = match cutoff.weekday() {
            chrono::Weekday::Sat => cutoff.pred_opt(),
            chrono::Weekday::Sun => cutoff.pred_opt().and_then(|day| day.pred_opt()),
            _ => Some(cutoff),
        }
        .ok_or_else(|| invalid("Data final fora do intervalo suportado."))?;
        let mut published = BTreeSet::new();
        for observation in rates {
            let day = date(&observation.date)?;
            parse_nonnegative(&observation.value, "Taxa CDI diária")?;
            if day > today || !published.insert(day) {
                return Err(invalid("O histórico CDI contém data futura ou duplicada."));
            }
        }
        let last = published.range(..=cutoff).next_back().copied();
        let following = published
            .range((
                std::ops::Bound::Excluded(cutoff),
                std::ops::Bound::Unbounded,
            ))
            .next()
            .copied();
        let closed_month = cutoff < today && month_end(cutoff)? == cutoff;
        let Some(last) = last.filter(|last| {
            closed_month
                && *last < expected
                && last.year() == cutoff.year()
                && last.month() == cutoff.month()
                && (cutoff - *last).num_days() <= 7
                // A recently closed month's terminal rate may simply not be
                // published yet. Require the series to resume after the month
                // before accepting this bounded historical publication gap.
                && following.is_some_and(|next| (next - *last).num_days() <= 7)
        }) else {
            return component_cdi_result(&basis, rates, cutoff, today);
        };
        let mut total = Decimal::ZERO;
        for component in basis.components.values() {
            let mut settled = BTreeMap::new();
            let mut invested = Decimal::ZERO;
            let mut withdrawn = Decimal::ZERO;
            for (day, flow) in component.flows.range(..=last) {
                settled.insert(*day, *flow);
                invested = add(invested, flow.deposits)?;
                withdrawn = add(withdrawn, flow.withdrawals)?;
            }
            let ending = calculate_flow_cdi(&settled, rates, last, today, invested, withdrawn)?;
            let mut balance = parse_decimal(&ending.ending_value)?;
            for (_, flow) in component.flows.range((
                std::ops::Bound::Excluded(last),
                std::ops::Bound::Included(cutoff),
            )) {
                let available = add(balance, flow.deposits)?;
                if flow.withdrawals > available {
                    return Err(invalid("Uma retirada supera o saldo do cenário CDI naquela data; não foi presumido empréstimo."));
                }
                balance = available
                    .checked_sub(flow.withdrawals)
                    .ok_or(CoreError::ArithmeticOverflow)?;
            }
            let result = add(balance, component.withdrawn)?
                .checked_sub(component.invested)
                .ok_or(CoreError::ArithmeticOverflow)?;
            total = add(total, result)?;
        }
        Ok(decimal_string(total))
    };
    let result = if basis.issues.is_empty() {
        match calculate() {
            Ok(result) => Some(result),
            Err(error) => {
                issues.push(error.to_string());
                None
            }
        }
    } else {
        None
    };
    if result.is_some() && cutoff < today && month_end(cutoff)? == cutoff {
        if let Some(last) = rates
            .iter()
            .filter_map(|rate| date(&rate.date).ok())
            .filter(|day| *day <= cutoff)
            .max()
        {
            if last < cutoff {
                issues.push(format!("CDI: última taxa publicada em {last}; o fechamento mantém as datas originais dos movimentos, sem criar juros nos dias sem taxa publicada."));
            }
        }
    }
    Ok((result, issues))
}

fn earliest_flow(snapshot: &PortfolioSnapshot, today: NaiveDate) -> Result<Option<NaiveDate>> {
    Ok(movements(snapshot, today)?.flows.keys().next().copied())
}

/// Read-only monthly evolution. Observations can be exported/restored, but a
/// changed historical ledger invalidates its old derived value automatically.
pub fn calculate_portfolio_evolution(
    snapshot: &PortfolioSnapshot,
    rates: &[DailyCdiRate],
    events: &[IncomeEvent],
    observations: &[PortfolioHistoryObservation],
    current: Option<&PortfolioHistoryObservation>,
    today: NaiveDate,
) -> Result<PortfolioEvolution> {
    let assumptions = vec![
        "As duas linhas mostram o resultado acumulado em reais, com os mesmos aportes e retiradas; não mostram apenas o saldo ou uma taxa anualizada.".into(),
        "A carteira inclui FIIs, pós-fixado e caixinhas. Os proventos conhecidos já entram no resultado uma única vez e não são reinvestidos no cenário CDI.".into(),
        "Os registros reais usam os saldos observados e os rendimentos disponíveis. Proventos públicos calculados não comprovam crédito na corretora; períodos sem cobertura de rendimentos podem alterar o resultado quando forem completados.".into(),
        "Cada mês usa o último saldo real observado de toda a carteira na mesma data. Meses sem saldo observado ficam sem ponto; nenhuma rentabilidade foi inventada.".into(),
        "O CDI usa a série diária histórica SGS 12, a partir do dia seguinte a cada aporte, sem impostos ou taxas. Compras antigas sem dia usam o fim do mês e aparecem como estimadas.".into(),
        "Se uma compra ou rendimento antigo for corrigido, o resultado salvo daquele período precisa ser observado novamente. Novas compras futuras preservam os pontos anteriores.".into(),
    ];
    let mut by_month: BTreeMap<String, PortfolioHistoryObservation> = BTreeMap::new();
    for observation in observations {
        validate_history_observation(observation)?;
        if date(&observation.as_of)? > today {
            return Err(invalid("O histórico da evolução contém uma data futura."));
        }
        if by_month
            .get(&observation.month)
            .is_none_or(|previous| previous.as_of < observation.as_of)
        {
            by_month.insert(observation.month.clone(), observation.clone());
        }
    }
    if let Some(observation) = current {
        validate_history_observation(observation)?;
        if date(&observation.as_of)? > today {
            return Err(invalid("A observação atual possui data futura."));
        }
        if by_month
            .get(&observation.month)
            .is_none_or(|previous| previous.as_of <= observation.as_of)
        {
            by_month.insert(observation.month.clone(), observation.clone());
        }
    }
    let newest_saved = by_month
        .values()
        .map(|observation| date(&observation.as_of))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .max();
    let end_date = current
        .map(|observation| date(&observation.as_of))
        .transpose()?
        .map(|current| newest_saved.map_or(current, |saved| current.max(saved)))
        .unwrap_or(today);
    let first = earliest_flow(snapshot, end_date)?
        .into_iter()
        .chain(
            by_month
                .values()
                .map(|observation| date(&observation.as_of))
                .collect::<Result<Vec<_>>>()?,
        )
        .min()
        .or_else(|| {
            (!snapshot.funds.is_empty()
                || !snapshot.cash_pockets.is_empty()
                || snapshot
                    .broker_position
                    .as_ref()
                    .is_some_and(|position| position.fixed_income.balance != "0"))
            .then_some(end_date)
        });
    let Some(first) = first else {
        return Ok(PortfolioEvolution {
            status: "empty".into(),
            start_date: None,
            as_of: None,
            points: vec![],
            issues: vec![],
            assumptions,
            latest_known: None,
        });
    };
    let count =
        (end_date.year() - first.year()) * 12 + end_date.month() as i32 - first.month() as i32 + 1;
    if !(1..=1200).contains(&count) {
        return Err(invalid("Período da evolução fora do intervalo suportado."));
    }
    let mut points = Vec::with_capacity(count as usize);
    let mut summary = BTreeSet::new();
    for offset in 0..count {
        let index = first.year() * 12 + first.month0() as i32 + offset;
        let month = format!("{:04}-{:02}", index / 12, index % 12 + 1);
        let end = month_end(month_start(&month)?)?;
        let observation = by_month.get(&month);
        let mut cutoff = observation
            .map(|observation| date(&observation.as_of))
            .transpose()?
            .unwrap_or(end.min(end_date));
        // Without a real valuation, an end-of-month holiday may use the last
        // published CDI date. The actual date remains visible on the point.
        if observation.is_none() && end < end_date {
            if let Some(last) = rates
                .iter()
                .filter_map(|rate| date(&rate.date).ok())
                .filter(|day| *day <= end && *day >= end - chrono::Duration::days(7))
                .max()
            {
                cutoff = last;
            }
        }
        let basis = movements(snapshot, cutoff)?;
        let mut issues = basis.issues.clone();
        if basis.estimated {
            issues.push("CDI: compras antigas sem dia informado usam o fim do mês; no mês parcial, usam a data final exibida.".into());
        }
        let mut portfolio_result = None;
        let mut invested = basis
            .issues
            .is_empty()
            .then(|| decimal_string(basis.invested));
        if let Some(observation) = observation {
            if observation.basis_fingerprint == basis_fingerprint(snapshot, events, cutoff)? {
                if basis.issues.is_empty()
                    && parse_decimal(&observation.invested)? != basis.invested
                {
                    issues.push("O total aplicado salvo não confere com os movimentos deste período. O ponto ficou indisponível para evitar uma comparação incorreta.".into());
                } else {
                    portfolio_result = Some(observation.portfolio_result.clone());
                    invested = Some(observation.invested.clone());
                }
            } else {
                issues.push("Os lançamentos deste período foram alterados. O ponto salvo não corresponde mais à base atual e ficou indisponível.".into());
            }
        } else {
            issues.push("Sem registro real da carteira neste mês. Valores antigos não são reconstruídos a partir do saldo atual; os próximos registros completos serão guardados automaticamente.".into());
        }
        let cdi_result = if basis.issues.is_empty() {
            match component_cdi_result(&basis, rates, cutoff, today) {
                Ok(result) => Some(result),
                Err(error) => {
                    issues.push(error.to_string());
                    None
                }
            }
        } else {
            None
        };
        let difference = match (&portfolio_result, &cdi_result) {
            (Some(actual), Some(cdi))
                if invested.as_deref().map(parse_decimal).transpose()? == Some(basis.invested) =>
            {
                Some(decimal_string(
                    parse_decimal(actual)?
                        .checked_sub(parse_decimal(cdi)?)
                        .ok_or(CoreError::ArithmeticOverflow)?,
                ))
            }
            (Some(_), Some(_)) => {
                issues.push("O total aplicado salvo não confere com os movimentos deste período; a diferença para o CDI está pendente.".into());
                None
            }
            _ => None,
        };
        let complete = portfolio_result.is_some() && cdi_result.is_some() && difference.is_some();
        for issue in &issues {
            summary.insert(issue.clone());
        }
        points.push(PortfolioEvolutionPoint {
            month,
            as_of: cutoff.to_string(),
            portfolio_result,
            cdi_result,
            invested,
            difference,
            status: if complete { "available" } else { "incomplete" }.into(),
            issues,
            estimated: false,
            is_partial_month: cutoff < end,
            provenance: None,
        });
    }
    Ok(PortfolioEvolution {
        status: if points.iter().all(|point| point.status == "available") {
            "available"
        } else {
            "partial"
        }
        .into(),
        start_date: Some(first.to_string()),
        as_of: points.last().map(|point| point.as_of.clone()),
        points,
        issues: summary.into_iter().collect(),
        assumptions,
        latest_known: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        performance::{calculate_fii_performance_with_income, PerformanceQuote},
        portfolio_benchmark::calculate_portfolio_benchmark,
        portfolio_performance::calculate_portfolio_performance,
    };
    use serde_json::json;

    fn day(value: &str) -> NaiveDate {
        date(value).unwrap()
    }

    fn evolution(
        snapshot: &PortfolioSnapshot,
        rates: &[DailyCdiRate],
        events: &[IncomeEvent],
        observations: &[PortfolioHistoryObservation],
        current: Option<&PortfolioHistoryObservation>,
        today: NaiveDate,
    ) -> Result<PortfolioEvolution> {
        let published = rates
            .iter()
            .filter(|rate| day(&rate.date) <= today)
            .cloned()
            .collect::<Vec<_>>();
        calculate_portfolio_evolution(snapshot, &published, events, observations, current, today)
    }

    fn snapshot() -> PortfolioSnapshot {
        serde_json::from_value(json!({"funds":[],"totals":{"invested":"0","cotas":"0","dividendsReceived":"0","estimatedMonthlyIncome":"0"},"recordCount":0,
            "cashPockets":[],"cashBalance":"0"})).unwrap()
    }

    fn with_fii() -> PortfolioSnapshot {
        serde_json::from_value(json!({
            "funds":[{"id":"fund-a","ticker":"TEST11","name":"Fundo de teste","sector":"",
                "entries":[{"id":"buy-a","month":"2026-01","purchaseDate":"2026-01-02",
                    "cotasAdded":"10","pricePerCota":"10","dividendPerCota":"0","dividendTotal":null,"notes":""}],
                "totalCotas":"10","totalInvested":"100","averagePrice":"10","dividendsReceived":"0","estimatedMonthlyIncome":"0"}],
            "totals":{"invested":"0","cotas":"0","dividendsReceived":"0","estimatedMonthlyIncome":"0"},"recordCount":1,"cashPockets":[],"cashBalance":"0"
        })).unwrap()
    }

    fn rates() -> Vec<DailyCdiRate> {
        let mut result = vec![];
        let mut current = day("2025-12-29");
        while current <= day("2026-03-31") {
            if !matches!(
                current.weekday(),
                chrono::Weekday::Sat | chrono::Weekday::Sun
            ) {
                result.push(DailyCdiRate {
                    date: current.to_string(),
                    value: "0.1".into(),
                });
            }
            current = current.succ_opt().unwrap();
        }
        result
    }

    fn observation(
        snapshot: &PortfolioSnapshot,
        as_of: &str,
        result: &str,
    ) -> PortfolioHistoryObservation {
        let cutoff = day(as_of);
        PortfolioHistoryObservation {
            month: cutoff.format("%Y-%m").to_string(),
            as_of: as_of.into(),
            invested: decimal_string(movements(snapshot, cutoff).unwrap().invested),
            portfolio_result: result.into(),
            basis_fingerprint: basis_fingerprint(snapshot, &[], cutoff).unwrap(),
        }
    }

    fn cash() -> crate::CashPocket {
        serde_json::from_value(json!({"id":"cash-a","name":"Caixinha de teste","institution":"Teste",
            "balance":"80","totalDeposits":"100","totalWithdrawals":"30","balanceDate":"2026-01-09",
            "cashFlows":[{"date":"2026-01-02","kind":"deposit","amount":"100"},{"date":"2026-01-08","kind":"withdrawal","amount":"30"}],
            "cdiPercent":"120","annualCdiRate":"9999","rateDate":null,"notes":"",
            "createdAt":"2020-01-01T00:00:00Z","updatedAt":"2026-01-09T00:00:00Z"})).unwrap()
    }

    fn fixed() -> crate::BrokerPosition {
        serde_json::from_value(json!({"referenceDate":"2026-01-09","institution":"Teste","importedAt":"2026-01-09T12:00:00Z",
            "sourcePath":"fixture.json","sourceHash":"a".repeat(64),"warnings":[],
            "overview":{"totalWithCash":"999","invested":"999","cash":"999","listedFunds":"999","fixedIncome":"999","pendingIncome":"999"},
            "listedFunds":{"marketValue":"0","costBasis":"0","unrealizedResult":"0","dailyResult":"0","holdings":[]},
            "fixedIncome":{"name":"Pós-fixado de teste","nameTruncated":false,"balance":"110","netBalance":"109","reportedEarnings":"10",
                "return12Months":"9999","redemptionConversion":"D+0","redemptionSettlement":"D+0",
                "latestObservation":{"referenceDate":"2026-01-09","name":"Pós-fixado de teste","category":"Renda fixa","riskLevel":"1",
                    "balance":"110","invested":"100","totalEarnings":"10","earnings12Months":"9999","return12Months":"9999",
                    "returnReferenceDate":"2026-01-08","contributions":[{"applicationDate":"2026-01-02","invested":"100","currentValue":"110","earnings":"10","returnPercent":"10"}],
                    "sources":[],"importedAt":"2026-01-09T12:00:00Z","sourcePath":"fixture.json","sourceHash":"b".repeat(64)}}})).unwrap()
    }

    fn event(amount: &str) -> IncomeEvent {
        IncomeEvent {
            symbol: "TEST11".into(),
            amount_per_share: amount.into(),
            record_date: "2026-01-05".into(),
            payment_date: "2026-01-09".into(),
            source_url: "https://example.test/event".into(),
            source: "Test".into(),
            id: "income-a".into(),
        }
    }

    fn capture(
        snapshot: &PortfolioSnapshot,
        as_of: &str,
        events: &[IncomeEvent],
        rates: &[DailyCdiRate],
    ) -> Option<PortfolioHistoryObservation> {
        let quotes = snapshot
            .funds
            .iter()
            .map(|fund| PerformanceQuote {
                ticker: fund.ticker.clone(),
                price: "11".into(),
                as_of: as_of.into(),
            })
            .collect::<Vec<_>>();
        let fii =
            calculate_fii_performance_with_income(snapshot, &quotes, rates, events, day(as_of))
                .unwrap();
        let portfolio = calculate_portfolio_performance(snapshot, &fii).unwrap();
        let benchmark =
            calculate_portfolio_benchmark(snapshot, &fii, &portfolio, rates, day(as_of)).unwrap();
        observe_portfolio(snapshot, &fii, &portfolio, &benchmark, events, day(as_of)).unwrap()
    }

    fn latest_parts(
        snapshot: &PortfolioSnapshot,
        as_of: &str,
        price: &str,
    ) -> (FiiPerformance, PortfolioPerformance, PortfolioBenchmark) {
        let quotes = snapshot
            .funds
            .iter()
            .map(|fund| PerformanceQuote {
                ticker: fund.ticker.clone(),
                price: price.into(),
                as_of: as_of.into(),
            })
            .collect::<Vec<_>>();
        let fii =
            calculate_fii_performance_with_income(snapshot, &quotes, &[], &[], day(as_of)).unwrap();
        let portfolio = calculate_portfolio_performance(snapshot, &fii).unwrap();
        let benchmark =
            calculate_portfolio_benchmark(snapshot, &fii, &portfolio, &[], day(as_of)).unwrap();
        (fii, portfolio, benchmark)
    }

    fn with_fii_and_fixed() -> PortfolioSnapshot {
        let mut original = with_fii();
        let mut position = fixed();
        // The real FII calculator reconciles holdings against the dated broker
        // position, so this mixed-asset fixture must describe both positions.
        position.listed_funds.holdings.push(crate::BrokerHolding {
            ticker: "TEST11".into(),
            shares: "10".into(),
            average_price: "10".into(),
            price: "11".into(),
            market_value: "110".into(),
            unrealized_result: "10".into(),
            cost_basis: "100".into(),
        });
        original.broker_position = Some(position);
        original
    }

    #[test]
    fn latest_values_keep_distinct_real_dates_without_fabricating_a_monthly_point_or_cdi() {
        let original = with_fii_and_fixed();
        let before = serde_json::to_value(&original).unwrap();
        let (fii, portfolio, benchmark) = latest_parts(&original, "2026-01-12", "11");
        assert_eq!(portfolio.total_result.as_deref(), Some("20"));
        assert!(benchmark.as_of.is_none());
        assert!(benchmark.cdi.is_none());
        assert_eq!(benchmark.status, "incomplete");
        let latest = latest_known_valuation(&fii, &portfolio, &benchmark, day("2026-01-12"))
            .unwrap()
            .unwrap();
        assert_eq!(latest.from_date, "2026-01-09");
        assert_eq!(latest.to_date, "2026-01-12");
        assert_eq!(latest.invested, "200");
        assert_eq!(latest.portfolio_result, "20");
        assert_eq!(latest.components.len(), 2);
        assert_eq!(latest.components[0].as_of, "2026-01-12");
        assert_eq!(latest.components[1].as_of, "2026-01-09");
        let current = observe_portfolio(
            &original,
            &fii,
            &portfolio,
            &benchmark,
            &[],
            day("2026-01-12"),
        )
        .unwrap();
        assert!(current.is_none());
        let history = evolution(
            &original,
            &[],
            &[],
            &[],
            current.as_ref(),
            day("2026-01-12"),
        )
        .unwrap();
        assert!(history.latest_known.is_none());
        assert!(history
            .points
            .iter()
            .all(|point| point.portfolio_result.is_none() && point.difference.is_none()));
        let serialized = serde_json::to_value(&latest).unwrap();
        assert!(serialized.get("cdiResult").is_none());
        assert!(serialized.get("difference").is_none());
        assert_eq!(serde_json::to_value(&original).unwrap(), before);
    }

    #[test]
    fn latest_valuation_accepts_losses_and_same_date_values_without_daily_cdi() {
        let original = with_fii();
        let (fii, portfolio, benchmark) = latest_parts(&original, "2026-01-09", "9");
        let latest = latest_known_valuation(&fii, &portfolio, &benchmark, day("2026-01-09"))
            .unwrap()
            .unwrap();
        assert_eq!(latest.from_date, "2026-01-09");
        assert_eq!(latest.to_date, "2026-01-09");
        assert_eq!(latest.portfolio_result, "-10");
        assert!(benchmark.cdi.is_none());
    }

    #[test]
    fn latest_valuation_preserves_a_reconciled_zero_and_ignores_absent_positions() {
        let mut original = snapshot();
        let mut pocket = cash();
        pocket.balance = "0".into();
        pocket.total_withdrawals = Some("100".into());
        pocket.cash_flows.as_mut().unwrap()[1].amount = "100".into();
        original.cash_pockets.push(pocket);
        let (fii, portfolio, benchmark) = latest_parts(&original, "2026-01-09", "1");
        let latest = latest_known_valuation(&fii, &portfolio, &benchmark, day("2026-01-09"))
            .unwrap()
            .unwrap();
        assert_eq!(latest.portfolio_result, "0");
        assert_eq!(latest.components.len(), 1);
        assert_eq!(latest.components[0].kind, "cash");
        let (fii, portfolio, benchmark) = latest_parts(&snapshot(), "2026-01-09", "1");
        assert!(
            latest_known_valuation(&fii, &portfolio, &benchmark, day("2026-01-09"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn latest_valuation_requires_all_component_dates_and_complete_accounting() {
        let original = with_fii_and_fixed();
        let (fii, portfolio, benchmark) = latest_parts(&original, "2026-01-12", "11");
        assert_eq!(portfolio.status, "available");
        for missing in ["date", "result", "cost", "component"] {
            let mut broken = benchmark.clone();
            match missing {
                "date" => broken.components[1].as_of = None,
                "result" => broken.components[1].result = None,
                "cost" => broken.components[1].invested = None,
                _ => {
                    broken.components.pop();
                }
            }
            assert!(
                latest_known_valuation(&fii, &portfolio, &broken, day("2026-01-12"))
                    .unwrap()
                    .is_none(),
                "{missing}"
            );
        }
        let mut incomplete = portfolio.clone();
        incomplete.total_result = None;
        assert!(
            latest_known_valuation(&fii, &incomplete, &benchmark, day("2026-01-12"))
                .unwrap()
                .is_none()
        );
        incomplete = portfolio.clone();
        incomplete.status = "incomplete".into();
        assert!(
            latest_known_valuation(&fii, &incomplete, &benchmark, day("2026-01-12"))
                .unwrap()
                .is_none()
        );
        let mut fixed_without_date = portfolio.clone();
        fixed_without_date.fixed_income.reference_date = None;
        assert!(
            latest_known_valuation(&fii, &fixed_without_date, &benchmark, day("2026-01-12"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn latest_valuation_rejects_future_dates_invalid_dates_and_mismatched_totals() {
        let original = with_fii();
        let (fii, portfolio, benchmark) = latest_parts(&original, "2026-01-12", "11");
        assert!(
            latest_known_valuation(&fii, &portfolio, &benchmark, day("2026-01-09"))
                .unwrap()
                .is_none()
        );
        let mut invalid = benchmark.clone();
        invalid.components[0].as_of = Some("2026-1-12".into());
        assert!(
            latest_known_valuation(&fii, &portfolio, &invalid, day("2026-01-12"))
                .unwrap()
                .is_none()
        );
        for field in ["result", "cost"] {
            let mut mismatch = portfolio.clone();
            if field == "result" {
                mismatch.total_result = Some("999".into());
            } else {
                mismatch.invested = Some("999".into());
            }
            assert!(
                latest_known_valuation(&fii, &mismatch, &benchmark, day("2026-01-12"))
                    .unwrap()
                    .is_none()
            );
        }
        let mut mismatch = benchmark.clone();
        mismatch.components[0].result = Some("999".into());
        assert!(
            latest_known_valuation(&fii, &portfolio, &mismatch, day("2026-01-12"))
                .unwrap()
                .is_none()
        );
        mismatch = benchmark.clone();
        mismatch.components.push(mismatch.components[0].clone());
        assert!(
            latest_known_valuation(&fii, &portfolio, &mismatch, day("2026-01-12"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn empty_wallet_has_no_fabricated_points() {
        let report = evolution(&snapshot(), &[], &[], &[], None, day("2026-03-15")).unwrap();
        assert_eq!(report.status, "empty");
        assert!(report.points.is_empty());
    }

    #[test]
    fn observation_validation_accepts_losses_and_rejects_noncanonical_dates() {
        let valid = observation(&with_fii(), "2026-01-30", "-7.25");
        validate_history_observation(&valid).unwrap();
        for (month, as_of) in [
            ("2026-1", "2026-01-30"),
            ("2026-02", "2026-01-30"),
            ("2026-01", "2026-1-30"),
            ("0000-01", "0000-01-30"),
        ] {
            let mut bad = valid.clone();
            bad.month = month.into();
            bad.as_of = as_of.into();
            assert!(validate_history_observation(&bad).is_err());
        }
        let mut bad = valid;
        bad.basis_fingerprint = "A".repeat(64);
        assert!(validate_history_observation(&bad).is_err());
    }

    #[test]
    fn cdi_line_is_independent_of_absent_real_monthly_values() {
        let report = evolution(&with_fii(), &rates(), &[], &[], None, day("2026-03-20")).unwrap();
        assert_eq!(report.points.len(), 3);
        assert!(report.points.iter().all(|p| p.portfolio_result.is_none()));
        assert!(report.points.iter().all(|p| p.cdi_result.is_some()));
        assert_eq!(report.status, "partial");
    }

    #[test]
    fn observed_fii_result_keeps_exact_api_income_without_double_counting() {
        let original = with_fii();
        let events = vec![event("0.5")];
        let captured = capture(&original, "2026-01-09", &events, &rates()).unwrap();
        assert_eq!(captured.portfolio_result, "15");
        let report = evolution(
            &original,
            &rates(),
            &events,
            &[],
            Some(&captured),
            day("2026-01-10"),
        )
        .unwrap();
        assert_eq!(report.points[0].portfolio_result.as_deref(), Some("15"));
        assert_eq!(report.points[0].as_of, "2026-01-09");
    }

    #[test]
    fn pending_cdi_does_not_discard_a_real_valuation() {
        let original = with_fii();
        let captured = capture(&original, "2026-01-09", &[], &[]).unwrap();
        let report =
            evolution(&original, &[], &[], &[], Some(&captured), day("2026-01-09")).unwrap();
        assert_eq!(report.points[0].portfolio_result.as_deref(), Some("10"));
        assert!(report.points[0].cdi_result.is_none());
    }

    #[test]
    fn retrospective_purchase_correction_invalidates_a_derived_real_point() {
        let mut original = with_fii();
        let captured = observation(&original, "2026-01-30", "10");
        original.funds[0].entries[0].price_per_cota = "9".into();
        let report = evolution(
            &original,
            &rates(),
            &[],
            &[captured],
            None,
            day("2026-02-02"),
        )
        .unwrap();
        assert!(report.points[0].portfolio_result.is_none());
        assert!(report.points[0].cdi_result.is_some());
        assert!(report.points[0]
            .issues
            .iter()
            .any(|issue| issue.contains("alterados")));
    }

    #[test]
    fn future_purchases_do_not_erase_previous_points() {
        let mut original = with_fii();
        let captured = observation(&original, "2026-01-30", "10");
        let mut future = original.funds[0].entries[0].clone();
        future.month = "2026-02".into();
        future.purchase_date = Some("2026-02-05".into());
        future.cotas_added = "5".into();
        original.funds[0].entries.push(future);
        let report = evolution(
            &original,
            &rates(),
            &[],
            &[captured],
            None,
            day("2026-02-09"),
        )
        .unwrap();
        assert_eq!(report.points[0].portfolio_result.as_deref(), Some("10"));
        assert_eq!(report.points[1].invested.as_deref(), Some("150"));
    }

    #[test]
    fn restored_identifiers_and_notes_do_not_change_economic_basis() {
        let mut original = with_fii();
        original.cash_pockets.push(cash());
        let captured = observation(&original, "2026-01-09", "20");
        original.funds[0].id = "restored-fund".into();
        original.funds[0].name = "Nome atualizado".into();
        original.funds[0].entries[0].id = "restored-entry".into();
        original.funds[0].entries[0].notes = "Nova nota".into();
        original.cash_pockets[0].id = "restored-pocket".into();
        let report = evolution(
            &original,
            &rates(),
            &[],
            &[],
            Some(&captured),
            day("2026-01-09"),
        )
        .unwrap();
        assert_eq!(report.points[0].portfolio_result.as_deref(), Some("20"));
    }

    #[test]
    fn a_new_or_corrected_api_payment_invalidates_old_known_income() {
        let original = with_fii();
        let captured = observation(&original, "2026-01-30", "10");
        let report = evolution(
            &original,
            &rates(),
            &[event("0.5")],
            &[captured],
            None,
            day("2026-02-02"),
        )
        .unwrap();
        assert!(report.points[0].portfolio_result.is_none());
    }

    #[test]
    fn explicit_manual_zero_masks_api_changes_in_history_too() {
        let mut original = with_fii();
        original.funds[0].entries[0].dividend_total = Some("0".into());
        let captured = observation(&original, "2026-01-30", "10");
        let report = evolution(
            &original,
            &rates(),
            &[event("999")],
            &[captured],
            None,
            day("2026-02-02"),
        )
        .unwrap();
        assert_eq!(report.points[0].portfolio_result.as_deref(), Some("10"));
    }

    #[test]
    fn future_api_payments_do_not_change_previous_points() {
        let original = with_fii();
        let captured = observation(&original, "2026-01-30", "10");
        let mut future = event("999");
        future.payment_date = "2026-02-02".into();
        let report = evolution(
            &original,
            &rates(),
            &[future],
            &[captured],
            None,
            day("2026-02-02"),
        )
        .unwrap();
        assert_eq!(report.points[0].portfolio_result.as_deref(), Some("10"));
    }

    #[test]
    fn latest_observation_in_month_wins_and_gaps_are_never_interpolated() {
        let original = with_fii();
        let first = observation(&original, "2026-01-09", "-5");
        let last = observation(&original, "2026-01-30", "10");
        let march = observation(&original, "2026-03-20", "30");
        let report = evolution(
            &original,
            &rates(),
            &[],
            &[first, last],
            Some(&march),
            day("2026-03-20"),
        )
        .unwrap();
        assert_eq!(report.points[0].portfolio_result.as_deref(), Some("10"));
        assert!(report.points[1].portfolio_result.is_none());
        assert_eq!(report.points[2].portfolio_result.as_deref(), Some("30"));
    }

    #[test]
    fn exact_dated_cash_withdrawals_are_used_in_both_results() {
        let mut original = snapshot();
        original.cash_pockets.push(cash());
        let captured = capture(&original, "2026-01-09", &[], &rates()).unwrap();
        assert_eq!(captured.invested, "100");
        assert_eq!(captured.portfolio_result, "10");
        let report = evolution(
            &original,
            &rates(),
            &[],
            &[],
            Some(&captured),
            day("2026-01-09"),
        )
        .unwrap();
        let cdi = parse_decimal(report.points[0].cdi_result.as_deref().unwrap()).unwrap();
        assert!(cdi > Decimal::ZERO && cdi < Decimal::ONE);
        assert!(report.points[0].difference.is_some());
    }

    #[test]
    fn cash_without_dated_flows_keeps_actual_value_but_not_a_fake_cdi() {
        let mut original = snapshot();
        let mut pocket = cash();
        pocket.cash_flows = None;
        original.cash_pockets.push(pocket);
        let captured = capture(&original, "2026-01-09", &[], &rates()).unwrap();
        let report = evolution(
            &original,
            &rates(),
            &[],
            &[],
            Some(&captured),
            day("2026-01-09"),
        )
        .unwrap();
        assert_eq!(report.points[0].portfolio_result.as_deref(), Some("10"));
        assert!(report.points[0].cdi_result.is_none());
        assert!(report.points[0]
            .issues
            .iter()
            .any(|issue| issue.contains("datas dos depósitos")));
    }

    #[test]
    fn fixed_income_uses_observed_gain_not_the_annual_indicator() {
        let mut original = snapshot();
        original.broker_position = Some(fixed());
        let captured = capture(&original, "2026-01-09", &[], &rates()).unwrap();
        assert_eq!(captured.portfolio_result, "10");
        let report = evolution(
            &original,
            &rates(),
            &[],
            &[],
            Some(&captured),
            day("2026-01-09"),
        )
        .unwrap();
        assert_eq!(report.points[0].portfolio_result.as_deref(), Some("10"));
        assert!(report.points[0].cdi_result.is_some());
    }

    #[test]
    fn all_three_classes_are_captured_once_on_the_same_date() {
        let mut original = with_fii();
        original.cash_pockets.push(cash());
        original.broker_position = Some(fixed());
        // The synthetic broker anchor has no listed positions and would reject
        // the FII ledger. Use an earlier anchor so only observed dates matter.
        original.broker_position.as_mut().unwrap().reference_date = "2025-12-31".into();
        original.broker_position.as_mut().unwrap().listed_funds.holdings=serde_json::from_value(json!([{"ticker":"TEST11","shares":"0","averagePrice":"0","price":"0","marketValue":"0","unrealizedResult":"0","costBasis":"0"}])).unwrap();
        let captured = capture(&original, "2026-01-09", &[], &rates()).unwrap();
        assert_eq!(captured.invested, "300");
        assert_eq!(captured.portfolio_result, "30");
    }

    #[test]
    fn mixed_component_dates_cannot_create_a_whole_wallet_history_point() {
        let mut original = snapshot();
        original.cash_pockets.push(cash());
        original.broker_position = Some(fixed());
        original.cash_pockets[0].balance_date = Some("2026-01-08".into());
        assert!(capture(&original, "2026-01-09", &[], &rates()).is_none());
    }

    #[test]
    fn missing_purchase_days_are_explicitly_estimated() {
        let mut original = with_fii();
        original.funds[0].entries[0].purchase_date = None;
        let captured = observation(&original, "2026-01-09", "10");
        let report = evolution(
            &original,
            &rates(),
            &[],
            &[],
            Some(&captured),
            day("2026-01-09"),
        )
        .unwrap();
        assert!(!report.points[0].estimated);
        assert!(report.points[0]
            .issues
            .iter()
            .any(|issue| issue.starts_with("CDI: compras antigas")));
        assert_eq!(report.points[0].cdi_result.as_deref(), Some("0"));
    }

    #[test]
    fn last_month_stops_at_the_actual_observation_not_the_machine_date() {
        let original = with_fii();
        let captured = observation(&original, "2026-01-30", "10");
        let report = evolution(
            &original,
            &rates(),
            &[],
            &[],
            Some(&captured),
            day("2026-02-02"),
        )
        .unwrap();
        assert_eq!(report.points.len(), 1);
        assert_eq!(report.as_of.as_deref(), Some("2026-01-30"));
        assert!(report.points[0].is_partial_month);
    }

    #[test]
    fn sub_cent_acquisition_cost_is_not_rounded_into_profit() {
        let mut original = with_fii();
        original.funds[0].entries[0].cotas_added = "1".into();
        original.funds[0].entries[0].price_per_cota = "0.001".into();
        let captured = observation(&original, "2026-01-09", "0");
        let zero_rates = rates()
            .into_iter()
            .map(|mut rate| {
                rate.value = "0".into();
                rate
            })
            .collect::<Vec<_>>();
        let report = evolution(
            &original,
            &zero_rates,
            &[],
            &[],
            Some(&captured),
            day("2026-01-09"),
        )
        .unwrap();
        assert_eq!(report.points[0].invested.as_deref(), Some("0.001"));
        assert_eq!(report.points[0].difference.as_deref(), Some("0"));
    }

    #[test]
    fn a_saved_cost_mismatch_does_not_show_a_misleading_difference() {
        let original = with_fii();
        let mut captured = observation(&original, "2026-01-09", "10");
        captured.invested = "99".into();
        let report = evolution(
            &original,
            &rates(),
            &[],
            &[],
            Some(&captured),
            day("2026-01-09"),
        )
        .unwrap();
        assert!(report.points[0].portfolio_result.is_none());
        assert!(report.points[0].difference.is_none());
    }

    #[test]
    fn future_observations_and_unbounded_year_ranges_are_rejected() {
        let original = with_fii();
        let captured = observation(&original, "2026-03-20", "10");
        assert!(evolution(
            &original,
            &rates(),
            &[],
            &[captured],
            None,
            day("2026-01-09")
        )
        .is_err());
        let mut ancient = original;
        ancient.funds[0].entries[0].month = "0001-01".into();
        ancient.funds[0].entries[0].purchase_date = Some("0001-01-02".into());
        assert!(evolution(&ancient, &rates(), &[], &[], None, day("2026-01-09")).is_err());
    }

    #[test]
    fn an_older_current_quote_does_not_truncate_newer_saved_months() {
        let original = with_fii();
        let current = observation(&original, "2026-01-30", "10");
        let newer = observation(&original, "2026-02-27", "20");
        let report = evolution(
            &original,
            &rates(),
            &[],
            &[newer],
            Some(&current),
            day("2026-03-02"),
        )
        .unwrap();
        assert_eq!(report.points.len(), 2);
        assert_eq!(report.as_of.as_deref(), Some("2026-02-27"));
        assert_eq!(report.points[1].portfolio_result.as_deref(), Some("20"));
    }

    #[test]
    fn monthly_cdi_matches_the_existing_whole_wallet_benchmark() {
        let mut original = snapshot();
        original.cash_pockets.push(cash());
        original.broker_position = Some(fixed());
        let published = rates()
            .into_iter()
            .filter(|rate| day(&rate.date) <= day("2026-01-09"))
            .collect::<Vec<_>>();
        let fii = calculate_fii_performance_with_income(
            &original,
            &[],
            &published,
            &[],
            day("2026-01-09"),
        )
        .unwrap();
        let portfolio = calculate_portfolio_performance(&original, &fii).unwrap();
        let benchmark = calculate_portfolio_benchmark(
            &original,
            &fii,
            &portfolio,
            &published,
            day("2026-01-09"),
        )
        .unwrap();
        let current = observe_portfolio(
            &original,
            &fii,
            &portfolio,
            &benchmark,
            &[],
            day("2026-01-09"),
        )
        .unwrap()
        .unwrap();
        let report = evolution(
            &original,
            &published,
            &[],
            &[],
            Some(&current),
            day("2026-01-09"),
        )
        .unwrap();
        assert_eq!(
            report.points[0].cdi_result.as_deref(),
            benchmark.cdi.as_ref().map(|cdi| cdi.result.as_str())
        );
        assert_eq!(report.points[0].difference, benchmark.difference);
    }

    #[test]
    fn one_pocket_cannot_borrow_the_other_assets_cdi_balance() {
        let mut original = with_fii();
        let mut pocket = cash();
        pocket.balance = "0".into();
        pocket.total_withdrawals = Some("200".into());
        pocket.cash_flows.as_mut().unwrap()[1].amount = "200".into();
        original.cash_pockets.push(pocket);
        let current = capture(&original, "2026-01-09", &[], &rates()).unwrap();
        let report = evolution(
            &original,
            &rates(),
            &[],
            &[],
            Some(&current),
            day("2026-01-09"),
        )
        .unwrap();
        assert!(report.points[0].portfolio_result.is_some());
        assert!(report.points[0].cdi_result.is_none());
        assert!(report.points[0]
            .issues
            .iter()
            .any(|issue| issue.contains("supera o saldo")));
    }

    #[test]
    fn missing_rate_at_the_observed_weekday_does_not_shift_the_real_point() {
        let original = with_fii();
        let current = observation(&original, "2026-01-09", "10");
        let published = rates()
            .into_iter()
            .filter(|rate| day(&rate.date) < day("2026-01-09"))
            .collect::<Vec<_>>();
        let report = evolution(
            &original,
            &published,
            &[],
            &[],
            Some(&current),
            day("2026-01-09"),
        )
        .unwrap();
        assert_eq!(report.points[0].as_of, "2026-01-09");
        assert_eq!(report.points[0].portfolio_result.as_deref(), Some("10"));
        assert!(report.points[0].cdi_result.is_none());
    }

    #[test]
    fn mismatched_cash_flow_totals_leave_the_cdi_line_pending() {
        let mut original = snapshot();
        let mut pocket = cash();
        pocket.cash_flows.as_mut().unwrap()[0].amount = "90".into();
        original.cash_pockets.push(pocket);
        let current = capture(&original, "2026-01-09", &[], &rates()).unwrap();
        let report = evolution(
            &original,
            &rates(),
            &[],
            &[],
            Some(&current),
            day("2026-01-09"),
        )
        .unwrap();
        assert_eq!(report.points[0].portfolio_result.as_deref(), Some("10"));
        assert!(report.points[0].cdi_result.is_none());
    }

    #[test]
    fn event_metadata_changes_do_not_invalidate_an_identical_payment() {
        let original = with_fii();
        let old = event("0.5");
        let mut current = observation(&original, "2026-01-30", "15");
        current.basis_fingerprint =
            basis_fingerprint(&original, &[old.clone()], day("2026-01-30")).unwrap();
        let mut renamed = old;
        renamed.id = "new-provider-id".into();
        renamed.source = "Renamed source".into();
        renamed.source_url = "https://example.test/new".into();
        let report = evolution(
            &original,
            &rates(),
            &[renamed],
            &[current],
            None,
            day("2026-02-02"),
        )
        .unwrap();
        assert_eq!(report.points[0].portfolio_result.as_deref(), Some("15"));
    }

    #[test]
    fn adding_a_known_empty_cash_pocket_preserves_old_financial_history() {
        let mut original = with_fii();
        let captured = observation(&original, "2026-01-09", "10");
        let mut pocket = cash();
        pocket.balance = "0".into();
        pocket.total_deposits = Some("0.00".into());
        pocket.total_withdrawals = Some("0".into());
        pocket.cash_flows = None;
        original.cash_pockets.push(pocket);
        let report = evolution(
            &original,
            &rates(),
            &[],
            &[captured],
            None,
            day("2026-02-02"),
        )
        .unwrap();
        assert_eq!(report.points[0].portfolio_result.as_deref(), Some("10"));
    }

    #[test]
    fn changed_event_identity_that_changes_conflict_resolution_invalidates_result() {
        let original = with_fii();
        let first = event("0.5");
        let mut conflict = event("0.6");
        let mut captured = observation(&original, "2026-01-09", "10");
        captured.basis_fingerprint = basis_fingerprint(
            &original,
            &[first.clone(), conflict.clone()],
            day("2026-01-09"),
        )
        .unwrap();
        conflict.id = "different-economic-payment".into();
        let report = evolution(
            &original,
            &rates(),
            &[first, conflict],
            &[],
            Some(&captured),
            day("2026-01-09"),
        )
        .unwrap();
        assert!(report.points[0].portfolio_result.is_none());
    }

    #[test]
    fn discovering_positive_fixed_income_without_application_dates_invalidates_old_scope() {
        for latest_observation_missing in [true, false] {
            let mut original = with_fii();
            let captured = observation(&original, "2026-01-09", "10");
            let mut position = fixed();
            if latest_observation_missing {
                position.fixed_income.latest_observation = None;
            } else {
                position
                    .fixed_income
                    .latest_observation
                    .as_mut()
                    .unwrap()
                    .contributions
                    .clear();
            }
            original.broker_position = Some(position);
            let report = evolution(
                &original,
                &rates(),
                &[],
                &[captured],
                None,
                day("2026-02-02"),
            )
            .unwrap();
            assert!(report.points[0].portfolio_result.is_none());
            assert!(report.points[0]
                .issues
                .iter()
                .any(|issue| issue.contains("alterados")));
        }
    }

    #[test]
    fn unknown_fixed_income_scope_is_stable_for_value_updates_but_not_new_principal() {
        let mut original = with_fii();
        let mut position = fixed();
        position
            .fixed_income
            .latest_observation
            .as_mut()
            .unwrap()
            .contributions
            .clear();
        original.broker_position = Some(position);
        let before = basis_fingerprint(&original, &[], day("2026-01-09")).unwrap();
        let latest = original
            .broker_position
            .as_mut()
            .unwrap()
            .fixed_income
            .latest_observation
            .as_mut()
            .unwrap();
        latest.balance = "120".into();
        latest.total_earnings = "20".into();
        latest.reference_date = "2026-02-02".into();
        assert_eq!(
            before,
            basis_fingerprint(&original, &[], day("2026-01-09")).unwrap()
        );
        original
            .broker_position
            .as_mut()
            .unwrap()
            .fixed_income
            .latest_observation
            .as_mut()
            .unwrap()
            .invested = "200".into();
        assert_ne!(
            before,
            basis_fingerprint(&original, &[], day("2026-01-09")).unwrap()
        );
    }

    #[test]
    fn a_known_zero_or_explicitly_future_fixed_position_preserves_previous_scope() {
        let mut original = with_fii();
        let captured = observation(&original, "2026-01-09", "10");
        let mut position = fixed();
        position.fixed_income.latest_observation = None;
        position.fixed_income.balance = "0.00".into();
        original.broker_position = Some(position);
        let report = evolution(
            &original,
            &rates(),
            &[],
            &[captured.clone()],
            None,
            day("2026-02-02"),
        )
        .unwrap();
        assert_eq!(report.points[0].portfolio_result.as_deref(), Some("10"));
        let mut position = fixed();
        position
            .fixed_income
            .latest_observation
            .as_mut()
            .unwrap()
            .reference_date = "2026-02-02".into();
        position
            .fixed_income
            .latest_observation
            .as_mut()
            .unwrap()
            .contributions[0]
            .application_date = "2026-02-02".into();
        original.broker_position = Some(position);
        let report = evolution(
            &original,
            &rates(),
            &[],
            &[captured],
            None,
            day("2026-02-02"),
        )
        .unwrap();
        assert_eq!(report.points[0].portfolio_result.as_deref(), Some("10"));
    }
}
