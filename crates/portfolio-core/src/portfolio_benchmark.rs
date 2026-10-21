//! A matched-flow, 100% historical CDI comparison for the selected wallet.
//! Each position keeps its observed valuation date. No balance, acquisition,
//! payment or deposit is inferred from the account's aggregate broker snapshot.
use super::{
    add, decimal_string, div, mul, normalize_purchase_date, parse_decimal, parse_nonnegative,
    performance::{CdiPerformance, DailyCdiRate, FiiPerformance},
    portfolio_performance::PortfolioPerformance,
    CoreError, PortfolioSnapshot, Result,
};
use chrono::{Datelike, NaiveDate, Weekday};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioBenchmarkComponent {
    /// `fiis`, `fixedIncome` or `cash`; cash has one component per pocket.
    pub kind: String,
    pub id: Option<String>,
    pub name: String,
    pub start_date: Option<String>,
    pub as_of: Option<String>,
    pub invested: Option<String>,
    pub result: Option<String>,
    pub cdi: Option<CdiPerformance>,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioBenchmark {
    /// `available`, `incomplete` or `empty`. Actual results can be known even
    /// when the matched CDI scenario is unavailable.
    pub status: String,
    pub start_date: Option<String>,
    /// Present only when every component has the same valuation date.
    pub as_of: Option<String>,
    pub result: Option<String>,
    pub invested: Option<String>,
    pub return_percent: Option<String>,
    pub cdi: Option<CdiPerformance>,
    pub difference: Option<String>,
    pub components: Vec<PortfolioBenchmarkComponent>,
    pub issues: Vec<String>,
    pub assumptions: Vec<String>,
}

fn invalid(message: &str) -> CoreError {
    CoreError::Validation(message.into())
}

fn date(value: &str) -> Result<NaiveDate> {
    let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| invalid("Data inválida na comparação da carteira com o CDI."))?;
    if parsed.year() < 1 || parsed.format("%Y-%m-%d").to_string() != value {
        return Err(invalid(
            "Data inválida na comparação da carteira com o CDI.",
        ));
    }
    Ok(parsed)
}

fn subtract(left: Decimal, right: Decimal) -> Result<Decimal> {
    left.checked_sub(right).ok_or(CoreError::ArithmeticOverflow)
}

