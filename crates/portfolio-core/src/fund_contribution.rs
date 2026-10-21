//! Read-only whole-share purchase scenarios from backend-approved quote bases.
//! Budgets are explicit choices, not recommendations or execution instructions.

use crate::contribution_simulation::{
    simulate_contribution, ContributionClassBase, ContributionSimulationInput,
};
use crate::{CoreError, Result};
use chrono::{Datelike, NaiveDate};
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const MAX_COEFFICIENT: i128 = 79_228_162_514_264_337_593_543_950_335;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FundAllocation {
    pub fund_id: String,
    pub amount: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FundContributionBase {
    pub fund_id: Option<String>,
    pub symbol: String,
    pub name: String,
    pub declared_sector: String,
    pub classification_known: bool,
    pub current_shares: Option<String>,
    /// Only backend-approved BRL quotes belong here; the model supplies budgets.
    pub price: Option<String>,
    pub quote: Value,
    pub quote_status: String,
    pub quote_date: Option<String>,
    pub issues: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundContributionRow {
    pub fund_id: Option<String>,
    pub symbol: String,
    pub name: String,
    pub declared_sector: String,
    pub classification_known: bool,
    pub budget: String,
    pub quote: Value,
    pub quote_status: String,
    pub quote_date: Option<String>,
    pub price: Option<String>,
    pub current_shares: Option<String>,
    pub shares_to_buy: Option<String>,
    pub after_shares: Option<String>,
    pub spent_amount: Option<String>,
    pub remainder: Option<String>,
    pub current_value: Option<String>,
    pub after_value: Option<String>,
    pub before_fii_percent: Option<String>,
    pub after_fii_percent: Option<String>,
    pub before_portfolio_percent: Option<String>,
    pub after_portfolio_percent: Option<String>,
    pub issues: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundPortfolioAllocationRow {
    pub kind: String,
    pub name: String,
    pub current_value: Option<String>,
    pub contribution: Option<String>,
    pub after_value: Option<String>,
    pub before_percent: Option<String>,
    pub after_percent: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundPortfolioAllocation {
    pub status: String,
    pub total_before: Option<String>,
    pub total_after: Option<String>,
    pub unallocated_cash: Option<String>,
    pub as_of: Option<String>,
    pub from_date: Option<String>,
    pub to_date: Option<String>,
    pub valuation_basis: String,
    pub rows: Vec<FundPortfolioAllocationRow>,
    pub issues: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundContributionSimulation {
    pub status: String,
    /// FII budget only, not the contribution's full multi-class budget.
    pub budget: String,
    pub spent_amount: Option<String>,
    pub remainder: Option<String>,
    pub rows: Vec<FundContributionRow>,
    pub fii_basis_complete: bool,
    pub portfolio_allocation: FundPortfolioAllocation,
    pub issues: Vec<String>,
    pub assumptions: Vec<String>,
}

fn invalid(message: &str) -> CoreError {
    CoreError::Validation(message.into())
}

fn text(value: Decimal) -> String {
    value.normalize().to_string()
}

fn decimal(value: &str, scale: usize) -> Option<Decimal> {
    let mut parts = value.split('.');
    let whole = parts.next()?;
    let fraction = parts.next();
    if whole.is_empty()
        || whole.len() > 29
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || parts.next().is_some()
        || fraction.is_some_and(|digits| {
            digits.is_empty()
                || digits.len() > scale
                || !digits.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return None;
    }
    Decimal::from_str_exact(value)
        .ok()
        .map(|value| value.normalize())
}

fn money(value: &str) -> Result<Decimal> {
    decimal(value, 2).ok_or_else(|| invalid("O orçamento de cada fundo precisa ser um valor não negativo em reais, com ponto decimal e no máximo duas casas."))
}

fn exact(mut coefficient: i128, mut scale: u32) -> Result<Decimal> {
    while scale > 0 && coefficient % 10 == 0 {
        coefficient /= 10;
        scale -= 1;
    }
    if coefficient < 0 || coefficient > MAX_COEFFICIENT || scale > 28 {
        return Err(CoreError::ArithmeticOverflow);
    }
    Ok(Decimal::from_parts(
        coefficient as u32,
        (coefficient >> 32) as u32,
        (coefficient >> 64) as u32,
        false,
        scale,
    ))
}

fn align(left: Decimal, right: Decimal) -> Result<(i128, i128, u32)> {
    let scale = left.scale().max(right.scale());
    let left = left
        .mantissa()
        .checked_mul(10_i128.pow(scale - left.scale()))
        .ok_or(CoreError::ArithmeticOverflow)?;
    let right = right
        .mantissa()
        .checked_mul(10_i128.pow(scale - right.scale()))
        .ok_or(CoreError::ArithmeticOverflow)?;
    Ok((left, right, scale))
}

fn add(left: Decimal, right: Decimal) -> Result<Decimal> {
    let (left, right, scale) = align(left, right)?;
    exact(
        left.checked_add(right)
            .ok_or(CoreError::ArithmeticOverflow)?,
        scale,
    )
}

fn subtract(left: Decimal, right: Decimal) -> Result<Decimal> {
    let (left, right, scale) = align(left, right)?;
    exact(
        left.checked_sub(right)
            .ok_or(CoreError::ArithmeticOverflow)?,
        scale,
    )
}

fn multiply(left: Decimal, right: Decimal) -> Result<Decimal> {
    exact(
        left.mantissa()
            .checked_mul(right.mantissa())
            .ok_or(CoreError::ArithmeticOverflow)?,
        left.scale() + right.scale(),
    )
}

fn percent(value: Decimal, total: Decimal) -> Result<Option<String>> {
    if total.is_zero() {
        return Ok(None);
    }
    let value = value
        .checked_div(total)
        .and_then(|value| value.checked_mul(Decimal::ONE_HUNDRED))
        .ok_or(CoreError::ArithmeticOverflow)?;
    Ok(Some(text(value.round_dp_with_strategy(
        2,
        RoundingStrategy::MidpointAwayFromZero,
    ))))
}

fn date_valid(value: &str) -> bool {
    value.len() == 10
        && NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .is_ok_and(|date| date.year() > 0 && date.format("%Y-%m-%d").to_string() == value)
}

fn ticker(value: &str) -> String {
    value.trim().to_uppercase()
}

fn ticker_valid(value: &str) -> bool {
    !value.is_empty() && value.len() <= 32 && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

/// Both inputs have cent precision. Integer division on cent coefficients is
/// exact; decimal division/rounding must never turn an unaffordable share into
/// an affordable one at a boundary such as 0.30 / 0.10.
fn purchase(budget: Decimal, price: Decimal) -> Result<(Decimal, Decimal, Decimal)> {
    let budget_cents = budget
        .mantissa()
        .checked_mul(10_i128.pow(2 - budget.scale()))
        .ok_or(CoreError::ArithmeticOverflow)?;
    let price_cents = price
        .mantissa()
        .checked_mul(10_i128.pow(2 - price.scale()))
        .ok_or(CoreError::ArithmeticOverflow)?;
    let count = exact(
        budget_cents
            .checked_div(price_cents)
            .ok_or(CoreError::ArithmeticOverflow)?,
        0,
    )?;
    let spent = multiply(count, price)?;
    Ok((count, spent, subtract(budget, spent)?))
}

struct Draft {
    row: FundContributionRow,
    budget: Decimal,
    current_shares: Option<Decimal>,
    current_value: Option<Decimal>,
    after_value: Option<Decimal>,
    spent: Option<Decimal>,
    remainder: Option<Decimal>,
}

fn draft(base: &FundContributionBase, budget: Decimal) -> Result<Draft> {
    let mut issues = base.issues.clone();
    let mut price = base
        .price
        .as_deref()
        .and_then(|value| decimal(value, 28))
        .filter(|value| *value > Decimal::ZERO && value.scale() <= 2);
    if base.price.is_some() && price.is_none() {
        issues.push("Cotação indisponível para calcular cotas inteiras: o preço precisa ser positivo em reais e ter precisão de centavos. O preço original não foi arredondado.".into());
    }
    let current_shares = base
        .current_shares
        .as_deref()
        .and_then(|value| decimal(value, 28));
    if base.current_shares.is_some() && current_shares.is_none() {
        issues.push(
            "Quantidade atual inválida ou fora da precisão suportada; a posição não foi presumida."
                .into(),
        );
    }
    let quote_date = base
        .quote_date
        .as_ref()
        .filter(|date| date_valid(date))
        .cloned();
    if base.quote_date.is_some() && quote_date.is_none() {
        price = None;
        issues.push("A data da cotação é inválida; o preço não foi usado na simulação.".into());
    }
    let current_value = match (current_shares, price) {
        (Some(shares), _) if shares.is_zero() => Some(Decimal::ZERO),
        (Some(shares), Some(price)) => Some(multiply(shares, price)?),
        _ => None,
    };
    let (to_buy, spent, remainder) = if budget.is_zero() {
        (
            Some(Decimal::ZERO),
            Some(Decimal::ZERO),
            Some(Decimal::ZERO),
        )
    } else if let Some(price) = price {
        let (count, spent, remainder) = purchase(budget, price)?;
        (Some(count), Some(spent), Some(remainder))
    } else {
        issues.push("Falta uma cotação aprovada para o orçamento deste fundo; cotas, gasto e sobra permanecem desconhecidos, sem supor compra zero.".into());
        (None, None, None)
    };
    let after_shares = current_shares
        .zip(to_buy)
        .map(|(before, count)| add(before, count))
        .transpose()?;
    let after_value = match (after_shares, price) {
        (Some(shares), _) if shares.is_zero() => Some(Decimal::ZERO),
        (Some(shares), Some(price)) => Some(multiply(shares, price)?),
        _ => None,
    };
    Ok(Draft {
        row: FundContributionRow {
            fund_id: base.fund_id.clone(),
            symbol: ticker(&base.symbol),
            name: base.name.clone(),
            declared_sector: base.declared_sector.clone(),
            classification_known: base.classification_known,
            budget: text(budget),
            quote: base.quote.clone(),
            quote_status: base.quote_status.clone(),
            quote_date,
            price: price.map(text),
            current_shares: current_shares.map(text),
            shares_to_buy: to_buy.map(text),
            after_shares: after_shares.map(text),
            spent_amount: spent.map(text),
            remainder: remainder.map(text),
            current_value: current_value.map(text),
            after_value: after_value.map(text),
            before_fii_percent: None,
            after_fii_percent: None,
            before_portfolio_percent: None,
            after_portfolio_percent: None,
            issues,
        },
        budget,
        current_shares,
        current_value,
        after_value,
        spent,
        remainder,
    })
}

/// Whole-share detail for an explicit FII budget. Bases and prices must be
/// resolved by the backend for the selected wallet; no model-supplied quote,
/// quantity, inferred holding or automatic leftover allocation is accepted.
pub fn simulate_fund_contribution(
    input: &ContributionSimulationInput,
    class_bases: &[ContributionClassBase],
    allocations: &[FundAllocation],
    bases: &[FundContributionBase],
) -> Result<FundContributionSimulation> {
    let class_scenario = simulate_contribution(input, class_bases)?;
    let fii_budget = money(&input.allocations.fiis)?;
    let full_budget = money(&input.amount)?;
    let mut symbols = BTreeSet::new();
    let mut identifiers = BTreeMap::new();
    for (index, base) in bases.iter().enumerate() {
        let symbol = ticker(&base.symbol);
        if !ticker_valid(&symbol) || !symbols.insert(symbol) {
            return Err(invalid("Há símbolos inválidos ou duplicados nas bases dos fundos; nenhuma posição foi escolhida para substituir outra."));
        }
        if let Some(id) = &base.fund_id {
            if id.is_empty()
                || id.len() > 100
                || id.trim() != id
                || identifiers.insert(id.as_str(), index).is_some()
            {
                return Err(invalid("Há identificadores de fundo inválidos ou duplicados nas bases da carteira selecionada."));
            }
        }
    }
    let mut budgets = BTreeMap::new();
    let mut allocated = Decimal::ZERO;
    for allocation in allocations {
        if !identifiers.contains_key(allocation.fund_id.as_str()) {
            return Err(invalid("O fundo escolhido não pertence às bases cadastradas desta carteira; informe um fundId da carteira selecionada."));
        }
        let amount = money(&allocation.amount)?;
        if amount.is_zero() {
            return Err(invalid("Cada fundo selecionado precisa ter orçamento positivo. Para orçamento de FIIs zero, informe uma lista vazia."));
        }
        if budgets
            .insert(allocation.fund_id.as_str(), amount)
            .is_some()
        {
            return Err(invalid("Há orçamentos duplicados para o mesmo fundo."));
        }
        allocated = add(allocated, amount)?;
    }
    if allocated != fii_budget {
        return Err(invalid("A soma dos orçamentos por fundo precisa ser exatamente igual ao orçamento reservado aos FIIs; nenhuma sobra será distribuída automaticamente."));
    }
    let mut drafts = bases
        .iter()
        .map(|base| {
            draft(
                base,
                base.fund_id
                    .as_deref()
                    .and_then(|id| budgets.get(id))
                    .copied()
                    .unwrap_or(Decimal::ZERO),
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let plan_known = drafts
        .iter()
        .all(|draft| draft.spent.is_some() && draft.remainder.is_some());
    let spent = plan_known
        .then(|| {
            drafts.iter().try_fold(Decimal::ZERO, |total, draft| {
                add(total, draft.spent.expect("checked spent"))
            })
        })
        .transpose()?;
    let remainder = plan_known
        .then(|| {
            drafts.iter().try_fold(Decimal::ZERO, |total, draft| {
                add(total, draft.remainder.expect("checked remainder"))
            })
        })
        .transpose()?;
    if let (Some(spent), Some(remainder)) = (spent, remainder) {
        if add(spent, remainder)? != fii_budget {
            return Err(CoreError::ArithmeticOverflow);
        }
    }

    let mut issues = class_scenario.issues.clone();
    let known_current = drafts.iter().all(|draft| draft.current_value.is_some());
    let fii_before = known_current
        .then(|| {
            drafts.iter().try_fold(Decimal::ZERO, |total, draft| {
                add(total, draft.current_value.expect("checked current value"))
            })
        })
        .transpose()?;
    let class_fii = class_scenario
        .rows
        .iter()
        .find(|row| row.kind == "fiis")
        .expect("validated three classes");
    let declared_fii = class_fii
        .current_value
        .as_deref()
        .and_then(|value| decimal(value, 28));
    let fii_complete = fii_before.is_some() && fii_before == declared_fii;
    if !fii_complete {
        issues.push("A posição por fundo não está completa ou não confere com o valor de mercado da classe FIIs. Nenhum peso foi normalizado apenas pelas posições conhecidas.".into());
    }
    if !plan_known {
        issues.push("Faltam cotações aprovadas para algum orçamento por fundo. O gasto e a sobra agregados, a composição posterior e os pesos dependentes permanecem indisponíveis.".into());
    }
    let fii_after = if fii_complete {
        fii_before
            .zip(spent)
            .map(|(before, spent)| add(before, spent))
            .transpose()?
    } else {
        None
    };
    if fii_complete {
        let before_total = fii_before.expect("complete FII base");
        for draft in &mut drafts {
            draft.row.before_fii_percent =
                percent(draft.current_value.expect("complete base"), before_total)?;
        }
    }
    if fii_complete && plan_known {
        let after_total = fii_after.expect("complete FII plan");
        let checked_after = drafts.iter().try_fold(Decimal::ZERO, |total, draft| {
            add(
                total,
                draft.after_value.ok_or(CoreError::ArithmeticOverflow)?,
            )
        })?;
        if checked_after != after_total {
            return Err(CoreError::ArithmeticOverflow);
        }
        for draft in &mut drafts {
            draft.row.after_fii_percent =
                percent(draft.after_value.expect("complete plan"), after_total)?;
        }
    }

    let portfolio_before_complete = fii_complete && class_scenario.status == "calculated";
    let portfolio_complete = portfolio_before_complete && plan_known;
    let total_before = if fii_complete {
        class_scenario
            .total_before
            .as_deref()
            .and_then(|value| decimal(value, 28))
    } else {
        None
    };
    let total_after = if portfolio_complete {
        total_before
            .map(|total| add(total, full_budget))
            .transpose()?
    } else {
        None
    };
    let mut portfolio_rows = Vec::with_capacity(4);
    for row in &class_scenario.rows {
        let is_fii = row.kind == "fiis";
        portfolio_rows.push(FundPortfolioAllocationRow {
            kind: row.kind.clone(),
            name: row.name.clone(),
            current_value: row.current_value.clone(),
            contribution: if is_fii {
                spent.map(text)
            } else {
                Some(row.contribution.clone())
            },
            after_value: if is_fii {
                fii_after.map(text)
            } else {
                row.after_value.clone()
            },
            before_percent: None,
            after_percent: None,
        });
    }
    portfolio_rows.push(FundPortfolioAllocationRow {
        kind: "unallocatedCash".into(),
        name: "Sobra não aplicada".into(),
        current_value: Some("0".into()),
        contribution: remainder.map(text),
        after_value: remainder.map(text),
        before_percent: None,
        after_percent: None,
    });
    if portfolio_before_complete {
        let before = total_before.expect("known portfolio");
        let row_before = portfolio_rows
            .iter()
            .try_fold(Decimal::ZERO, |total, row| {
                add(
                    total,
                    decimal(
                        row.current_value.as_deref().expect("known portfolio row"),
                        28,
                    )
                    .ok_or(CoreError::ArithmeticOverflow)?,
                )
            })?;
        if row_before != before {
            return Err(CoreError::ArithmeticOverflow);
        }
        for row in &mut portfolio_rows {
            row.before_percent = percent(
                decimal(row.current_value.as_deref().expect("known row"), 28)
                    .ok_or(CoreError::ArithmeticOverflow)?,
                before,
            )?;
        }
        for draft in &mut drafts {
            draft.row.before_portfolio_percent =
                percent(draft.current_value.expect("known fund"), before)?;
        }
    }
    if portfolio_complete {
        let after = total_after.expect("known portfolio plan");
        let row_after = portfolio_rows
            .iter()
            .try_fold(Decimal::ZERO, |total, row| {
                add(
                    total,
                    decimal(row.after_value.as_deref().expect("known plan row"), 28)
                        .ok_or(CoreError::ArithmeticOverflow)?,
                )
            })?;
        if row_after != after {
            return Err(CoreError::ArithmeticOverflow);
        }
        for row in &mut portfolio_rows {
            row.after_percent = percent(
                decimal(row.after_value.as_deref().expect("known row"), 28)
                    .ok_or(CoreError::ArithmeticOverflow)?,
                after,
            )?;
        }
        for draft in &mut drafts {
            draft.row.after_portfolio_percent =
                percent(draft.after_value.expect("known fund plan"), after)?;
        }
    }

    let mut dates = BTreeSet::new();
    dates.extend(class_scenario.from_date.iter().cloned());
    dates.extend(class_scenario.to_date.iter().cloned());
    let mut undated = class_scenario.valuation_basis == "undated";
    for draft in &drafts {
        if draft
            .current_shares
            .is_none_or(|shares| shares > Decimal::ZERO)
            || draft.budget > Decimal::ZERO
        {
            if let Some(date) = &draft.row.quote_date {
                dates.insert(date.clone());
            } else {
                undated = true;
            }
        }
    }
    let valuation_basis = if undated || dates.is_empty() {
        "undated"
    } else if dates.len() > 1 || class_scenario.valuation_basis == "mixed-dates" {
        "mixed-dates"
    } else {
        "same-date"
    };
    if valuation_basis == "mixed-dates" {
        issues.push("As cotações e os saldos têm datas diferentes; o cenário conserva os últimos valores conhecidos sem atualizar ou interpolar os intervalos.".into());
    } else if valuation_basis == "undated" {
        issues.push("Há base ou cotação sem data suficiente; o cenário não representa uma posição consolidada em uma data presumida.".into());
    }
    issues.extend(drafts.iter().flat_map(|draft| {
        draft
            .row
            .issues
            .iter()
            .map(|issue| format!("{}: {issue}", draft.row.symbol))
    }));
    let from_date = dates.first().cloned();
    let to_date = dates.last().cloned();
    let as_of = (portfolio_complete && valuation_basis == "same-date")
        .then(|| from_date.clone())
        .flatten();
    let status = if portfolio_complete {
        "calculated"
    } else {
        "incomplete"
    }
    .to_string();
    let portfolio_allocation = FundPortfolioAllocation {
        status: status.clone(),
        total_before: total_before.map(text),
        total_after: total_after.map(text),
        unallocated_cash: remainder.map(text),
        as_of,
        from_date,
        to_date,
        valuation_basis: valuation_basis.into(),
        rows: portfolio_rows,
        issues: issues.clone(),
    };
    Ok(FundContributionSimulation {
        status, budget: text(fii_budget), spent_amount: spent.map(text), remainder: remainder.map(text), rows: drafts.into_iter().map(|draft| draft.row).collect(), fii_basis_complete: fii_complete, portfolio_allocation, issues,
        assumptions: vec![
            "Os orçamentos por fundo são escolhas explícitas do solicitante e somam exatamente a reserva para FIIs; esta ferramenta não recomenda uma divisão.".into(),
            "As cotas inteiras são o piso exato de orçamento dividido pela cotação aprovada. A cotação é uma referência datada, não um preço de execução garantido.".into(),
            "Gasto é quantidade inteira multiplicada pela cotação. A sobra permanece livre, sem reinvestimento automático e sem criar caixinha, saldo persistido ou ordem.".into(),
            "Pesos dentro dos FIIs usam o valor de mercado de todos os fundos mantidos, inclusive os que não recebem aporte. Custo histórico e proventos não são acrescentados.".into(),
            "Na carteira inteira, o denominador posterior inclui o aporte total e sua sobra não aplicada. A linha virtual unallocatedCash não é uma caixinha nem dinheiro já investido em FIIs.".into(),
            "Sem base completa e conciliada não se normalizam apenas os fundos conhecidos. Cotações faltantes não significam quantidade, gasto ou sobra zero.".into(),
            "Porcentagens arredondadas a duas casas podem não somar exatamente 100%. Valores de mercado e datas são preservados; não se projetam rendimentos, taxas, impostos ou movimentações.".into(),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contribution_simulation::ContributionAllocations;
    use serde_json::json;

    fn input(amount: &str, fiis: &str, fixed: &str, cash: &str) -> ContributionSimulationInput {
        ContributionSimulationInput {
            amount: amount.into(),
            allocations: ContributionAllocations {
                fiis: fiis.into(),
                fixed_income: fixed.into(),
                cash_pockets: cash.into(),
            },
        }
    }
    fn classes(
        fiis: Option<&str>,
        fixed: Option<&str>,
        cash: Option<&str>,
    ) -> Vec<ContributionClassBase> {
        [
            ("fiis", fiis),
            ("fixedIncome", fixed),
            ("cashPockets", cash),
        ]
        .into_iter()
        .map(|(kind, value)| ContributionClassBase {
            kind: kind.into(),
            name: kind.into(),
            current_value: value.map(str::to_owned),
            dates: value
                .filter(|value| *value != "0")
                .map(|_| vec!["2026-10-20".into()])
                .unwrap_or_default(),
            issues: vec![],
        })
        .collect()
    }
    fn base(id: &str, shares: Option<&str>, price: Option<&str>) -> FundContributionBase {
        FundContributionBase {
            fund_id: Some(id.into()),
            symbol: id.to_uppercase(),
            name: format!("Fund {id}"),
            declared_sector: "Fundo Imobiliário".into(),
            classification_known: false,
            current_shares: shares.map(str::to_owned),
            price: price.map(str::to_owned),
            quote: json!({"symbol":id.to_uppercase(),"status":"cached","error":"original source warning","observation":{"symbol":id.to_uppercase(),"currency":"BRL","price":price,"quotedAt":"2026-10-20T18:00:00Z","source":"synthetic"}}),
            quote_status: "cached".into(),
            quote_date: Some("2026-10-20".into()),
            issues: vec![],
        }
    }
    fn allocations(values: &[(&str, &str)]) -> Vec<FundAllocation> {
        values
            .iter()
            .map(|(id, amount)| FundAllocation {
                fund_id: (*id).into(),
                amount: (*amount).into(),
            })
            .collect()
    }

    #[test]
    fn whole_shares_spend_only_available_budget_and_keep_free_remainder_in_the_wallet_denominator()
    {
        let bases = vec![
            base("ONE11", Some("10"), Some("30")),
            base("TWO11", Some("5"), Some("60")),
        ];
        let before = serde_json::to_value(&bases).unwrap();
        let result = simulate_fund_contribution(
            &input("300", "200", "50", "50"),
            &classes(Some("600"), Some("400"), Some("0")),
            &allocations(&[("ONE11", "100"), ("TWO11", "100")]),
            &bases,
        )
        .unwrap();
        assert_eq!(result.status, "calculated");
        assert_eq!(
            (
                result.budget.as_str(),
                result.spent_amount.as_deref(),
                result.remainder.as_deref()
            ),
            ("200", Some("150"), Some("50"))
        );
        assert_eq!(
            (
                result.rows[0].shares_to_buy.as_deref(),
                result.rows[0].spent_amount.as_deref(),
                result.rows[0].remainder.as_deref(),
                result.rows[0].after_shares.as_deref()
            ),
            (Some("3"), Some("90"), Some("10"), Some("13"))
        );
        assert_eq!(
            (
                result.rows[1].shares_to_buy.as_deref(),
                result.rows[1].spent_amount.as_deref(),
                result.rows[1].remainder.as_deref()
            ),
            (Some("1"), Some("60"), Some("40"))
        );
        assert_eq!(result.rows[0].before_fii_percent.as_deref(), Some("50"));
        assert_eq!(result.rows[0].after_fii_percent.as_deref(), Some("52"));
        assert_eq!(result.rows[1].after_fii_percent.as_deref(), Some("48"));
        assert_eq!(
            result.rows[0].after_portfolio_percent.as_deref(),
            Some("30")
        );
        let whole = &result.portfolio_allocation;
        assert_eq!(
            (whole.total_before.as_deref(), whole.total_after.as_deref()),
            (Some("1000"), Some("1300"))
        );
        assert_eq!(
            whole
                .rows
                .iter()
                .map(|row| row.kind.as_str())
                .collect::<Vec<_>>(),
            ["fiis", "fixedIncome", "cashPockets", "unallocatedCash"]
        );
        assert_eq!(
            whole
                .rows
                .iter()
                .map(|row| row.after_value.as_deref())
                .collect::<Vec<_>>(),
            [Some("750"), Some("450"), Some("50"), Some("50")]
        );
        assert_eq!(whole.rows[0].contribution.as_deref(), Some("150"));
        assert_eq!(whole.rows[2].contribution.as_deref(), Some("50"));
        assert_eq!(whole.rows[3].contribution.as_deref(), Some("50"));
        assert_eq!(
            whole
                .rows
                .iter()
                .map(|row| decimal(row.after_value.as_deref().unwrap(), 28).unwrap())
                .fold(Decimal::ZERO, |total, value| add(total, value).unwrap()),
            Decimal::from(1300)
        );
        assert_eq!(result.rows[0].quote, bases[0].quote);
        assert_eq!(serde_json::to_value(&bases).unwrap(), before);
    }

    #[test]
    fn integer_cent_division_does_not_use_floating_point_or_round_up_unaffordable_cotas() {
        for (budget, price, count, spent, remainder) in [
            ("0.30", "0.10", "3", "0.3", "0"),
            ("0.29", "0.10", "2", "0.2", "0.09"),
            ("100", "9.46", "10", "94.6", "5.4"),
        ] {
            let result = simulate_fund_contribution(
                &input(budget, budget, "0", "0"),
                &classes(Some("0"), Some("0"), Some("0")),
                &allocations(&[("ONE11", budget)]),
                &[base("ONE11", Some("0"), Some(price))],
            )
            .unwrap();
            assert_eq!(result.rows[0].shares_to_buy.as_deref(), Some(count));
            assert_eq!(result.spent_amount.as_deref(), Some(spent));
            assert_eq!(result.remainder.as_deref(), Some(remainder));
        }
    }

    #[test]
    fn budget_below_one_share_is_a_known_zero_purchase_with_the_entire_amount_left_free() {
        let result = simulate_fund_contribution(
            &input("29.99", "29.99", "0", "0"),
            &classes(Some("0"), Some("0"), Some("0")),
            &allocations(&[("ONE11", "29.99")]),
            &[base("ONE11", Some("0"), Some("30"))],
        )
        .unwrap();
        assert_eq!(result.rows[0].shares_to_buy.as_deref(), Some("0"));
        assert_eq!(result.spent_amount.as_deref(), Some("0"));
        assert_eq!(result.remainder.as_deref(), Some("29.99"));
        assert!(
            result.rows[0].before_fii_percent.is_none()
                && result.rows[0].after_fii_percent.is_none()
        );
        assert_eq!(result.rows[0].after_portfolio_percent.as_deref(), Some("0"));
        assert_eq!(
            result.portfolio_allocation.rows[3].after_percent.as_deref(),
            Some("100")
        );
    }

    #[test]
    fn missing_allocated_quote_keeps_quantities_cost_and_leftover_unknown_and_blocks_after_weights()
    {
        let result = simulate_fund_contribution(
            &input("100", "100", "0", "0"),
            &classes(Some("0"), Some("100"), Some("0")),
            &allocations(&[("ONE11", "100")]),
            &[base("ONE11", Some("0"), None)],
        )
        .unwrap();
        assert_eq!(result.status, "incomplete");
        let row = &result.rows[0];
        assert!(
            row.shares_to_buy.is_none()
                && row.spent_amount.is_none()
                && row.remainder.is_none()
                && row.after_value.is_none()
        );
        assert_eq!(row.current_value.as_deref(), Some("0"));
        assert!(result.spent_amount.is_none() && result.remainder.is_none());
        assert_eq!(
            result.portfolio_allocation.total_before.as_deref(),
            Some("100")
        );
        assert!(result.portfolio_allocation.total_after.is_none());
        assert!(result
            .portfolio_allocation
            .rows
            .iter()
            .all(|row| row.after_percent.is_none()));
        assert_eq!(
            result.portfolio_allocation.rows[1]
                .before_percent
                .as_deref(),
            Some("100")
        );
        assert_eq!(
            result.portfolio_allocation.rows[1].after_value.as_deref(),
            Some("100")
        );
        assert!(result.portfolio_allocation.rows[3].after_value.is_none());
    }

    #[test]
    fn missing_price_for_a_new_fund_preserves_complete_before_concentration_without_guessing_after()
    {
        let result = simulate_fund_contribution(
            &input("100", "100", "0", "0"),
            &classes(Some("100"), Some("100"), Some("0")),
            &allocations(&[("TWO11", "100")]),
            &[
                base("ONE11", Some("10"), Some("10")),
                base("TWO11", Some("0"), None),
            ],
        )
        .unwrap();
        assert_eq!(result.status, "incomplete");
        assert!(result.fii_basis_complete);
        assert_eq!(result.rows[0].before_fii_percent.as_deref(), Some("100"));
        assert_eq!(result.rows[1].before_fii_percent.as_deref(), Some("0"));
        assert_eq!(
            result.rows[0].before_portfolio_percent.as_deref(),
            Some("50")
        );
        assert_eq!(
            result.rows[1].before_portfolio_percent.as_deref(),
            Some("0")
        );
        assert!(result
            .rows
            .iter()
            .all(|row| row.after_fii_percent.is_none() && row.after_portfolio_percent.is_none()));
        assert_eq!(
            result.portfolio_allocation.total_before.as_deref(),
            Some("200")
        );
        assert!(result.portfolio_allocation.total_after.is_none());
        assert_eq!(
            result
                .portfolio_allocation
                .rows
                .iter()
                .map(|row| row.before_percent.as_deref())
                .collect::<Vec<_>>(),
            [Some("50"), Some("50"), Some("0"), Some("0")]
        );
        assert!(result
            .portfolio_allocation
            .rows
            .iter()
            .all(|row| row.after_percent.is_none()));
        assert_eq!(result.rows[0].after_value.as_deref(), Some("100"));
        assert!(result.rows[1].after_value.is_none());
    }

    #[test]
    fn unknown_held_fund_is_not_omitted_or_normalized_away_even_without_an_allocated_budget() {
        let mut unknown = base("BROK11", None, Some("10"));
        unknown.fund_id = None;
        let result = simulate_fund_contribution(
            &input("10", "10", "0", "0"),
            &classes(Some("100"), Some("0"), Some("0")),
            &allocations(&[("ONE11", "10")]),
            &[base("ONE11", Some("10"), Some("10")), unknown],
        )
        .unwrap();
        assert_eq!(result.rows.len(), 2);
        assert!(!result.fii_basis_complete);
        assert_eq!(result.spent_amount.as_deref(), Some("10"));
        assert_eq!(result.rows[1].budget, "0");
        assert_eq!(result.rows[1].shares_to_buy.as_deref(), Some("0"));
        assert!(result.rows[1].current_value.is_none());
        assert!(result
            .rows
            .iter()
            .all(|row| row.before_fii_percent.is_none()
                && row.after_fii_percent.is_none()
                && row.before_portfolio_percent.is_none()
                && row.after_portfolio_percent.is_none()));
        assert!(
            result.portfolio_allocation.total_before.is_none()
                && result.portfolio_allocation.total_after.is_none()
        );
    }

    #[test]
    fn maintained_fund_without_new_budget_remains_in_before_and_after_concentration() {
        let sources = vec![
            base("ONE11", Some("10"), Some("10")),
            base("TWO11", Some("10"), Some("10.0000")),
        ];
        let result = simulate_fund_contribution(
            &input("100", "100", "0", "0"),
            &classes(Some("200"), Some("0"), Some("0")),
            &allocations(&[("ONE11", "100")]),
            &sources,
        )
        .unwrap();
        assert_eq!(result.status, "calculated");
        assert_eq!(result.rows[1].budget, "0");
        assert_eq!(result.rows[1].shares_to_buy.as_deref(), Some("0"));
        assert_eq!(result.rows[1].after_shares.as_deref(), Some("10"));
        assert_eq!(result.rows[1].current_value.as_deref(), Some("100"));
        assert_eq!(result.rows[1].after_value.as_deref(), Some("100"));
        assert_eq!(result.rows[1].before_fii_percent.as_deref(), Some("50"));
        assert_eq!(result.rows[1].after_fii_percent.as_deref(), Some("33.33"));
        assert_eq!(result.rows[0].after_fii_percent.as_deref(), Some("66.67"));
        assert_eq!(result.rows[1].price.as_deref(), Some("10"));
        assert_eq!(result.rows[1].quote, sources[1].quote);
    }

    #[test]
    fn incompatible_class_value_does_not_replace_the_class_with_a_known_subset() {
        let result = simulate_fund_contribution(
            &input("10", "10", "0", "0"),
            &classes(Some("100"), Some("0"), Some("0")),
            &allocations(&[("ONE11", "10")]),
            &[base("ONE11", Some("9"), Some("10"))],
        )
        .unwrap();
        assert!(!result.fii_basis_complete);
        assert_eq!(result.rows[0].current_value.as_deref(), Some("90"));
        assert_eq!(
            result.portfolio_allocation.rows[0].current_value.as_deref(),
            Some("100")
        );
        assert!(result.portfolio_allocation.rows[0].after_value.is_none());
        assert!(result.portfolio_allocation.total_after.is_none());
    }

    #[test]
    fn missing_other_asset_class_blocks_wallet_weights_but_not_complete_fii_concentration() {
        let result = simulate_fund_contribution(
            &input("10", "10", "0", "0"),
            &classes(Some("100"), None, Some("0")),
            &allocations(&[("ONE11", "10")]),
            &[base("ONE11", Some("10"), Some("10"))],
        )
        .unwrap();
        assert_eq!(result.status, "incomplete");
        assert!(result.fii_basis_complete);
        assert_eq!(result.rows[0].before_fii_percent.as_deref(), Some("100"));
        assert_eq!(result.rows[0].after_fii_percent.as_deref(), Some("100"));
        assert!(
            result.rows[0].before_portfolio_percent.is_none()
                && result.rows[0].after_portfolio_percent.is_none()
        );
        assert!(
            result.portfolio_allocation.total_before.is_none()
                && result.portfolio_allocation.total_after.is_none()
        );
    }

    #[test]
    fn unallocated_known_empty_fund_does_not_need_a_quote_or_receive_a_fake_purchase() {
        let result = simulate_fund_contribution(
            &input("100", "0", "100", "0"),
            &classes(Some("0"), Some("0"), Some("0")),
            &[],
            &[base("ONE11", Some("0"), None)],
        )
        .unwrap();
        assert_eq!(result.status, "calculated");
        assert_eq!(result.rows[0].shares_to_buy.as_deref(), Some("0"));
        assert_eq!(result.rows[0].spent_amount.as_deref(), Some("0"));
        assert_eq!(result.rows[0].remainder.as_deref(), Some("0"));
        assert_eq!(result.rows[0].after_value.as_deref(), Some("0"));
        assert_eq!(
            result.portfolio_allocation.total_after.as_deref(),
            Some("100")
        );
    }

    #[test]
    fn malformed_zero_or_subcent_quote_stays_unavailable_without_rounding_or_discarding_original_quote(
    ) {
        for price in ["0", "-1", "NaN", "1.001", "1e2"] {
            let source = base("ONE11", Some("0"), Some(price));
            let result = simulate_fund_contribution(
                &input("100", "100", "0", "0"),
                &classes(Some("0"), Some("0"), Some("0")),
                &allocations(&[("ONE11", "100")]),
                &[source.clone()],
            )
            .unwrap();
            assert_eq!(result.status, "incomplete", "{price}");
            assert!(result.rows[0].price.is_none() && result.rows[0].shares_to_buy.is_none());
            assert_eq!(result.rows[0].quote, source.quote);
            assert!(!result.rows[0].issues.is_empty());
        }
    }

    #[test]
    fn exact_allocation_scope_and_duplicate_validation_reject_invalid_requests() {
        let request = input("100", "100", "0", "0");
        let class = classes(Some("0"), Some("0"), Some("0"));
        let source = base("ONE11", Some("0"), Some("10"));
        for invalid in [
            allocations(&[("FOREIGN11", "100")]),
            allocations(&[("ONE11", "99.99")]),
            allocations(&[("ONE11", "100.01")]),
            allocations(&[("ONE11", "50"), ("ONE11", "50")]),
            allocations(&[("ONE11", "0")]),
            allocations(&[("ONE11", "100.001")]),
            allocations(&[("ONE11", "-100")]),
        ] {
            assert!(matches!(
                simulate_fund_contribution(&request, &class, &invalid, &[source.clone()]),
                Err(CoreError::Validation(_))
            ));
        }
        let mut duplicate_symbol = source.clone();
        duplicate_symbol.fund_id = Some("different-id".into());
        duplicate_symbol.symbol = " one11 ".into();
        assert!(simulate_fund_contribution(
            &request,
            &class,
            &allocations(&[("ONE11", "100")]),
            &[source.clone(), duplicate_symbol]
        )
        .is_err());
        let mut duplicate_id = source.clone();
        duplicate_id.symbol = "TWO11".into();
        assert!(simulate_fund_contribution(
            &request,
            &class,
            &allocations(&[("ONE11", "100")]),
            &[source, duplicate_id]
        )
        .is_err());
        assert!(serde_json::from_value::<FundAllocation>(
            json!({"fundId":"ONE11","amount":"100","price":"1"})
        )
        .is_err());
    }

    #[test]
    fn quote_and_balance_dates_remain_mixed_or_undated_without_inventing_a_common_day() {
        let mut source = base("ONE11", Some("10"), Some("10"));
        source.quote_date = Some("2026-10-21".into());
        let result = simulate_fund_contribution(
            &input("10", "10", "0", "0"),
            &classes(Some("100"), Some("50"), Some("0")),
            &allocations(&[("ONE11", "10")]),
            &[source.clone()],
        )
        .unwrap();
        assert_eq!(result.portfolio_allocation.valuation_basis, "mixed-dates");
        assert!(result.portfolio_allocation.as_of.is_none());
        assert_eq!(
            (
                result.portfolio_allocation.from_date.as_deref(),
                result.portfolio_allocation.to_date.as_deref()
            ),
            (Some("2026-10-20"), Some("2026-10-21"))
        );
        source.quote_date = None;
        let result = simulate_fund_contribution(
            &input("10", "10", "0", "0"),
            &classes(Some("100"), Some("50"), Some("0")),
            &allocations(&[("ONE11", "10")]),
            &[source.clone()],
        )
        .unwrap();
        assert_eq!(result.portfolio_allocation.valuation_basis, "undated");
        assert!(result.portfolio_allocation.as_of.is_none());
        source.quote_date = Some("2026-02-30".into());
        let result = simulate_fund_contribution(
            &input("10", "10", "0", "0"),
            &classes(Some("100"), Some("50"), Some("0")),
            &allocations(&[("ONE11", "10")]),
            &[source],
        )
        .unwrap();
        assert_eq!(result.status, "incomplete");
        assert!(result.rows[0].price.is_none());
    }

    #[test]
    fn checked_overflow_never_returns_a_rounded_share_count_or_saturated_position() {
        let result = simulate_fund_contribution(
            &input("1", "1", "0", "0"),
            &classes(Some("0"), Some("0"), Some("0")),
            &allocations(&[("ONE11", "1")]),
            &[base(
                "ONE11",
                Some("79228162514264337593543950335"),
                Some("2"),
            )],
        );
        assert!(matches!(result, Err(CoreError::ArithmeticOverflow)));
        let result = simulate_fund_contribution(
            &input(
                "79228162514264337593543950335",
                "79228162514264337593543950335",
                "0",
                "0",
            ),
            &classes(Some("0"), Some("0"), Some("0")),
            &allocations(&[("ONE11", "79228162514264337593543950335")]),
            &[base("ONE11", Some("0"), Some("0.01"))],
        );
        assert!(matches!(result, Err(CoreError::ArithmeticOverflow)));
    }
}
