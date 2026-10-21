//! Pure, read-only comparison of saved quotes with dated ledger purchase costs.
//! This module never fetches data, changes holdings, ranks funds or gives advice.

use crate::quotes::{QuoteItem, QuotePanel};
use chrono::{Datelike, NaiveDate};
use ivestfii_core::{FundView, PortfolioSnapshot};
use rust_decimal::Decimal;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

const SCOPE: &str = "selected-wallet-ledger-purchases-only; excludes post-fixed income, cash pockets and account cash";
const BASIS: &str = "Eligible recorded purchase quantity times recorded unit cost, divided by purchased shares, through asOf. Future purchases and income are excluded. Legacy purchases without a day retain their recorded month; no day is invented.";
const SOURCE: &str = "Local selected-wallet ledger and original saved public quote observations; no network request or financial write.";
const MAX_COEFFICIENT: i128 = 79_228_162_514_264_337_593_543_950_335;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseAnalysis {
    pub as_of: String,
    pub threshold_percent: String,
    pub scope: String,
    pub basis: String,
    pub source: String,
    pub basis_complete: bool,
    pub missing_data: bool,
    pub overflow: bool,
    pub rows: Vec<PurchaseAnalysisRow>,
    pub allocation: PurchaseAllocation,
    pub issues: Vec<String>,
    pub quote_errors: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseAnalysisRow {
    pub fund_id: Option<String>,
    pub symbol: String,
    pub name: Option<String>,
    pub declared_sector: Option<String>,
    /// Specific user-declared label, not an independently verified exposure.
    pub classification_known: bool,
    pub shares: Option<String>,
    pub invested: Option<String>,
    pub average_price: Option<String>,
    pub invested_share_percent: Option<String>,
    /// Preserves the original observation, status, source dates and error.
    pub quote: Option<QuoteItem>,
    /// No arbitrary quote is selected when duplicate symbol rows are supplied.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub quote_candidates: Vec<QuoteItem>,
    pub difference_percent: Option<String>,
    pub price_band: String,
    pub missing_data: bool,
    pub overflow: bool,
    pub issues: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseAllocation {
    pub scope: String,
    pub basis: String,
    pub basis_complete: bool,
    pub total_invested: Option<String>,
    pub by_fund: Vec<FundPurchaseAllocation>,
    pub by_declared_sector: Vec<SectorPurchaseAllocation>,
    pub issues: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FundPurchaseAllocation {
    pub fund_id: String,
    pub symbol: String,
    pub invested: Option<String>,
    pub share_percent: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SectorPurchaseAllocation {
    pub declared_sector: String,
    pub classification_known: bool,
    pub invested: Option<String>,
    pub share_percent: Option<String>,
    pub symbols: Vec<String>,
}

#[derive(Clone, Copy, Debug)]
enum CalculationIssue {
    Missing(&'static str),
    Overflow,
}

impl CalculationIssue {
    fn text(self) -> &'static str {
        match self {
            Self::Missing(message) => message,
            Self::Overflow => "O cálculo excedeu a precisão decimal suportada; nenhuma classificação foi presumida.",
        }
    }
}

type Calculation<T> = Result<T, CalculationIssue>;

fn symbol(value: &str) -> String {
    value.trim().to_uppercase()
}

fn valid_symbol(value: &str) -> bool {
    !value.is_empty() && value.len() <= 32 && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

/// Accept the same unsigned decimal grammar and coefficient bounds as the UI.
fn decimal(value: &str) -> Option<Decimal> {
    let mut parts = value.split('.');
    let whole = parts.next()?;
    let fraction = parts.next();
    if whole.is_empty()
        || whole.len() > 29
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || parts.next().is_some()
        || fraction.is_some_and(|digits| {
            digits.is_empty()
                || digits.len() > 28
                || !digits.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return None;
    }
    let coefficient = format!("{whole}{}", fraction.unwrap_or_default())
        .parse::<i128>()
        .ok()?;
    if coefficient > MAX_COEFFICIENT {
        return None;
    }
    Some(
        Decimal::from_parts(
            coefficient as u32,
            (coefficient >> 32) as u32,
            (coefficient >> 64) as u32,
            false,
            fraction.map_or(0, |digits| digits.len() as u32),
        )
        .normalize(),
    )
}

fn text(value: Decimal) -> String {
    value.normalize().to_string()
}

fn exact(coefficient: i128, mut scale: u32) -> Calculation<Decimal> {
    let mut coefficient = coefficient;
    while scale > 0 && coefficient % 10 == 0 {
        coefficient /= 10;
        scale -= 1;
    }
    if scale > 28 || coefficient.unsigned_abs() > MAX_COEFFICIENT as u128 {
        return Err(CalculationIssue::Overflow);
    }
    let unsigned = coefficient.unsigned_abs();
    Ok(Decimal::from_parts(
        unsigned as u32,
        (unsigned >> 32) as u32,
        (unsigned >> 64) as u32,
        coefficient < 0,
        scale,
    ))
}

fn align(left: Decimal, right: Decimal) -> Calculation<(i128, i128, u32)> {
    let scale = left.scale().max(right.scale());
    let left = left
        .mantissa()
        .checked_mul(10_i128.pow(scale - left.scale()))
        .ok_or(CalculationIssue::Overflow)?;
    let right = right
        .mantissa()
        .checked_mul(10_i128.pow(scale - right.scale()))
        .ok_or(CalculationIssue::Overflow)?;
    Ok((left, right, scale))
}

fn add(left: Decimal, right: Decimal) -> Calculation<Decimal> {
    let (left, right, scale) = align(left, right)?;
    exact(
        left.checked_add(right).ok_or(CalculationIssue::Overflow)?,
        scale,
    )
}

fn multiply(left: Decimal, right: Decimal) -> Calculation<Decimal> {
    exact(
        left.mantissa()
            .checked_mul(right.mantissa())
            .ok_or(CalculationIssue::Overflow)?,
        left.scale() + right.scale(),
    )
}

/// Exact integer rounding of a ratio to a two-decimal percentage. The rounded
/// display value is never used to choose a price band or an allocation base.
fn ratio_percent(numerator: Decimal, denominator: Decimal) -> Calculation<Decimal> {
    let (numerator, denominator, _) = align(numerator, denominator)?;
    if denominator <= 0 {
        return Err(CalculationIssue::Missing(
            "Não há custo positivo para calcular a porcentagem.",
        ));
    }
    let scaled = numerator
        .checked_abs()
        .and_then(|value| value.checked_mul(10_000))
        .and_then(|value| value.checked_add(denominator / 2))
        .ok_or(CalculationIssue::Overflow)?;
    let rounded = scaled / denominator;
    exact(if numerator < 0 { -rounded } else { rounded }, 2)
}

fn valid_month(value: &str) -> bool {
    value.len() == 7
        && NaiveDate::parse_from_str(&format!("{value}-01"), "%Y-%m-%d")
            .is_ok_and(|date| date.year() > 0 && date.format("%Y-%m").to_string() == value)
}

fn valid_date(value: &str) -> Option<NaiveDate> {
    let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()?;
    (value.len() == 10 && parsed.year() > 0 && parsed.format("%Y-%m-%d").to_string() == value)
        .then_some(parsed)
}

fn purchase_basis(fund: &FundView, today: NaiveDate) -> Calculation<(Decimal, Decimal)> {
    let cutoff = today.format("%Y-%m").to_string();
    let mut shares = Decimal::ZERO;
    let mut cost = Decimal::ZERO;
    for entry in &fund.entries {
        if !valid_month(&entry.month) {
            return Err(CalculationIssue::Missing(
                "Confira o mês das compras para calcular o preço médio.",
            ));
        }
        if entry.month > cutoff {
            continue;
        }
        let quantity = decimal(&entry.cotas_added).ok_or(CalculationIssue::Missing(
            "Confira a quantidade das compras para calcular o preço médio.",
        ))?;
        if quantity.is_zero() {
            continue;
        }
        if let Some(day) = entry.purchase_date.as_deref().filter(|day| !day.is_empty()) {
            let parsed = valid_date(day)
                .filter(|_| day.starts_with(&format!("{}-", entry.month)))
                .ok_or(CalculationIssue::Missing(
                    "Confira a data das compras para calcular o preço médio.",
                ))?;
            if parsed > today {
                continue;
            }
        }
        let price = decimal(&entry.price_per_cota)
            .filter(|price| *price > Decimal::ZERO)
            .ok_or(CalculationIssue::Missing(
                "Há compras sem preço válido. Complete esses valores para comparar a cotação.",
            ))?;
        shares = add(shares, quantity)?;
        cost = add(cost, multiply(quantity, price)?)?;
    }
    Ok((shares, cost))
}

fn classification_known(sector: &str) -> bool {
    !matches!(
        sector.trim().to_lowercase().as_str(),
        "" | "-"
            | "n/a"
            | "outro"
            | "outros"
            | "não informado"
            | "nao informado"
            | "fii"
            | "fiis"
            | "fiagro"
            | "fundo imobiliário"
            | "fundo imobiliario"
            | "fundos imobiliários"
            | "fundos imobiliarios"
            | "fundo de investimento imobiliário"
            | "fundo de investimento imobiliario"
            | "fundo de investimento"
            | "fundos de investimento"
    )
}

fn note(row: &mut PurchaseAnalysisRow, issue: CalculationIssue) {
    row.missing_data |= matches!(issue, CalculationIssue::Missing(_));
    row.overflow |= matches!(issue, CalculationIssue::Overflow);
    let message = issue.text().to_string();
    if !row.issues.contains(&message) {
        row.issues.push(message);
    }
}

fn build_row(
    fund: Option<&FundView>,
    ticker: String,
    quotes: &[&QuoteItem],
    unique_fund: bool,
    today: NaiveDate,
) -> PurchaseAnalysisRow {
    let mut row = PurchaseAnalysisRow {
        fund_id: fund.map(|fund| fund.id.clone()),
        symbol: ticker,
        name: fund.map(|fund| fund.name.clone()),
        declared_sector: fund.map(|fund| fund.sector.clone()),
        classification_known: fund.is_some_and(|fund| classification_known(&fund.sector)),
        shares: None,
        invested: None,
        average_price: None,
        invested_share_percent: None,
        quote: (quotes.len() == 1).then(|| (*quotes[0]).clone()),
        quote_candidates: if quotes.len() > 1 {
            quotes.iter().map(|item| (*item).clone()).collect()
        } else {
            vec![]
        },
        difference_percent: None,
        price_band: "unknown".into(),
        missing_data: false,
        overflow: false,
        issues: vec![],
    };
    if !valid_symbol(&row.symbol) || !unique_fund {
        note(
            &mut row,
            CalculationIssue::Missing(
                "Não há uma base única de compras para este fundo na carteira selecionada.",
            ),
        );
        return row;
    }
    let Some(fund) = fund else {
        note(&mut row, CalculationIssue::Missing("Este ativo não possui compras registradas nesta carteira; não há preço médio nem peso de compras."));
        return row;
    };
    let (shares, cost) = match purchase_basis(fund, today) {
        Ok(basis) => basis,
        Err(issue) => {
            note(&mut row, issue);
            return row;
        }
    };
    row.shares = Some(text(shares));
    row.invested = Some(text(cost));
    if shares.is_zero() {
        note(
            &mut row,
            CalculationIssue::Missing(
                "Registre as compras deste fundo nesta carteira para comparar com seu preço médio.",
            ),
        );
        return row;
    }
    let Some(average) = cost.checked_div(shares) else {
        note(&mut row, CalculationIssue::Overflow);
        return row;
    };
    row.average_price = Some(text(average));
    if quotes.len() > 1 {
        note(
            &mut row,
            CalculationIssue::Missing(
                "Há cotações duplicadas para este ativo; nenhuma foi escolhida para a comparação.",
            ),
        );
        return row;
    }
    let quote_price = row
        .quote
        .as_ref()
        .and_then(|item| item.observation.as_ref())
        .filter(|observation| {
            observation.currency == "BRL" && symbol(&observation.symbol) == row.symbol
        })
        .and_then(|observation| decimal(&observation.price))
        .filter(|price| *price > Decimal::ZERO);
    let Some(price) = quote_price else {
        note(
            &mut row,
            CalculationIssue::Missing(
                "A comparação precisa de uma cotação positiva em reais para o mesmo ativo.",
            ),
        );
        return row;
    };
    let compared = (|| {
        let quoted_cost = multiply(price, shares)?;
        let comparison = multiply(quoted_cost, Decimal::ONE_HUNDRED)?;
        let lower = multiply(cost, Decimal::from(99))?;
        let upper = multiply(cost, Decimal::from(101))?;
        let band = if comparison < lower {
            "below"
        } else if comparison > upper {
            "above"
        } else {
            "near"
        };
        let (market, cost, scale) = align(quoted_cost, cost)?;
        let difference = exact(
            market.checked_sub(cost).ok_or(CalculationIssue::Overflow)?,
            scale,
        )?;
        Ok::<_, CalculationIssue>((band, ratio_percent(difference, exact(cost, scale)?)?))
    })();
    match compared {
        Ok((band, difference)) => {
            row.price_band = band.into();
            row.difference_percent = Some(text(difference));
        }
        Err(issue) => note(&mut row, issue),
    }
    row
}

/// Produces the same relative-price bands as Mercado from immutable local
/// inputs. `today` is the caller's BRT calendar date, never a guessed quote day.
pub fn build_purchase_analysis(
    snapshot: &PortfolioSnapshot,
    quotes: &QuotePanel,
    today: NaiveDate,
) -> PurchaseAnalysis {
    let mut quote_map: BTreeMap<String, Vec<&QuoteItem>> = BTreeMap::new();
    for quote in &quotes.items {
        quote_map
            .entry(symbol(&quote.symbol))
            .or_default()
            .push(quote);
    }
    let mut counts = BTreeMap::<String, usize>::new();
    for fund in &snapshot.funds {
        *counts.entry(symbol(&fund.ticker)).or_default() += 1;
    }
    let mut rows = vec![];
    for fund in &snapshot.funds {
        let ticker = symbol(&fund.ticker);
        rows.push(build_row(
            Some(fund),
            ticker.clone(),
            quote_map
                .get(&ticker)
                .map(Vec::as_slice)
                .unwrap_or_default(),
            counts[&ticker] == 1,
            today,
        ));
    }
    for (ticker, items) in &quote_map {
        if !counts.contains_key(ticker) {
            rows.push(build_row(None, ticker.clone(), items, true, today));
        }
    }

    let mut allocation_issues = vec![];
    let mut complete = rows
        .iter()
        .filter(|row| row.fund_id.is_some())
        .all(|row| row.shares.is_some() && row.invested.is_some());
    // A broker-only holding is visible but has no ledger cost. Suppress weights
    // rather than presenting a partial denominator as the complete FII mix.
    if snapshot.broker_position.as_ref().is_some_and(|position| {
        position.listed_funds.holdings.iter().any(|holding| {
            decimal(&holding.shares).is_none_or(|shares| shares > Decimal::ZERO)
                && !rows.iter().any(|row| {
                    row.fund_id.is_some()
                        && row.symbol == symbol(&holding.ticker)
                        && row
                            .shares
                            .as_deref()
                            .and_then(decimal)
                            .is_some_and(|shares| shares > Decimal::ZERO)
                        && row
                            .invested
                            .as_deref()
                            .and_then(decimal)
                            .is_some_and(|cost| cost > Decimal::ZERO)
                })
        })
    }) {
        complete = false;
        allocation_issues.push("Há ativos na posição importada sem histórico de compras; os pesos não são calculados com uma base parcial.".into());
    }
    let mut total = Decimal::ZERO;
    let mut allocation_overflow = false;
    for row in rows.iter().filter(|row| row.fund_id.is_some()) {
        if let Some(cost) = row.invested.as_deref().and_then(decimal) {
            match add(total, cost) {
                Ok(value) => total = value,
                Err(_) => {
                    complete = false;
                    allocation_overflow = true;
                }
            }
        }
    }
    if !complete {
        allocation_issues.push("A distribuição por fundo e setor declarado precisa do custo completo de todos os fundos com cotas; nenhum peso foi presumido.".into());
    }
    if allocation_overflow {
        allocation_issues.push(CalculationIssue::Overflow.text().into());
    }

    let held = |row: &&PurchaseAnalysisRow| {
        row.fund_id.is_some()
            && row
                .shares
                .as_deref()
                .and_then(decimal)
                .is_none_or(|shares| shares > Decimal::ZERO)
    };
    let mut by_fund: Vec<_> = rows
        .iter()
        .filter(held)
        .map(|row| FundPurchaseAllocation {
            fund_id: row.fund_id.clone().expect("ledger row"),
            symbol: row.symbol.clone(),
            invested: row.invested.clone(),
            share_percent: None,
        })
        .collect();
    let mut sector_map = BTreeMap::<String, SectorPurchaseAllocation>::new();
    for row in rows.iter().filter(held) {
        let declared = row.declared_sector.clone().unwrap_or_default();
        let sector = sector_map
            .entry(declared.trim().to_lowercase())
            .or_insert_with(|| SectorPurchaseAllocation {
                declared_sector: declared,
                classification_known: row.classification_known,
                invested: Some("0".into()),
                share_percent: None,
                symbols: vec![],
            });
        sector.symbols.push(row.symbol.clone());
        sector.invested = match (
            sector.invested.as_deref().and_then(decimal),
            row.invested.as_deref().and_then(decimal),
        ) {
            (Some(previous), Some(cost)) => match add(previous, cost) {
                Ok(sum) => Some(text(sum)),
                Err(_) => {
                    complete = false;
                    allocation_overflow = true;
                    None
                }
            },
            _ => None,
        };
    }
    let mut by_declared_sector: Vec<_> = sector_map.into_values().collect();
    for sector in &mut by_declared_sector {
        sector.symbols.sort();
        sector.symbols.dedup();
    }
    if complete && total > Decimal::ZERO {
        let percentages = (|| {
            let fund_weights = by_fund
                .iter()
                .map(|fund| {
                    ratio_percent(
                        decimal(fund.invested.as_deref().unwrap_or_default())
                            .ok_or(CalculationIssue::Overflow)?,
                        total,
                    )
                    .map(text)
                })
                .collect::<Calculation<Vec<_>>>()?;
            let sector_weights = by_declared_sector
                .iter()
                .map(|sector| {
                    ratio_percent(
                        decimal(sector.invested.as_deref().unwrap_or_default())
                            .ok_or(CalculationIssue::Overflow)?,
                        total,
                    )
                    .map(text)
                })
                .collect::<Calculation<Vec<_>>>()?;
            Ok::<_, CalculationIssue>((fund_weights, sector_weights))
        })();
        match percentages {
            Ok((fund_weights, sector_weights)) => {
                for (fund, weight) in by_fund.iter_mut().zip(fund_weights) {
                    fund.share_percent = Some(weight);
                }
                for (sector, weight) in by_declared_sector.iter_mut().zip(sector_weights) {
                    sector.share_percent = Some(weight);
                }
                for row in &mut rows {
                    row.invested_share_percent = by_fund
                        .iter()
                        .find(|fund| Some(&fund.fund_id) == row.fund_id.as_ref())
                        .and_then(|fund| fund.share_percent.clone());
                }
            }
            Err(_) => {
                complete = false;
                allocation_overflow = true;
                allocation_issues.push(
                    "Os pesos excederam a precisão decimal suportada; nenhum foi publicado.".into(),
                );
            }
        }
    }
    if by_declared_sector
        .iter()
        .any(|sector| !sector.classification_known)
    {
        allocation_issues.push("Setores vazios ou genéricos continuam sem classificação específica. Os rótulos são declarados no histórico, não representam verificação da exposição real dos fundos.".into());
    }
    let missing_data =
        rows.iter().any(|row| row.missing_data) || (!complete && !allocation_overflow);
    let overflow = rows.iter().any(|row| row.overflow) || allocation_overflow;
    let issues: Vec<_> = rows
        .iter()
        .flat_map(|row| {
            row.issues
                .iter()
                .map(|issue| format!("{}: {issue}", row.symbol))
        })
        .chain(allocation_issues.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    PurchaseAnalysis {
        as_of: today.to_string(), threshold_percent: "1".into(), scope: SCOPE.into(), basis: BASIS.into(), source: SOURCE.into(),
        basis_complete: complete, missing_data, overflow, rows,
        allocation: PurchaseAllocation { scope: SCOPE.into(), basis: "Purchase cost only; percentages rounded to two decimal places, without market appreciation, dividends, cash or post-fixed income.".into(), basis_complete: complete, total_invested: complete.then(|| text(total)), by_fund, by_declared_sector, issues: allocation_issues },
        issues, quote_errors: quotes.errors.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 21).unwrap()
    }
    fn fund(ticker: &str, sector: &str, buys: &[(&str, &str)]) -> FundView {
        let entries: Vec<_> = buys.iter().enumerate().map(|(index, (quantity, price))| json!({"id":format!("{ticker}-{index}"),"month":"2026-09","cotasAdded":quantity,"pricePerCota":price,"dividendPerCota":"999","dividendTotal":"999","notes":""})).collect();
        serde_json::from_value(json!({"id":format!("fund-{ticker}"),"ticker":ticker,"name":format!("Fund {ticker}"),"sector":sector,"entries":entries,"totalCotas":"9999","totalInvested":"9999","averagePrice":"9999","dividendsReceived":"999","estimatedMonthlyIncome":"999"})).unwrap()
    }
    fn snapshot(funds: Vec<FundView>) -> PortfolioSnapshot {
        serde_json::from_value(json!({"funds":funds,"totals":{"invested":"9999","cotas":"9999","dividendsReceived":"999","estimatedMonthlyIncome":"999"},"recordCount":0})).unwrap()
    }
    fn quotes(prices: &[(&str, &str)]) -> QuotePanel {
        serde_json::from_value(json!({"items":prices.iter().map(|(ticker, price)| json!({"symbol":ticker,"status":"cached","error":"original provider error","observation":{"symbol":ticker,"price":price,"currency":"BRL","quotedAt":"2026-10-20T16:00:00Z","fetchedAt":"2026-10-21T12:00:00Z","source":"synthetic","sourceUrl":"https://example.invalid/quote","delayMinutes":30}})).collect::<Vec<_>>(),"fetchedAt":"2026-10-21T12:00:00Z","credentialConfigured":false,"errors":["original panel error"]})).unwrap()
    }

    #[test]
    fn weighted_purchases_ignore_aggregates_and_dividends_and_preserve_inputs() {
        let original = snapshot(vec![fund(
            "TEST11",
            "Logística",
            &[("2", "50"), ("3", "100")],
        )]);
        let quotes = quotes(&[("TEST11", "79")]);
        let before = serde_json::to_vec(&original).unwrap();
        let report = build_purchase_analysis(&original, &quotes, today());
        let row = &report.rows[0];
        assert_eq!(
            (
                row.shares.as_deref(),
                row.invested.as_deref(),
                row.average_price.as_deref()
            ),
            (Some("5"), Some("400"), Some("80"))
        );
        assert_eq!(
            (row.price_band.as_str(), row.difference_percent.as_deref()),
            ("below", Some("-1.25"))
        );
        assert_eq!(row.quote.as_ref(), quotes.items.first());
        assert_eq!(report.quote_errors, quotes.errors);
        assert_eq!(serde_json::to_vec(&original).unwrap(), before);
        assert_eq!(report.threshold_percent, "1");
    }

    #[test]
    fn exact_inclusive_one_percent_boundaries_do_not_use_rounded_prices() {
        for (quote, expected) in [
            ("98.999999999999999999", "below"),
            ("99", "near"),
            ("100", "near"),
            ("101", "near"),
            ("101.000000000000000001", "above"),
        ] {
            let report = build_purchase_analysis(
                &snapshot(vec![fund("TEST11", "", &[("1", "100")])]),
                &quotes(&[("TEST11", quote)]),
                today(),
            );
            assert_eq!(report.rows[0].price_band, expected, "{quote}");
        }
    }

    #[test]
    fn futures_and_zero_quantity_information_do_not_change_eligible_basis() {
        let mut item = fund(
            "TEST11",
            "Recebíveis",
            &[
                ("2", "50"),
                ("0", "9999"),
                ("10", "500"),
                ("10", "1000"),
                ("10", "2000"),
            ],
        );
        item.entries[2].month = "2026-11".into();
        item.entries[3].month = "2026-10".into();
        item.entries[3].purchase_date = Some("2026-10-22".into());
        item.entries[4].month = "2026-10".into();
        item.entries[4].purchase_date = Some("2026-10-21".into());
        let report = build_purchase_analysis(
            &snapshot(vec![item]),
            &quotes(&[("TEST11", "1675")]),
            today(),
        );
        assert_eq!(report.rows[0].shares.as_deref(), Some("12"));
        assert_eq!(report.rows[0].invested.as_deref(), Some("20100"));
        assert_eq!(report.rows[0].average_price.as_deref(), Some("1675"));
        assert_eq!(report.rows[0].price_band, "near");
    }

    #[test]
    fn incomplete_cost_suppresses_all_allocation_weights_and_signals() {
        let report = build_purchase_analysis(
            &snapshot(vec![
                fund("KNOWN11", "Logística", &[("1", "100")]),
                fund("COST11", "Recebíveis", &[("1", "0")]),
            ]),
            &quotes(&[("KNOWN11", "90"), ("COST11", "1")]),
            today(),
        );
        assert_eq!(report.rows[1].price_band, "unknown");
        assert!(!report.basis_complete);
        assert!(report.allocation.total_invested.is_none());
        assert!(report
            .rows
            .iter()
            .all(|row| row.invested_share_percent.is_none()));
        assert!(report
            .allocation
            .by_fund
            .iter()
            .all(|fund| fund.share_percent.is_none()));
    }

    #[test]
    fn missing_foreign_mismatched_and_malformed_quotes_are_neutral() {
        let wallet = snapshot(vec![fund("TEST11", "Logística", &[("1", "100")])]);
        for mode in ["missing", "foreign", "mismatch", "zero", "invalid"] {
            let mut panel = quotes(&[("TEST11", "50")]);
            match mode {
                "missing" => panel.items[0].observation = None,
                "foreign" => panel.items[0].observation.as_mut().unwrap().currency = "USD".into(),
                "mismatch" => {
                    panel.items[0].observation.as_mut().unwrap().symbol = "OTHER11".into()
                }
                "zero" => panel.items[0].observation.as_mut().unwrap().price = "0".into(),
                _ => panel.items[0].observation.as_mut().unwrap().price = "NaN".into(),
            }
            let report = build_purchase_analysis(&wallet, &panel, today());
            assert_eq!(report.rows[0].price_band, "unknown", "{mode}");
            assert_eq!(report.rows[0].quote.as_ref(), panel.items.first());
            assert!(report.rows[0].difference_percent.is_none());
            assert!(
                report.basis_complete,
                "quote availability does not change ledger cost"
            );
        }
    }

    #[test]
    fn cost_allocation_groups_declared_sectors_without_inferring_generic_exposure() {
        let report = build_purchase_analysis(
            &snapshot(vec![
                fund("ONE11", "Logística", &[("2", "50")]),
                fund("TWO11", "Logística", &[("3", "100")]),
                fund("GEN11", "Fundo Imobiliário", &[("2", "50")]),
            ]),
            &quotes(&[("ONE11", "40"), ("TWO11", "120"), ("GEN11", "50")]),
            today(),
        );
        assert_eq!(report.allocation.total_invested.as_deref(), Some("500"));
        assert_eq!(
            report
                .rows
                .iter()
                .map(|row| row.invested_share_percent.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("20"), Some("60"), Some("20")]
        );
        let logistics = report
            .allocation
            .by_declared_sector
            .iter()
            .find(|sector| sector.declared_sector == "Logística")
            .unwrap();
        assert_eq!(logistics.invested.as_deref(), Some("400"));
        assert_eq!(logistics.share_percent.as_deref(), Some("80"));
        assert!(logistics.classification_known);
        assert!(!report.rows[2].classification_known);
        assert!(
            !report
                .allocation
                .by_declared_sector
                .iter()
                .find(|sector| sector.declared_sector == "Fundo Imobiliário")
                .unwrap()
                .classification_known
        );
        assert!(report.allocation.scope.contains("excludes post-fixed"));
    }

    #[test]
    fn normalized_duplicate_tickers_and_duplicate_quotes_fail_neutral() {
        let report = build_purchase_analysis(
            &snapshot(vec![
                fund("TEST11", "", &[("1", "100")]),
                fund(" test11 ", "", &[("1", "200")]),
            ]),
            &quotes(&[("TEST11", "50")]),
            today(),
        );
        assert!(report.rows.iter().all(|row| row.price_band == "unknown"));
        assert!(!report.basis_complete);
        let panel = quotes(&[("TEST11", "50"), ("test11", "120")]);
        let report = build_purchase_analysis(
            &snapshot(vec![fund("TEST11", "", &[("1", "100")])]),
            &panel,
            today(),
        );
        assert_eq!(report.rows[0].price_band, "unknown");
        assert_eq!(report.rows[0].quote_candidates.len(), 2);
        assert!(report.rows[0].quote.is_none());
    }

    #[test]
    fn quote_only_rows_stay_visible_without_ledger_basis_or_weights() {
        let report =
            build_purchase_analysis(&snapshot(vec![]), &quotes(&[("BROK11", "50")]), today());
        assert_eq!(report.rows.len(), 1);
        assert!(report.rows[0].fund_id.is_none());
        assert_eq!(report.rows[0].price_band, "unknown");
        assert!(report.rows[0].invested_share_percent.is_none());
        assert!(report.allocation.by_fund.is_empty());
    }

    #[test]
    fn broker_only_positive_holding_blocks_partial_denominator_and_does_not_supply_a_purchase_price(
    ) {
        let mut wallet = snapshot(vec![fund("TEST11", "Logística", &[("1", "100")])]);
        wallet.broker_position = Some(serde_json::from_value(json!({
            "referenceDate":"2026-10-20", "institution":"Synthetic", "importedAt":"2026-10-20T12:00:00Z", "sourcePath":"", "sourceHash":"",
            "overview":{"updatedTime":null,"capturedTime":null,"totalWithCash":"9000","invested":"9000","cash":"0","listedFunds":"300","fixedIncome":"8700","pendingIncome":"0"},
            "listedFunds":{"capturedTime":null,"marketValue":"300","costBasis":"300","unrealizedResult":"0","dailyResult":"0","holdings":[{"ticker":"BROK11","shares":"2","averagePrice":"100","price":"100","marketValue":"200","unrealizedResult":"0","costBasis":"200"}]},
            "fixedIncome":{"name":"Unrelated post-fixed fund","nameTruncated":false,"balance":"8700","netBalance":"8700","reportedEarnings":"700","return12Months":"10","redemptionConversion":"D+0","redemptionSettlement":"D+0"},"warnings":[]
        })).unwrap());
        let panel = quotes(&[("TEST11", "90"), ("BROK11", "50")]);
        let report = build_purchase_analysis(&wallet, &panel, today());
        assert!(!report.basis_complete);
        assert!(report.allocation.total_invested.is_none());
        assert!(report
            .rows
            .iter()
            .all(|row| row.invested_share_percent.is_none()));
        assert_eq!(report.rows[0].price_band, "below");
        assert_eq!(report.rows[0].invested.as_deref(), Some("100"));
        assert_eq!(report.rows[1].price_band, "unknown");
        assert!(report.rows[1].average_price.is_none());
        assert!(report.rows[1].invested.is_none());
        assert_eq!(report.rows[1].quote.as_ref(), panel.items.get(1));
        assert!(report
            .allocation
            .by_fund
            .iter()
            .all(|fund| fund.symbol != "BROK11"));
        // Merely registering the broker symbol without any dated purchases
        // still cannot provide its unknown acquisition cost to the denominator.
        wallet.funds.push(fund("BROK11", "", &[]));
        let registered = build_purchase_analysis(&wallet, &panel, today());
        assert!(!registered.basis_complete);
        assert!(registered
            .rows
            .iter()
            .all(|row| row.invested_share_percent.is_none()));
    }

    #[test]
    fn overflow_and_unrepresentable_precision_never_produce_a_price_band() {
        for buys in [
            vec![("2", "79228162514264337593543950335")],
            vec![("0.0000000000000000000000000001", "0.1")],
        ] {
            let report = build_purchase_analysis(
                &snapshot(vec![fund("TEST11", "", &buys)]),
                &quotes(&[("TEST11", "1")]),
                today(),
            );
            assert_eq!(report.rows[0].price_band, "unknown");
            assert!(report.rows[0].overflow);
            assert!(report.overflow);
            assert!(!report.basis_complete);
        }
    }

    #[test]
    fn invalid_purchase_dates_and_decimal_strings_remain_missing() {
        for bad in ["2026-09-31", "2026-08-20"] {
            let mut item = fund("TEST11", "", &[("1", "100")]);
            item.entries[0].purchase_date = Some(bad.into());
            let report = build_purchase_analysis(
                &snapshot(vec![item]),
                &quotes(&[("TEST11", "90")]),
                today(),
            );
            assert_eq!(report.rows[0].price_band, "unknown");
            assert!(report.missing_data);
            assert!(!report.overflow);
        }
        for bad in ["-1", "1e2", "NaN", "1,5", ""] {
            let report = build_purchase_analysis(
                &snapshot(vec![fund("TEST11", "", &[("1", bad)])]),
                &quotes(&[("TEST11", "90")]),
                today(),
            );
            assert_eq!(report.rows[0].price_band, "unknown");
            assert!(report.missing_data);
        }
    }
}