fn percent(result: Decimal, invested: Decimal) -> Result<Decimal> {
    if invested.is_zero() {
        Ok(Decimal::ZERO)
    } else {
        mul(div(result, invested)?, Decimal::ONE_HUNDRED)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Flow {
    pub(crate) deposits: Decimal,
    pub(crate) withdrawals: Decimal,
}

pub(crate) fn add_flow(
    flows: &mut BTreeMap<NaiveDate, Flow>,
    day: NaiveDate,
    deposit: Decimal,
    withdrawal: Decimal,
) -> Result<()> {
    let current = flows.entry(day).or_default();
    current.deposits = add(current.deposits, deposit)?;
    current.withdrawals = add(current.withdrawals, withdrawal)?;
    Ok(())
}

fn apply_flow(balance: Decimal, flow: &Flow) -> Result<Decimal> {
    // Same-day movements are settled together. A benchmark cannot borrow at
    // CDI to fund a withdrawal that its simulated balance does not support.
    let available = add(balance, flow.deposits)?;
    if flow.withdrawals > available {
        return Err(invalid(
            "Uma retirada supera o saldo do cenário CDI naquela data; não foi presumido empréstimo.",
        ));
    }
    subtract(available, flow.withdrawals)
}

pub(crate) fn calculate_flow_cdi(
    flows: &BTreeMap<NaiveDate, Flow>,
    rates: &[DailyCdiRate],
    as_of: NaiveDate,
    today: NaiveDate,
    invested: Decimal,
    withdrawals: Decimal,
) -> Result<CdiPerformance> {
    let Some(first_flow) = flows.keys().next().copied() else {
        if invested.is_zero() && withdrawals.is_zero() {
            return Ok(CdiPerformance {
                ending_value: "0".into(),
                result: "0".into(),
                return_percent: "0".into(),
            });
        }
        return Err(invalid(
            "Faltam as datas dos aportes para comparar com o CDI.",
        ));
    };
    if as_of > today || flows.keys().any(|day| *day > as_of) {
        return Err(invalid(
            "Há saldo ou movimentação posterior à data final da comparação.",
        ));
    }
    let mut daily = BTreeMap::new();
    for observation in rates {
        let day = date(&observation.date)?;
        if day > today {
            return Err(invalid("O histórico CDI contém uma data futura."));
        }
        let rate = parse_nonnegative(&observation.value, "Taxa CDI diária")?;
        if daily.insert(day, rate).is_some() {
            return Err(invalid("O histórico CDI contém datas duplicadas."));
        }
    }
    daily.retain(|day, _| *day <= as_of);
    // Saturdays and Sundays have no daily CDI. Their closing balance uses
    // Friday's published observation, without inventing interest for weekend
    // days. A missing Friday or a weekday holiday remains pending coverage.
    let last_rate_day = match as_of.weekday() {
        Weekday::Sat => as_of.pred_opt(),
        Weekday::Sun => as_of.pred_opt().and_then(|day| day.pred_opt()),
        _ => Some(as_of),
    }
    .ok_or_else(|| invalid("Data final fora do intervalo suportado."))?;
    if daily.keys().next_back().copied() != Some(last_rate_day) {
        return Err(invalid(
            "O histórico diário CDI ainda não cobre a data deste saldo.",
        ));
    }
    if daily.keys().next().is_none_or(|day| *day > first_flow) {
        return Err(invalid(
            "O histórico diário CDI não cobre o primeiro aporte.",
        ));
    }
    // Match the public SGS cache's conservative coverage policy. Short gaps
    // can be weekends/holidays; a longer gap cannot silently earn zero CDI.
    let coverage_start = daily
        .range(..=first_flow)
        .next_back()
        .map(|(day, _)| *day)
        .expect("the first-flow boundary was checked");
    let mut previous: Option<NaiveDate> = None;
    for day in daily.keys().copied().filter(|day| *day >= coverage_start) {
        if previous.is_some_and(|previous| (day - previous).num_days() > 7) {
            return Err(invalid("O histórico diário CDI tem uma lacuna superior a sete dias no período deste investimento."));
        }
        previous = Some(day);
    }
    let mut balance = Decimal::ZERO;
    let mut pending = flows.iter().peekable();
    for (day, rate) in daily {
        while pending.peek().is_some_and(|(flow_day, _)| **flow_day < day) {
            balance = apply_flow(balance, pending.next().expect("checked movement").1)?;
        }
        balance = mul(
            balance,
            add(Decimal::ONE, div(rate, Decimal::ONE_HUNDRED)?)?,
        )?;
        // A deposit or withdrawal settles at the end of its day, after that
        // day's rate; deposits start earning strictly on the next observation.
        while pending
            .peek()
            .is_some_and(|(flow_day, _)| **flow_day == day)
        {
            balance = apply_flow(balance, pending.next().expect("checked movement").1)?;
        }
    }
    for (_, flow) in pending {
        balance = apply_flow(balance, flow)?;
    }
    let result = subtract(add(balance, withdrawals)?, invested)?;
    Ok(CdiPerformance {
        ending_value: decimal_string(balance),
        result: decimal_string(result),
        return_percent: decimal_string(percent(result, invested)?),
    })
}

fn component(
    kind: &str,
    name: String,
    invested: Option<String>,
    result: Option<String>,
    as_of: Option<String>,
) -> PortfolioBenchmarkComponent {
    PortfolioBenchmarkComponent {
        kind: kind.into(),
        id: None,
        name,
        start_date: None,
        as_of,
        invested,
        result,
        cdi: None,
        issues: vec![],
    }
}

fn finish_component(
    item: &mut PortfolioBenchmarkComponent,
    flows: &BTreeMap<NaiveDate, Flow>,
    rates: &[DailyCdiRate],
    today: NaiveDate,
    withdrawals: Decimal,
) -> Result<()> {
    item.start_date = flows.keys().next().map(ToString::to_string);
    let Some(as_of) = item.as_of.as_deref() else {
        item.issues
            .push("Falta a data do saldo para comparar com o CDI.".into());
        return Ok(());
    };
    let as_of = date(as_of)?;
    if as_of > today {
        item.issues.push(
            "A data deste saldo é futura; a comparação aguarda uma observação válida.".into(),
        );
        return Ok(());
    }
    let Some(invested) = item.invested.as_deref() else {
        item.issues
            .push("Falta o valor aplicado para comparar com o CDI.".into());
        return Ok(());
    };
    if !item.issues.is_empty() || item.result.is_none() {
        return Ok(());
    }
    match calculate_flow_cdi(
        flows,
        rates,
        as_of,
        today,
        parse_nonnegative(invested, "Valor aplicado")?,
        withdrawals,
    ) {
        Ok(value) => item.cdi = Some(value),
        Err(error) => item.issues.push(error.to_string()),
    }
    Ok(())
}

fn fii_start(snapshot: &PortfolioSnapshot, as_of: NaiveDate) -> Result<Option<String>> {
    let mut first = None;
    for fund in &snapshot.funds {
        for entry in &fund.entries {
            let quantity = parse_nonnegative(&entry.cotas_added, "Cotas compradas")?;
            let cost = mul(
                quantity,
                parse_nonnegative(&entry.price_per_cota, "Preço por cota")?,
            )?;
            if cost.is_zero() {
                continue;
            }
            let month = date(&format!("{}-01", entry.month))?;
            if month.format("%Y-%m").to_string() > as_of.format("%Y-%m").to_string() {
                continue;
            }
            let day = match normalize_purchase_date(
                entry.purchase_date.as_deref(),
                &entry.month,
                quantity,
            )? {
                Some(value) => date(&value)?,
                None => {
                    let next = if month.month() == 12 {
                        NaiveDate::from_ymd_opt(month.year() + 1, 1, 1)
                    } else {
                        NaiveDate::from_ymd_opt(month.year(), month.month() + 1, 1)
                    };
                    next.and_then(|day| day.pred_opt())
                        .ok_or_else(|| invalid("Mês inválido no histórico."))?
                        .min(as_of)
                }
            };
            if day <= as_of && first.is_none_or(|previous| day < previous) {
                first = Some(day);
            }
        }
    }
    Ok(first.map(|day| day.to_string()))
}

/// Compare the observed wallet result with 100% CDI using each component's
/// actual external movements. This never creates ledger or payment records.
pub fn calculate_portfolio_benchmark(
    snapshot: &PortfolioSnapshot,
    fii: &FiiPerformance,
    portfolio: &PortfolioPerformance,
    rates: &[DailyCdiRate],
    today: NaiveDate,
) -> Result<PortfolioBenchmark> {
    let mut components = vec![];
    let mut assumptions = vec![
        "Carteira inteira: FIIs, pós-fixado e caixinhas, sem somar o patrimônio agregado da corretora.".into(),
        "Cada investimento é comparado com 100% do CDI diário histórico SGS 12, usando os mesmos aportes, retiradas e a data do respectivo saldo.".into(),
        "Aportes e retiradas ocorrem no fim do dia; aportes começam a render na próxima observação do CDI. O cenário é bruto, sem impostos ou taxas.".into(),
        "Saldos de sábado e domingo usam o último CDI publicado na sexta-feira, sem juros extras no fim de semana. Feriados sem cobertura permanecem pendentes.".into(),
        "A porcentagem é o ganho acumulado dividido pelo total aplicado; não é uma taxa anualizada nem ajustada pelo tempo dos aportes.".into(),
    ];
    if fii.status != "empty" {
        let mut item = component(
            "fiis",
            "FIIs".into(),
            Some(fii.invested.clone()),
            fii.total_result.clone(),
            fii.as_of.clone(),
        );
        item.start_date = fii_start(
            snapshot,
            fii.as_of.as_deref().map(date).transpose()?.unwrap_or(today),
        )?;
        item.cdi = fii.cdi.clone();
        item.issues = fii.issues.clone();
        if item.result.is_none() {
            item.issues
                .push("O resultado dos FIIs está incompleto.".into());
        }
        if item.cdi.is_none() && item.issues.is_empty() {
            item.issues
                .push("O histórico CDI ainda não cobre o período dos FIIs.".into());
        }
        assumptions.push("Nos FIIs, compras sem dia informado usam o fim do mês; no mês final, usam a data da cotação. Proventos manuais e da API já estão no resultado e não são somados novamente nem reinvestidos no cenário.".into());
        components.push(item);
    }
    if portfolio.fixed_income.status != "empty" {
        let fixed = &portfolio.fixed_income;
        let mut item = component(
            "fixedIncome",
            fixed.name.clone().unwrap_or_else(|| "Pós-fixado".into()),
            fixed.invested.clone(),
            fixed.result.clone(),
            fixed.reference_date.clone(),
        );
        let mut flows = BTreeMap::new();
        if let Some(observation) = snapshot
            .broker_position
            .as_ref()
            .and_then(|position| position.fixed_income.latest_observation.as_ref())
        {
            let mut invested = Decimal::ZERO;
            let mut current = Decimal::ZERO;
            let mut earnings = Decimal::ZERO;
            for contribution in &observation.contributions {
                let amount = parse_nonnegative(&contribution.invested, "Aporte no pós-fixado")?;
                let day = date(&contribution.application_date)?;
                invested = add(invested, amount)?;
                current = add(
                    current,
                    parse_nonnegative(&contribution.current_value, "Saldo do aporte")?,
                )?;
                earnings = add(earnings, parse_decimal(&contribution.earnings)?)?;
                if !amount.is_zero() {
                    add_flow(&mut flows, day, amount, Decimal::ZERO)?;
                }
            }
            if observation.contributions.is_empty() {
                item.issues.push(
                    "Faltam os aportes com data do pós-fixado para comparar com o CDI.".into(),
                );
            } else if invested
                != parse_nonnegative(&observation.invested, "Valor aplicado no pós-fixado")?
                || current != parse_nonnegative(&observation.balance, "Saldo do pós-fixado")?
                || earnings != parse_decimal(&observation.total_earnings)?
            {
                item.issues.push("Os aportes do pós-fixado não conferem com o total observado; revise a importação antes de comparar com o CDI.".into());
            }
        } else {
            item.issues
                .push("Falta uma observação do pós-fixado com os aportes e suas datas.".into());
        }
        finish_component(&mut item, &flows, rates, today, Decimal::ZERO)?;
        components.push(item);
    }
    for pocket in &snapshot.cash_pockets {
        let accounting = portfolio
            .cash
            .items
            .iter()
            .find(|item| item.id == pocket.id)
            .ok_or_else(|| invalid("A caixinha não está presente nos totais da carteira."))?;
        let mut item = component(
            "cash",
            pocket.name.clone(),
            accounting.invested.clone(),
            accounting.income.clone(),
            accounting.balance_date.clone(),
        );
        item.id = Some(pocket.id.clone());
        let mut flows = BTreeMap::new();
        let mut deposits = Decimal::ZERO;
        let mut withdrawals = Decimal::ZERO;
        if let Some(movements) = &pocket.cash_flows {
            for movement in movements {
                let day = date(&movement.date)?;
                let amount = parse_nonnegative(&movement.amount, "Movimentação da caixinha")?;
                if amount.is_zero() {
                    item.issues
                        .push("Uma movimentação da caixinha está sem valor positivo.".into());
                    continue;
                }
                match movement.kind.as_str() {
                    "deposit" => {
                        deposits = add(deposits, amount)?;
                        add_flow(&mut flows, day, amount, Decimal::ZERO)?;
                    }
                    "withdrawal" => {
                        withdrawals = add(withdrawals, amount)?;
                        add_flow(&mut flows, day, Decimal::ZERO, amount)?;
                    }
                    _ => item
                        .issues
                        .push("Há uma movimentação de caixinha com tipo inválido.".into()),
                }
            }
            if accounting
                .invested
                .as_deref()
                .map(parse_decimal)
                .transpose()?
                != Some(deposits)
                || accounting
                    .withdrawals
                    .as_deref()
                    .map(parse_decimal)
                    .transpose()?
                    != Some(withdrawals)
            {
                item.issues.push("Os movimentos com data não conferem com o total depositado e retirado da caixinha.".into());
            }
        } else if accounting.invested.as_deref() != Some("0")
            || accounting.withdrawals.as_deref() != Some("0")
        {
            item.issues.push("Informe as datas e os valores dos depósitos e retiradas desta caixinha para comparar com o CDI.".into());
        }
        finish_component(&mut item, &flows, rates, today, withdrawals)?;
        components.push(item);
    }
    let start_date = components
        .iter()
        .filter_map(|item| item.start_date.clone())
        .min();
    let dates: BTreeSet<_> = components
        .iter()
        .filter_map(|item| item.as_of.clone())
        .collect();
    let common_date = !components.is_empty()
        && dates.len() == 1
        && components.iter().all(|item| item.as_of.is_some());
    let as_of = common_date.then(|| dates.iter().next().expect("one date").clone());
    if dates.len() > 1 {
        assumptions.push("Os saldos têm datas diferentes. A comparação soma cenários que terminam na data de cada investimento; consulte as datas na tabela. Não representa toda a carteira em um único dia.".into());
    }
    let mut issues = vec![];
    for item in &components {
        for issue in &item.issues {
            issues.push(format!("{}: {issue}", item.name));
        }
    }
    let actual_result = portfolio
        .total_result
        .as_deref()
        .map(parse_decimal)
        .transpose()?;
    let invested = portfolio
        .invested
        .as_deref()
        .map(parse_decimal)
        .transpose()?;
    let mut cdi_balance = Decimal::ZERO;
    let mut cdi_result = Decimal::ZERO;
    let mut component_result = Decimal::ZERO;
    let mut component_invested = Decimal::ZERO;
    let mut complete = portfolio.status == "available" && !components.is_empty();
    for item in &components {
        match (&item.cdi, item.result.as_deref(), item.invested.as_deref()) {
            (Some(cdi), Some(result), Some(cost)) => {
                cdi_balance = add(
                    cdi_balance,
                    parse_nonnegative(&cdi.ending_value, "Saldo CDI")?,
                )?;
                cdi_result = add(cdi_result, parse_decimal(&cdi.result)?)?;
                component_result = add(component_result, parse_decimal(result)?)?;
                component_invested = add(
                    component_invested,
                    parse_nonnegative(cost, "Valor aplicado")?,
                )?;
            }
            _ => complete = false,
        }
    }
    if complete && (actual_result != Some(component_result) || invested != Some(component_invested))
    {
        complete = false;
        issues.push("Os componentes da comparação não conferem com os totais da carteira.".into());
    }
    if portfolio.status != "available" && portfolio.status != "empty" {
        issues.extend(portfolio.issues.iter().cloned());
    }
    let cdi = if complete {
        Some(CdiPerformance {
            ending_value: decimal_string(cdi_balance),
            result: decimal_string(cdi_result),
            return_percent: decimal_string(percent(cdi_result, invested.expect("complete cost"))?),
        })
    } else {
        None
    };
    let difference = if complete {
        Some(decimal_string(subtract(
            actual_result.expect("complete result"),
            cdi_result,
        )?))
    } else {
        None
    };
    Ok(PortfolioBenchmark {
        status: if components.is_empty() {
            "empty"
        } else if complete {
            "available"
        } else {
            "incomplete"
        }
        .into(),
        start_date,
        as_of,
        result: portfolio.total_result.clone(),
        invested: portfolio.invested.clone(),
        return_percent: portfolio.total_return_percent.clone(),
        cdi,
        difference,
        components,
        issues,
        assumptions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        calculate_fund,
        performance::{calculate_fii_performance, FiiIncomeBreakdown, PerformanceQuote},
        portfolio_performance::calculate_portfolio_performance,
        BrokerPosition, CashPocket, MonthlyEntry, PortfolioTotals,
    };
    use serde_json::json;

    fn today() -> NaiveDate {
        date("2026-01-09").unwrap()
    }

    fn rates() -> Vec<DailyCdiRate> {
        [
            "2026-01-02",
            "2026-01-05",
            "2026-01-06",
            "2026-01-07",
            "2026-01-08",
            "2026-01-09",
        ]
        .iter()
        .map(|day| DailyCdiRate {
            date: (*day).into(),
            value: "1".into(),
        })
        .collect()
    }

    fn snapshot() -> PortfolioSnapshot {
        PortfolioSnapshot {
            funds: vec![],
            totals: PortfolioTotals {
                invested: "99999".into(),
                cotas: "99999".into(),
                dividends_received: "99999".into(),
                estimated_monthly_income: "99999".into(),
            },
            record_count: 0,
            broker_position: None,
            cash_pockets: vec![],
            cash_balance: "99999".into(),
            performance_history: vec![],
        }
    }

    fn fixed() -> BrokerPosition {
        serde_json::from_value(json!({
            "referenceDate": "2026-01-09", "institution": "Corretora sintética",
            "importedAt": "2026-01-09T12:00:00Z", "sourcePath": "fixture.json", "sourceHash": "a".repeat(64),
            "overview": { "totalWithCash": "99999", "invested": "99999", "cash": "99999", "listedFunds": "99999", "fixedIncome": "99999", "pendingIncome": "99999" },
            "listedFunds": { "marketValue": "99999", "costBasis": "99999", "unrealizedResult": "99999", "dailyResult": "99999", "holdings": [] },
            "fixedIncome": {
                "name": "Nome antigo", "nameTruncated": true, "balance": "99999", "netBalance": "99999", "reportedEarnings": "99999", "return12Months": "99999", "redemptionConversion": "D+0", "redemptionSettlement": "D+0",
                "latestObservation": {
                    "referenceDate": "2026-01-09", "name": "Pós-fixado sintético", "category": "Renda fixa", "riskLevel": "1", "balance": "165", "invested": "150", "totalEarnings": "15",
                    "earnings12Months": "99999", "return12Months": "99999", "returnReferenceDate": "2026-01-08",
                    "contributions": [
                        { "applicationDate": "2026-01-02", "invested": "100", "currentValue": "110", "earnings": "10", "returnPercent": "10" },
                        { "applicationDate": "2026-01-07", "invested": "50", "currentValue": "55", "earnings": "5", "returnPercent": "10" }
                    ],
                    "sources": [], "importedAt": "2026-01-09T12:00:00Z", "sourcePath": "fixture.json", "sourceHash": "b".repeat(64)
                }
            }, "warnings": []
        })).unwrap()
    }

    fn cash() -> CashPocket {
        serde_json::from_value(json!({
            "id": "cash-fixture", "name": "Caixinha sintética", "institution": "Teste",
            "balance": "170", "totalDeposits": "200", "totalWithdrawals": "50", "balanceDate": "2026-01-09",
            "cashFlows": [{ "date": "2026-01-05", "kind": "deposit", "amount": "200" }, { "date": "2026-01-08", "kind": "withdrawal", "amount": "50" }],
            "cdiPercent": "120", "annualCdiRate": "99999", "notes": "Sintético",
            "createdAt": "2020-01-01T12:00:00Z", "updatedAt": "2026-01-09T12:00:00Z"
        })).unwrap()
    }

    fn fii(snapshot: &mut PortfolioSnapshot) -> FiiPerformance {
        snapshot.funds = vec![calculate_fund(
            "fii".into(),
            "TEST11".into(),
            "Fundo sintético".into(),
            "Teste".into(),
            vec![MonthlyEntry {
                id: "purchase-fixture".into(),
                month: "2026-01".into(),
                cotas_added: "10".into(),
                purchase_date: Some("2026-01-05".into()),
                price_per_cota: "10".into(),
                dividend_per_cota: "0".into(),
                dividend_total: Some("5".into()),
                notes: "Sintético".into(),
            }],
        )
        .unwrap()];
        snapshot.record_count = 1;
        let mut result = calculate_fii_performance(
            snapshot,
            &[PerformanceQuote {
                ticker: "TEST11".into(),
                price: "11".into(),
                as_of: "2026-01-09".into(),
            }],
            &rates(),
            today(),
        )
        .unwrap();
        // Simulate the already reconciled report boundary: API supplements have
        // been counted once upstream, with manual/API provenance preserved.
        result.dividends = "7".into();
        result.total_result = Some("17".into());
        result.total_return_percent = Some("17".into());
        result.income = Some(FiiIncomeBreakdown {
            recorded_amount: "5".into(),
            api_amount: "2".into(),
            as_of: "2026-01-09".into(),
            start_month: "2026-01".into(),
            end_month: "2026-01".into(),
            status: "complete".into(),
            missing_fund_month_count: 0,
            date_required_fund_month_count: 0,
            ignored_event_count: 0,
            by_fund: vec![],
        });
        result
    }

    fn benchmark(
        snapshot: &PortfolioSnapshot,
        fii: &FiiPerformance,
        rates: &[DailyCdiRate],
    ) -> PortfolioBenchmark {
        let accounting = calculate_portfolio_performance(snapshot, fii).unwrap();
        calculate_portfolio_benchmark(snapshot, fii, &accounting, rates, today()).unwrap()
    }

    fn empty_fii(snapshot: &PortfolioSnapshot) -> FiiPerformance {
        calculate_fii_performance(snapshot, &[], &rates(), today()).unwrap()
    }

    #[test]
    fn all_components_match_their_flows_and_do_not_duplicate_income_or_broker_totals() {
        let mut wallet = snapshot();
        let fii = fii(&mut wallet);
        wallet.broker_position = Some(fixed());
        wallet.cash_pockets = vec![cash()];
        let unchanged = serde_json::to_value(&wallet).unwrap();
        let actual = benchmark(&wallet, &fii, &rates());
        assert_eq!(actual.status, "available");
        assert_eq!(actual.start_date.as_deref(), Some("2026-01-02"));
        assert_eq!(actual.as_of.as_deref(), Some("2026-01-09"));
        assert_eq!(actual.invested.as_deref(), Some("450"));
        assert_eq!(actual.result.as_deref(), Some("52"));
        assert_eq!(actual.components.len(), 3);
        assert_eq!(actual.components[0].result.as_deref(), Some("17"));
        assert_eq!(actual.components[1].result.as_deref(), Some("15"));
        assert_eq!(actual.components[2].result.as_deref(), Some("20"));
        let expected_cdi = add(
            add(
                Decimal::from_str_exact("4.060401").unwrap(),
                Decimal::from_str_exact("6.10600501").unwrap(),
            )
            .unwrap(),
            Decimal::from_str_exact("7.620802").unwrap(),
        )
        .unwrap();
        assert_eq!(
            actual.cdi.as_ref().unwrap().result,
            decimal_string(expected_cdi)
        );
        assert_eq!(
            actual.difference.as_deref(),
            Some(decimal_string(subtract(Decimal::from(52), expected_cdi).unwrap()).as_str())
        );
        assert_eq!(serde_json::to_value(&wallet).unwrap(), unchanged);
    }

    #[test]
    fn fixed_income_only_uses_the_earliest_application_date() {
        let mut wallet = snapshot();
        let fii = empty_fii(&wallet);
        wallet.broker_position = Some(fixed());
        let actual = benchmark(&wallet, &fii, &rates());
        assert_eq!(actual.status, "available");
        assert_eq!(actual.components.len(), 1);
        assert_eq!(actual.start_date.as_deref(), Some("2026-01-02"));
        assert_eq!(actual.result.as_deref(), Some("15"));
        assert_eq!(actual.cdi.unwrap().result, "6.10600501");
    }

    #[test]
    fn cash_only_includes_gross_withdrawals_and_uses_historical_100_percent_cdi() {
        let mut wallet = snapshot();
        let fii = empty_fii(&wallet);
        wallet.cash_pockets = vec![cash()];
        let actual = benchmark(&wallet, &fii, &rates());
        assert_eq!(actual.status, "available");
        assert_eq!(actual.start_date.as_deref(), Some("2026-01-05"));
        assert_eq!(actual.result.as_deref(), Some("20"));
        let cdi = actual.cdi.unwrap();
        assert_eq!(cdi.ending_value, "157.620802");
        assert_eq!(cdi.result, "7.620802");
        assert_eq!(cdi.return_percent, "3.810401");
    }

    #[test]
    fn missing_cash_flow_dates_preserves_accounting_but_withholds_the_whole_benchmark() {
        let mut wallet = snapshot();
        let fii = empty_fii(&wallet);
        let mut pocket = cash();
        pocket.cash_flows = None;
        wallet.cash_pockets = vec![pocket];
        let actual = benchmark(&wallet, &fii, &rates());
        assert_eq!(actual.status, "incomplete");
        assert_eq!(actual.result.as_deref(), Some("20"));
        assert!(actual.cdi.is_none());
        assert!(actual.components[0].start_date.is_none());
        assert_eq!(actual.components[0].as_of.as_deref(), Some("2026-01-09"));
        assert!(actual
            .issues
            .iter()
            .any(|issue| issue.contains("datas e os valores")));
    }

    #[test]
    fn movement_and_contribution_totals_must_reconcile() {
        let mut wallet = snapshot();
        let fii = empty_fii(&wallet);
        let mut pocket = cash();
        pocket.cash_flows.as_mut().unwrap()[0].amount = "199".into();
        wallet.cash_pockets = vec![pocket];
        let mut position = fixed();
        position
            .fixed_income
            .latest_observation
            .as_mut()
            .unwrap()
            .contributions[0]
            .invested = "99".into();
        wallet.broker_position = Some(position);
        let actual = benchmark(&wallet, &fii, &rates());
        assert_eq!(actual.status, "incomplete");
        assert!(actual.cdi.is_none());
        assert_eq!(
            actual.components[0].start_date.as_deref(),
            Some("2026-01-02")
        );
        assert_eq!(
            actual.components[1].start_date.as_deref(),
            Some("2026-01-05")
        );
        assert!(actual
            .components
            .iter()
            .all(|component| component.cdi.is_none()));
        assert_eq!(actual.issues.len(), 2);
    }

    #[test]
    fn differing_observation_dates_are_explicit_and_each_scenario_uses_its_own_cutoff() {
        let mut wallet = snapshot();
        let fii = fii(&mut wallet);
        let mut position = fixed();
        position
            .fixed_income
            .latest_observation
            .as_mut()
            .unwrap()
            .reference_date = "2026-01-08".into();
        wallet.broker_position = Some(position);
        let actual = benchmark(&wallet, &fii, &rates());
        assert_eq!(actual.status, "available");
        assert!(actual.as_of.is_none());
        assert_eq!(actual.components[0].as_of.as_deref(), Some("2026-01-09"));
        assert_eq!(actual.components[1].as_of.as_deref(), Some("2026-01-08"));
        assert_eq!(
            actual.components[1].cdi.as_ref().unwrap().result,
            "4.560401"
        );
        assert!(actual
            .assumptions
            .iter()
            .any(|assumption| assumption.contains("datas diferentes")));
    }

    #[test]
    fn benchmark_does_not_fund_withdrawals_by_borrowing() {
        let mut wallet = snapshot();
        let fii = empty_fii(&wallet);
        let mut pocket = cash();
        pocket.balance = "0".into();
        pocket.total_withdrawals = Some("250".into());
        pocket.cash_flows.as_mut().unwrap()[1].amount = "250".into();
        wallet.cash_pockets = vec![pocket];
        let actual = benchmark(&wallet, &fii, &rates());
        assert_eq!(actual.status, "incomplete");
        assert_eq!(actual.result.as_deref(), Some("50"));
        assert!(actual.cdi.is_none());
        assert!(actual
            .issues
            .iter()
            .any(|issue| issue.contains("empréstimo")));
    }

    #[test]
    fn same_day_net_flows_settle_after_interest_and_zero_rates_do_not_invent_losses() {
        let mut flows = BTreeMap::new();
        add_flow(
            &mut flows,
            date("2026-01-05").unwrap(),
            Decimal::from_str_exact("100.123").unwrap(),
            Decimal::from(40),
        )
        .unwrap();
        let zero = rates()
            .into_iter()
            .map(|observation| DailyCdiRate {
                value: "0".into(),
                ..observation
            })
            .collect::<Vec<_>>();
        let cdi = calculate_flow_cdi(
            &flows,
            &zero,
            today(),
            today(),
            Decimal::from_str_exact("100.123").unwrap(),
            Decimal::from(40),
        )
        .unwrap();
        assert_eq!(cdi.ending_value, "60.123");
        assert_eq!(cdi.result, "0");
        let daily = calculate_flow_cdi(
            &flows,
            &rates(),
            date("2026-01-05").unwrap(),
            today(),
            Decimal::from_str_exact("100.123").unwrap(),
            Decimal::from(40),
        )
        .unwrap();
        assert_eq!(daily.result, "0");
    }

    #[test]
    fn incomplete_cdi_boundaries_preserve_requested_component_dates() {
        let mut wallet = snapshot();
        let fii = empty_fii(&wallet);
        wallet.broker_position = Some(fixed());
        for historical in [rates()[1..].to_vec(), rates()[..5].to_vec()] {
            let actual = benchmark(&wallet, &fii, &historical);
            assert_eq!(actual.status, "incomplete");
            assert_eq!(actual.start_date.as_deref(), Some("2026-01-02"));
            assert_eq!(actual.as_of.as_deref(), Some("2026-01-09"));
            assert_eq!(actual.result.as_deref(), Some("15"));
            assert!(actual.cdi.is_none());
        }
    }

    #[test]
    fn invalid_daily_rates_are_not_replaced_by_an_annual_projection() {
        let mut wallet = snapshot();
        let fii = empty_fii(&wallet);
        wallet.cash_pockets = vec![cash()];
        let mut duplicate = rates();
        duplicate.push(duplicate[0].clone());
        let mut negative = rates();
        negative[0].value = "-1".into();
        let mut future = rates();
        future.push(DailyCdiRate {
            date: "2026-01-12".into(),
            value: "1".into(),
        });
        for historical in [duplicate, negative, future] {
            let actual = benchmark(&wallet, &fii, &historical);
            assert_eq!(actual.status, "incomplete");
            assert!(actual.cdi.is_none());
        }
    }

    #[test]
    fn post_fixed_without_dated_applications_keeps_its_result_and_requires_dates() {
        let mut wallet = snapshot();
        let fii = empty_fii(&wallet);
        let mut position = fixed();
        position
            .fixed_income
            .latest_observation
            .as_mut()
            .unwrap()
            .contributions
            .clear();
        wallet.broker_position = Some(position);
        let actual = benchmark(&wallet, &fii, &rates());
        assert_eq!(actual.status, "incomplete");
        assert_eq!(actual.result.as_deref(), Some("15"));
        assert_eq!(actual.invested.as_deref(), Some("150"));
        assert!(actual.start_date.is_none());
        assert!(actual.cdi.is_none());
        assert!(actual
            .issues
            .iter()
            .any(|issue| issue.contains("aportes com data")));
    }

    #[test]
    fn legacy_month_only_fii_purchase_uses_its_existing_estimated_date() {
        let mut wallet = snapshot();
        let _ = fii(&mut wallet);
        wallet.funds[0].entries[0].purchase_date = None;
        let fii = calculate_fii_performance(
            &wallet,
            &[PerformanceQuote {
                ticker: "TEST11".into(),
                price: "11".into(),
                as_of: "2026-01-09".into(),
            }],
            &rates(),
            today(),
        )
        .unwrap();
        let actual = benchmark(&wallet, &fii, &rates());
        assert_eq!(actual.status, "available");
        assert_eq!(actual.start_date.as_deref(), Some("2026-01-09"));
        assert_eq!(actual.cdi.unwrap().result, "0");
        assert!(actual
            .assumptions
            .iter()
            .any(|assumption| assumption.contains("fim do mês")));
    }

    #[test]
    fn movement_after_the_cash_balance_is_not_included_in_a_past_scenario() {
        let mut wallet = snapshot();
        let fii = empty_fii(&wallet);
        let mut pocket = cash();
        pocket.cash_flows.as_mut().unwrap()[1].date = "2026-01-12".into();
        wallet.cash_pockets = vec![pocket];
        let actual = benchmark(&wallet, &fii, &rates());
        assert_eq!(actual.status, "incomplete");
        assert!(actual.cdi.is_none());
        assert_eq!(actual.result.as_deref(), Some("20"));
        assert!(actual
            .issues
            .iter()
            .any(|issue| issue.contains("posterior à data final")));
    }

    #[test]
    fn an_actual_loss_is_preserved_when_cdi_has_a_positive_return() {
        let mut wallet = snapshot();
        let fii = empty_fii(&wallet);
        let mut pocket = cash();
        pocket.balance = "140".into();
        wallet.cash_pockets = vec![pocket];
        let actual = benchmark(&wallet, &fii, &rates());
        assert_eq!(actual.status, "available");
        assert_eq!(actual.result.as_deref(), Some("-10"));
        assert_eq!(actual.return_percent.as_deref(), Some("-5"));
        assert_eq!(actual.difference.as_deref(), Some("-17.620802"));
    }

    #[test]
    fn weekend_balances_use_friday_without_fabricated_daily_rates() {
        let mut wallet = snapshot();
        let fii = empty_fii(&wallet);
        let mut pocket = cash();
        pocket.balance_date = Some("2026-01-11".into());
        wallet.cash_pockets = vec![pocket];
        let mut position = fixed();
        position
            .fixed_income
            .latest_observation
            .as_mut()
            .unwrap()
            .reference_date = "2026-01-10".into();
        wallet.broker_position = Some(position);
        let accounting = calculate_portfolio_performance(&wallet, &fii).unwrap();
        let actual = calculate_portfolio_benchmark(
            &wallet,
            &fii,
            &accounting,
            &rates(),
            date("2026-01-12").unwrap(),
        )
        .unwrap();
        assert_eq!(actual.status, "available");
        assert!(actual.as_of.is_none());
        assert_eq!(
            actual.components[0].cdi.as_ref().unwrap().result,
            "6.10600501"
        );
        assert_eq!(
            actual.components[1].cdi.as_ref().unwrap().result,
            "7.620802"
        );
        assert_eq!(actual.components[0].as_of.as_deref(), Some("2026-01-10"));
        assert_eq!(actual.components[1].as_of.as_deref(), Some("2026-01-11"));
        assert!(actual
            .assumptions
            .iter()
            .any(|assumption| assumption.contains("sem juros extras")));
        // A Friday holiday/publication gap is not silently treated as covered.
        let missing_friday = calculate_portfolio_benchmark(
            &wallet,
            &fii,
            &accounting,
            &rates()[..5],
            date("2026-01-12").unwrap(),
        )
        .unwrap();
        assert_eq!(missing_friday.status, "incomplete");
        assert!(missing_friday.cdi.is_none());
    }

    #[test]
    fn internal_history_gaps_over_seven_days_do_not_count_as_zero_interest() {
        let mut flows = BTreeMap::new();
        add_flow(
            &mut flows,
            date("2026-01-02").unwrap(),
            Decimal::from(100),
            Decimal::ZERO,
        )
        .unwrap();
        let historical = vec![
            DailyCdiRate {
                date: "2026-01-02".into(),
                value: "1".into(),
            },
            DailyCdiRate {
                date: "2026-01-12".into(),
                value: "1".into(),
            },
        ];
        let error = calculate_flow_cdi(
            &flows,
            &historical,
            date("2026-01-12").unwrap(),
            date("2026-01-12").unwrap(),
            Decimal::from(100),
            Decimal::ZERO,
        )
        .unwrap_err();
        assert!(error.to_string().contains("lacuna superior a sete dias"));
    }

    #[test]
    fn empty_reconciled_cash_has_no_invented_start_date_or_percentage() {
        let mut wallet = snapshot();
        let fii = empty_fii(&wallet);
        let mut pocket = cash();
        pocket.balance = "0".into();
        pocket.total_deposits = Some("0".into());
        pocket.total_withdrawals = Some("0".into());
        pocket.cash_flows = Some(vec![]);
        wallet.cash_pockets = vec![pocket];
        let actual = benchmark(&wallet, &fii, &[]);
        assert_eq!(actual.status, "available");
        assert_eq!(actual.invested.as_deref(), Some("0"));
        assert!(actual.start_date.is_none());
        assert!(actual.return_percent.is_none());
        assert_eq!(actual.cdi.unwrap().result, "0");
    }

    #[test]
    fn no_positions_is_explicitly_empty() {
        let wallet = snapshot();
        let fii = empty_fii(&wallet);
        let actual = benchmark(&wallet, &fii, &rates());
        assert_eq!(actual.status, "empty");
        assert!(actual.components.is_empty());
        assert!(actual.start_date.is_none());
        assert!(actual.as_of.is_none());
        assert!(actual.cdi.is_none());
    }
}
