//! Historical valuations derived from real public raw prices/NAV and the user's
//! declared movements. This is not an investor account statement: NAV cannot
//! reveal undisclosed redemptions, tax redemptions, or changes in held units.
use crate::{
    add, decimal_string, div,
    income::{calculate_monthly_income, IncomeEvent},
    money, mul, normalize_ticker, parse_decimal, parse_nonnegative,
    performance::DailyCdiRate,
    portfolio_evolution::{reconstructed_month_cdi, PortfolioEvolution},
    CoreError, PortfolioSnapshot, Result,
};
use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalMarketPrice {
    pub symbol: String,
    pub date: String,
    /// Unadjusted BRL exchange closing price. Never adjusted-close or FII NAV.
    pub raw_close: String,
    pub source: String,
    pub source_url: String,
    pub fetched_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FundNavObservation {
    pub cnpj: String,
    pub date: String,
    pub value: String,
    pub source: String,
    pub source_url: String,
    pub fetched_at: String,
}

/// The caller must explicitly link the investor's fixed-income fund to this
/// verified CNPJ/class and confirm D+0 contribution conversion before providing
/// a series. Matching only a similar fund name is insufficient.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalFixedIncomeSeries {
    pub cnpj: String,
    pub fund_name: String,
    pub observations: Vec<FundNavObservation>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalValuationData {
    #[serde(default)]
    pub prices: Vec<HistoricalMarketPrice>,
    #[serde(default)]
    pub fixed_income: Option<HistoricalFixedIncomeSeries>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalValuationComponent {
    pub kind: String,
    pub id: Option<String>,
    pub name: String,
    pub as_of: String,
    pub invested: String,
    pub market_value: String,
    pub result: String,
    pub source: String,
    pub source_url: String,
    pub fetched_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalMonthlyProvenance {
    pub kind: String,
    /// False means the result contains only known income, not assumed payments.
    pub income_complete: bool,
    pub components: Vec<HistoricalValuationComponent>,
    pub assumptions: Vec<String>,
}

fn invalid(message: &str) -> CoreError {
    CoreError::Validation(message.into())
}

fn day(value: &str) -> Result<NaiveDate> {
    let result = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| invalid("Data inválida na valorização histórica."))?;
    if result.year() < 1 || result.format("%Y-%m-%d").to_string() != value {
        return Err(invalid("Data inválida na valorização histórica."));
    }
    Ok(result)
}

fn end_of_month(month: &str) -> Result<NaiveDate> {
    let start = day(&format!("{month}-01"))?;
    let next = if start.month() == 12 {
        NaiveDate::from_ymd_opt(start.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(start.year(), start.month() + 1, 1)
    };
    next.and_then(|date| date.pred_opt())
        .ok_or_else(|| invalid("Mês inválido na valorização histórica."))
}

fn subtract(left: Decimal, right: Decimal) -> Result<Decimal> {
    left.checked_sub(right).ok_or(CoreError::ArithmeticOverflow)
}

fn positive(value: &str) -> Result<Decimal> {
    let result = parse_nonnegative(value, "Preço histórico")?;
    if result.is_zero() {
        return Err(invalid("O preço histórico precisa ser positivo."));
    }
    Ok(result)
}

fn source_valid(source: &str, url: &str, fetched_at: &str) -> bool {
    !source.trim().is_empty()
        && url.starts_with("https://")
        && chrono::DateTime::parse_from_rfc3339(fetched_at).is_ok()
}

struct HistoricalIndex<'a> {
    prices: BTreeMap<String, BTreeMap<NaiveDate, &'a HistoricalMarketPrice>>,
    navs: BTreeMap<NaiveDate, &'a FundNavObservation>,
    price_conflicts: BTreeMap<String, BTreeSet<NaiveDate>>,
    nav_conflicts: BTreeSet<NaiveDate>,
}

impl<'a> HistoricalIndex<'a> {
    fn new(history: &'a HistoricalValuationData, today: NaiveDate) -> Result<Self> {
        let mut index = Self {
            prices: BTreeMap::new(),
            navs: BTreeMap::new(),
            price_conflicts: BTreeMap::new(),
            nav_conflicts: BTreeSet::new(),
        };
        for observation in &history.prices {
            let symbol = normalize_ticker(&observation.symbol)?;
            let date = day(&observation.date)?;
            positive(&observation.raw_close)?;
            if date > today
                || !source_valid(
                    &observation.source,
                    &observation.source_url,
                    &observation.fetched_at,
                )
            {
                return Err(invalid(
                    "A cotação histórica possui data futura ou fonte inválida.",
                ));
            }
            let values = index.prices.entry(symbol.clone()).or_default();
            if let Some(previous) = values.insert(date, observation) {
                if parse_decimal(&previous.raw_close)? != parse_decimal(&observation.raw_close)? {
                    index
                        .price_conflicts
                        .entry(symbol)
                        .or_default()
                        .insert(date);
                }
            }
        }
        if let Some(fixed) = &history.fixed_income {
            if fixed.cnpj.len() != 14
                || !fixed.cnpj.bytes().all(|byte| byte.is_ascii_digit())
                || fixed.fund_name.trim().is_empty()
            {
                return Err(invalid("Identificação do fundo pós-fixado inválida."));
            }
            for observation in &fixed.observations {
                let date = day(&observation.date)?;
                positive(&observation.value)?;
                if observation.cnpj != fixed.cnpj
                    || date > today
                    || !source_valid(
                        &observation.source,
                        &observation.source_url,
                        &observation.fetched_at,
                    )
                {
                    return Err(invalid(
                        "A cota histórica não corresponde ao fundo ou à data válida.",
                    ));
                }
                if let Some(previous) = index.navs.insert(date, observation) {
                    if parse_decimal(&previous.value)? != parse_decimal(&observation.value)? {
                        index.nav_conflicts.insert(date);
                    }
                }
            }
        }
        Ok(index)
    }

    fn price(&self, symbol: &str, cutoff: NaiveDate) -> Option<&HistoricalMarketPrice> {
        let symbol = symbol.to_uppercase();
        let (date, value) = self.prices.get(&symbol)?.range(..=cutoff).next_back()?;
        (date.month() == cutoff.month()
            && date.year() == cutoff.year()
            && (cutoff - *date).num_days() <= 7
            && !self
                .price_conflicts
                .get(&symbol)
                .is_some_and(|days| days.contains(date)))
        .then_some(*value)
    }

    fn closing_nav(&self, cutoff: NaiveDate) -> Option<&FundNavObservation> {
        let (date, value) = self.navs.range(..=cutoff).next_back()?;
        (date.month() == cutoff.month()
            && date.year() == cutoff.year()
            && (cutoff - *date).num_days() <= 7
            && !self.nav_conflicts.contains(date))
        .then_some(*value)
    }
}

fn fixed_income_component(
    snapshot: &PortfolioSnapshot,
    history: &HistoricalValuationData,
    index: &HistoricalIndex<'_>,
    cutoff: NaiveDate,
) -> Result<Option<HistoricalValuationComponent>> {
    let Some(position) = &snapshot.broker_position else {
        return Ok(None);
    };
    let Some(observation) = &position.fixed_income.latest_observation else {
        if parse_nonnegative(&position.fixed_income.balance, "Saldo do pós-fixado")?.is_zero() {
            return Ok(None);
        }
        return Err(invalid(
            "Pós-fixado: faltam os aportes com data para reconstruir os meses.",
        ));
    };
    let mut invested = Decimal::ZERO;
    let mut eligible = Vec::new();
    let mut all_principal = Decimal::ZERO;
    if observation.contributions.is_empty()
        && (!parse_nonnegative(&observation.invested, "Valor aplicado no pós-fixado")?.is_zero()
            || !parse_nonnegative(&observation.balance, "Saldo do pós-fixado")?.is_zero())
    {
        return Err(invalid(
            "Pós-fixado: faltam os aportes com data para reconstruir os meses.",
        ));
    }
    for contribution in &observation.contributions {
        let amount = parse_nonnegative(&contribution.invested, "Aporte do pós-fixado")?;
        all_principal = add(all_principal, amount)?;
        if day(&contribution.application_date)? <= cutoff && !amount.is_zero() {
            invested = add(invested, amount)?;
            eligible.push(contribution);
        }
    }
    if all_principal != parse_nonnegative(&observation.invested, "Valor aplicado no pós-fixado")? {
        return Err(invalid(
            "Pós-fixado: os aportes declarados não conferem com o valor aplicado.",
        ));
    }
    if invested.is_zero() {
        return Ok(None);
    }
    let series = history.fixed_income.as_ref().ok_or_else(|| {
        invalid("Pós-fixado: falta o histórico de cotas da classe identificada na CVM.")
    })?;
    let closing = index.closing_nav(cutoff).ok_or_else(|| {
        invalid("Pós-fixado: falta uma cota pública válida no fechamento deste mês.")
    })?;
    let mut units = Decimal::ZERO;
    for contribution in eligible {
        let application = day(&contribution.application_date)?;
        let initial = index.navs.get(&application).filter(|_| !index.nav_conflicts.contains(&application))
            .ok_or_else(|| invalid("Pós-fixado: falta a cota pública exata da data de um aporte; a data não foi deslocada."))?;
        units = add(
            units,
            div(
                parse_nonnegative(&contribution.invested, "Aporte do pós-fixado")?,
                positive(&initial.value)?,
            )?,
        )?;
    }
    let market_value = money(mul(units, positive(&closing.value)?)?);
    Ok(Some(HistoricalValuationComponent {
        kind: "fixedIncome".into(),
        id: Some(series.cnpj.clone()),
        name: observation.name.clone(),
        as_of: closing.date.clone(),
        invested: decimal_string(invested),
        market_value: decimal_string(market_value),
        result: decimal_string(subtract(market_value, invested)?),
        source: closing.source.clone(),
        source_url: closing.source_url.clone(),
        fetched_at: closing.fetched_at.clone(),
    }))
}

fn unresolved_cash(snapshot: &PortfolioSnapshot, cutoff: NaiveDate) -> Result<bool> {
    for pocket in &snapshot.cash_pockets {
        if let Some(flows) = &pocket.cash_flows {
            if !flows.is_empty()
                && flows
                    .iter()
                    .all(|flow| day(&flow.date).is_ok_and(|date| date > cutoff))
            {
                continue;
            }
        }
        let empty = parse_nonnegative(&pocket.balance, "Saldo da caixinha")?.is_zero()
            && pocket
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
                == Some(Decimal::ZERO);
        if !empty {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Fill missing closed-month points using real source prices and declared
/// holdings. Actual archived observations always have priority. No financial or
/// observation records are persisted, and missing cash balances remain gaps.
pub fn reconstruct_portfolio_evolution(
    snapshot: &PortfolioSnapshot,
    base: &mut PortfolioEvolution,
    history: &HistoricalValuationData,
    events: &[IncomeEvent],
    rates: &[DailyCdiRate],
    today: NaiveDate,
) -> Result<()> {
    let index = HistoricalIndex::new(history, today)?;
    for point in &mut base.points {
        if point.portfolio_result.is_some() {
            continue;
        }
        let cutoff = end_of_month(&point.month)?;
        if cutoff >= today {
            continue;
        }
        let attempt = || -> Result<(Decimal, Decimal, Vec<HistoricalValuationComponent>, bool, Vec<String>)> {
            if unresolved_cash(snapshot, cutoff)? {
                return Err(invalid("Caixinhas: falta um saldo real deste mês. O CDI contratado não foi tratado como saldo histórico."));
            }
            let first_income = snapshot.funds.iter().flat_map(|fund| &fund.entries)
                .filter(|entry| entry.month <= point.month).map(|entry| entry.month.as_str()).min();
            let income = first_income.map(|first| calculate_monthly_income(snapshot, events, first, &point.month, cutoff)).transpose()?;
            let mut by_fund = BTreeMap::<String, Decimal>::new();
            let mut income_complete = true;
            let mut issues = Vec::new();
            if let Some(report) = income {
                for month in report.months {
                    for fund in month.funds {
                        income_complete &= !matches!(fund.status.as_str(), "missing" | "date_required");
                        if let Some(amount) = fund.amount {
                            let total = by_fund.entry(fund.fund_id).or_default();
                            *total = add(*total, parse_nonnegative(&amount, "Proventos conhecidos")?)?;
                        }
                    }
                }
            }
            if !income_complete {
                issues.push("Proventos: o resultado inclui os valores manuais e públicos conhecidos; há períodos sem cobertura ou com data de compra pendente, sem pagamentos presumidos.".into());
            }
            let mut components = Vec::new();
            let mut invested = Decimal::ZERO;
            let mut result = Decimal::ZERO;
            for fund in &snapshot.funds {
                let mut held = Decimal::ZERO;
                let mut cost = Decimal::ZERO;
                for entry in &fund.entries {
                    if entry.month > point.month || entry.purchase_date.as_deref().is_some_and(|date| date > cutoff.to_string().as_str()) {
                        continue;
                    }
                    let shares = parse_nonnegative(&entry.cotas_added, "Cotas compradas")?;
                    let price = parse_nonnegative(&entry.price_per_cota, "Preço de compra")?;
                    if !shares.is_zero() && price.is_zero() {
                        return Err(invalid("FIIs: falta o preço de uma compra neste período."));
                    }
                    held = add(held, shares)?;
                    cost = add(cost, mul(shares, price)?)?;
                }
                if held.is_zero() {
                    continue;
                }
                let quote = index.price(&fund.ticker, cutoff).ok_or_else(|| {
                    invalid(&format!("{}: falta um fechamento histórico bruto válido neste mês.", fund.ticker))
                })?;
                let market = money(mul(held, positive(&quote.raw_close)?)?);
                let gain = add(subtract(market, cost)?, by_fund.get(&fund.id).copied().unwrap_or_default())?;
                invested = add(invested, cost)?;
                result = add(result, gain)?;
                components.push(HistoricalValuationComponent {
                    kind: "fii".into(), id: Some(fund.id.clone()), name: fund.ticker.clone(), as_of: quote.date.clone(),
                    invested: decimal_string(cost), market_value: decimal_string(market), result: decimal_string(gain),
                    source: quote.source.clone(), source_url: quote.source_url.clone(), fetched_at: quote.fetched_at.clone(),
                });
            }
            if let Some(fixed) = fixed_income_component(snapshot, history, &index, cutoff)? {
                invested = add(invested, parse_nonnegative(&fixed.invested, "Aplicado no pós-fixado")?)?;
                result = add(result, parse_decimal(&fixed.result)?)?;
                components.push(fixed);
            }
            if components.is_empty() {
                return Err(invalid("Não há investimento com histórico de valorização neste mês."));
            }
            Ok((invested, result, components, income_complete, issues))
        };
        match attempt() {
            Ok((invested, result, components, income_complete, mut issues)) => {
                let (cdi, cdi_issues) = reconstructed_month_cdi(snapshot, rates, cutoff, today)?;
                issues.extend(cdi_issues);
                let mut assumptions = vec![
                    "Fechamento reconstruído com preços públicos reais e quantidades compradas no histórico. Não é um saldo mensal observado na corretora.".into(),
                    "FIIs: fechamento bruto do último pregão disponível no mês, mais proventos conhecidos uma única vez. Sem compra nova, as cotas continuam acumuladas.".into(),
                    "As quantidades seguem os movimentos informados; desdobramentos, amortizações e conversões de ativos não informados precisam ser conciliados no histórico.".into(),
                ];
                if components
                    .iter()
                    .any(|component| component.kind == "fixedIncome")
                {
                    assumptions.push("Pós-fixado: valor bruto calculado pelos aportes declarados e pelas cotas CVM; a data da aplicação foi usada como data de cotização (D+0). Sem movimentos de cotas não informados, resgates ou ajustes individuais de impostos; não altera o saldo informado da corretora.".into());
                }
                point.as_of = cutoff.to_string();
                point.invested = Some(decimal_string(invested));
                point.portfolio_result = Some(decimal_string(result));
                point.difference = if income_complete {
                    cdi.as_deref()
                        .map(parse_decimal)
                        .transpose()?
                        .map(|benchmark| subtract(result, benchmark))
                        .transpose()?
                        .map(decimal_string)
                } else {
                    None
                };
                point.cdi_result = cdi;
                point.status = if point.difference.is_some() && income_complete {
                    "available"
                } else {
                    "incomplete"
                }
                .into();
                point.issues = issues;
                point.estimated = false;
                point.is_partial_month = false;
                point.provenance = Some(HistoricalMonthlyProvenance {
                    kind: "reconstructed".into(),
                    income_complete,
                    components,
                    assumptions,
                });
            }
            Err(error) => {
                let message = error.to_string();
                if !point.issues.contains(&message) {
                    point.issues.push(message);
                }
            }
        }
    }
    base.issues = base
        .points
        .iter()
        .flat_map(|point| point.issues.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if base.points.iter().any(|point| {
        point
            .provenance
            .as_ref()
            .is_some_and(|source| source.kind == "reconstructed")
    }) {
        base.assumptions.retain(|assumption| {
            !assumption.starts_with("Cada mês usa o último saldo real observado")
                && !assumption.starts_with("Se uma compra ou rendimento antigo for corrigido")
        });
        for assumption in [
            "Os meses encerrados usam registros observados ou uma reconstrução com fechamentos públicos reais e os movimentos declarados. Consulte a origem de cada ponto; períodos sem preços, cotas ou saldos suficientes continuam com lacunas.",
            "A reconstrução não é um extrato da corretora: o pós-fixado usa a data da aplicação como cotização D+0 e não presume movimentos de cotas ou impostos individuais não informados. Valores observados permanecem prioritários.",
            "Correções retroativas nos movimentos ou proventos recalculam os meses reconstruídos. Registros observados que não correspondem mais à base financeira permanecem pendentes de nova observação.",
        ] {
            if !base.assumptions.iter().any(|saved| saved == assumption) {
                base.assumptions.push(assumption.into());
            }
        }
    }
    if !base.points.is_empty() {
        base.status = if base.points.iter().all(|point| point.status == "available") {
            "available"
        } else {
            "partial"
        }
        .into();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portfolio_evolution::calculate_portfolio_evolution;
    use serde_json::json;

    fn date(value: &str) -> NaiveDate {
        day(value).unwrap()
    }

    fn snapshot() -> PortfolioSnapshot {
        serde_json::from_value(json!({"funds":[],"totals":{"invested":"0","cotas":"0","dividendsReceived":"0","estimatedMonthlyIncome":"0"},
            "recordCount":0,"cashPockets":[],"cashBalance":"0"})).unwrap()
    }

    fn fii() -> PortfolioSnapshot {
        serde_json::from_value(json!({"funds":[{"id":"fund-a","ticker":"TEST11","name":"Fundo de teste","sector":"",
            "entries":[{"id":"a","month":"2026-01","cotasAdded":"10","pricePerCota":"10","dividendPerCota":"0","dividendTotal":"2","notes":""},
                {"id":"b","month":"2026-02","cotasAdded":"0","pricePerCota":"0","dividendPerCota":"0","dividendTotal":"3","notes":""}],
            "totalCotas":"10","totalInvested":"100","averagePrice":"10","dividendsReceived":"5","estimatedMonthlyIncome":"0"}],
            "totals":{"invested":"100","cotas":"10","dividendsReceived":"5","estimatedMonthlyIncome":"0"},"recordCount":2,"cashPockets":[],"cashBalance":"0"})).unwrap()
    }

    fn fixed() -> crate::BrokerPosition {
        serde_json::from_value(json!({"referenceDate":"2026-02-27","institution":"Teste","importedAt":"2026-02-27T12:00:00Z",
            "sourcePath":"fixture.json","sourceHash":"a".repeat(64),"warnings":[],
            "overview":{"totalWithCash":"120","invested":"100","cash":"0","listedFunds":"0","fixedIncome":"120","pendingIncome":"0"},
            "listedFunds":{"marketValue":"0","costBasis":"0","unrealizedResult":"0","dailyResult":"0","holdings":[]},
            "fixedIncome":{"name":"Pós-fixado de teste","nameTruncated":false,"balance":"120","netBalance":"119","reportedEarnings":"20",
                "return12Months":"9999","redemptionConversion":"D+0","redemptionSettlement":"D+0",
                "latestObservation":{"referenceDate":"2026-02-27","name":"Pós-fixado de teste","category":"Renda fixa","riskLevel":"1",
                    "balance":"120","invested":"100","totalEarnings":"20","earnings12Months":"9999","return12Months":"9999",
                    "returnReferenceDate":"2026-02-27","contributions":[{"applicationDate":"2026-01-02","invested":"100","currentValue":"120","earnings":"20","returnPercent":"20"}],
                    "sources":[],"importedAt":"2026-02-27T12:00:00Z","sourcePath":"fixture.json","sourceHash":"b".repeat(64)}}})).unwrap()
    }

    fn prices() -> Vec<HistoricalMarketPrice> {
        [("2026-01-30", "9"), ("2026-02-27", "11")]
            .iter()
            .map(|(date, value)| HistoricalMarketPrice {
                symbol: "TEST11".into(),
                date: (*date).into(),
                raw_close: (*value).into(),
                source: "Teste raw close".into(),
                source_url: "https://example.test/close".into(),
                fetched_at: "2026-03-03T12:00:00Z".into(),
            })
            .collect()
    }

    fn navs() -> HistoricalFixedIncomeSeries {
        HistoricalFixedIncomeSeries {
            cnpj: "00332266000131".into(),
            fund_name: "Pós-fixado de teste".into(),
            observations: [
                ("2026-01-02", "1"),
                ("2026-01-30", "1.1"),
                ("2026-02-27", "1.2"),
            ]
            .iter()
            .map(|(date, value)| FundNavObservation {
                cnpj: "00332266000131".into(),
                date: (*date).into(),
                value: (*value).into(),
                source: "Teste CVM".into(),
                source_url: "https://example.test/nav".into(),
                fetched_at: "2026-03-03T12:00:00Z".into(),
            })
            .collect(),
        }
    }

    fn rates() -> Vec<DailyCdiRate> {
        let mut result = Vec::new();
        let mut current = date("2025-12-30");
        while current <= date("2026-03-03") {
            if !matches!(
                current.weekday(),
                chrono::Weekday::Sat | chrono::Weekday::Sun
            ) {
                result.push(DailyCdiRate {
                    date: current.to_string(),
                    value: "0.01".into(),
                });
            }
            current = current.succ_opt().unwrap();
        }
        result
    }

    fn evolution(
        snapshot: &PortfolioSnapshot,
        history: &HistoricalValuationData,
        events: &[IncomeEvent],
    ) -> PortfolioEvolution {
        let mut base = calculate_portfolio_evolution(
            snapshot,
            &rates(),
            events,
            &[],
            None,
            date("2026-03-03"),
        )
        .unwrap();
        reconstruct_portfolio_evolution(
            snapshot,
            &mut base,
            history,
            events,
            &rates(),
            date("2026-03-03"),
        )
        .unwrap();
        base
    }

    #[test]
    fn legacy_purchase_month_and_unchanged_holdings_reconstruct_raw_price_and_income_once() {
        let original = fii();
        let before = serde_json::to_value(&original).unwrap();
        let history = HistoricalValuationData {
            prices: prices(),
            fixed_income: None,
        };
        let event = IncomeEvent {
            symbol: "TEST11".into(),
            amount_per_share: "999".into(),
            record_date: "2026-01-05".into(),
            payment_date: "2026-01-15".into(),
            source_url: "https://example.test/income".into(),
            source: "Test".into(),
            id: "income".into(),
        };
        let actual = evolution(&original, &history, &[event]);
        assert_eq!(actual.points[0].portfolio_result.as_deref(), Some("-8"));
        assert_eq!(actual.points[1].portfolio_result.as_deref(), Some("15"));
        assert_eq!(actual.points[1].invested.as_deref(), Some("100"));
        assert_eq!(actual.points[0].as_of, "2026-01-31");
        assert_eq!(
            actual.points[0].provenance.as_ref().unwrap().components[0].as_of,
            "2026-01-30"
        );
        assert!(!actual.points[0].estimated);
        assert!(actual.points[0]
            .issues
            .iter()
            .any(|issue| issue.contains("sem dia")));
        assert_eq!(serde_json::to_value(&original).unwrap(), before);
    }

    #[test]
    fn explicit_zero_overrides_public_income_and_is_not_a_missing_month() {
        let mut original = fii();
        original.funds[0].entries[0].dividend_total = Some("0".into());
        let public = IncomeEvent {
            symbol: "TEST11".into(),
            amount_per_share: "0.5".into(),
            record_date: "2026-01-05".into(),
            payment_date: "2026-01-15".into(),
            source_url: "https://example.test/income".into(),
            source: "Test".into(),
            id: "income".into(),
        };
        let actual = evolution(
            &original,
            &HistoricalValuationData {
                prices: prices(),
                fixed_income: None,
            },
            &[public],
        );
        assert_eq!(actual.points[0].portfolio_result.as_deref(), Some("-10"));
        assert!(
            actual.points[0]
                .provenance
                .as_ref()
                .unwrap()
                .income_complete
        );
        assert!(actual.points[0].difference.is_some());
    }

    #[test]
    fn uncovered_income_is_partial_and_cannot_claim_above_or_below_cdi() {
        let mut original = fii();
        original.funds[0].entries[0].dividend_total = None;
        let actual = evolution(
            &original,
            &HistoricalValuationData {
                prices: prices(),
                fixed_income: None,
            },
            &[],
        );
        assert_eq!(actual.points[0].portfolio_result.as_deref(), Some("-10"));
        assert_eq!(actual.points[0].status, "incomplete");
        assert!(actual.points[0].difference.is_none());
        assert!(
            !actual.points[0]
                .provenance
                .as_ref()
                .unwrap()
                .income_complete
        );
        assert!(
            !actual.points[1]
                .provenance
                .as_ref()
                .unwrap()
                .income_complete
        );
    }

    #[test]
    fn historical_api_income_uses_data_com_and_deduplicates_known_events() {
        let mut original = fii();
        original.funds[0].entries[0].dividend_total = None;
        original.funds[0].entries[0].purchase_date = Some("2026-01-02".into());
        let event = IncomeEvent {
            symbol: "TEST11".into(),
            amount_per_share: "0.2".into(),
            record_date: "2026-01-05".into(),
            payment_date: "2026-01-15".into(),
            source_url: "https://example.test/income".into(),
            source: "Test".into(),
            id: "income".into(),
        };
        let actual = evolution(
            &original,
            &HistoricalValuationData {
                prices: prices(),
                fixed_income: None,
            },
            &[event.clone(), event],
        );
        assert_eq!(actual.points[0].portfolio_result.as_deref(), Some("-8"));
        assert!(
            actual.points[0]
                .provenance
                .as_ref()
                .unwrap()
                .income_complete
        );
    }

    #[test]
    fn missing_same_month_purchase_day_never_assumes_dividend_entitlement() {
        let mut original = fii();
        original.funds[0].entries[0].dividend_total = None;
        let event = IncomeEvent {
            symbol: "TEST11".into(),
            amount_per_share: "0.2".into(),
            record_date: "2026-01-05".into(),
            payment_date: "2026-01-15".into(),
            source_url: "https://example.test/income".into(),
            source: "Test".into(),
            id: "income".into(),
        };
        let actual = evolution(
            &original,
            &HistoricalValuationData {
                prices: prices(),
                fixed_income: None,
            },
            &[event],
        );
        assert_eq!(actual.points[0].portfolio_result.as_deref(), Some("-10"));
        assert!(
            !actual.points[0]
                .provenance
                .as_ref()
                .unwrap()
                .income_complete
        );
    }

    #[test]
    fn future_purchases_are_excluded_and_retroactive_changes_recalculate_later_months() {
        let original = fii();
        let history = HistoricalValuationData {
            prices: prices(),
            fixed_income: None,
        };
        let before = evolution(&original, &history, &[]);
        let mut changed = original.clone();
        changed.funds[0].entries.push(serde_json::from_value(json!({"id":"future","month":"2026-03","purchaseDate":"2026-03-02","cotasAdded":"100","pricePerCota":"100","dividendPerCota":"0","notes":""})).unwrap());
        let after = evolution(&changed, &history, &[]);
        assert_eq!(
            before.points[0].portfolio_result,
            after.points[0].portfolio_result
        );
        assert_eq!(
            before.points[1].portfolio_result,
            after.points[1].portfolio_result
        );
        changed.funds[0].entries[0].cotas_added = "20".into();
        let revised = evolution(&changed, &history, &[]);
        assert_eq!(revised.points[0].portfolio_result.as_deref(), Some("-18"));
        assert_eq!(revised.points[1].portfolio_result.as_deref(), Some("25"));
    }

    #[test]
    fn no_prior_month_or_stale_close_is_used_and_conflicting_price_is_unavailable() {
        let mut history = HistoricalValuationData {
            prices: prices(),
            fixed_income: None,
        };
        history.prices[0].date = "2026-01-02".into();
        let mut conflict = history.prices[1].clone();
        conflict.raw_close = "12".into();
        history.prices.push(conflict);
        let actual = evolution(&fii(), &history, &[]);
        assert!(actual.points[0].portfolio_result.is_none());
        assert!(actual.points[1].portfolio_result.is_none());
        history.prices.clear();
        history.prices.push(HistoricalMarketPrice {
            date: "2025-12-31".into(),
            ..prices()[0].clone()
        });
        assert!(evolution(&fii(), &history, &[]).points[0]
            .portfolio_result
            .is_none());
    }

    #[test]
    fn observed_points_win_and_current_partial_month_is_not_backfilled() {
        let original = fii();
        let mut base =
            calculate_portfolio_evolution(&original, &rates(), &[], &[], None, date("2026-03-03"))
                .unwrap();
        base.points[0].portfolio_result = Some("777".into());
        let mut history = HistoricalValuationData {
            prices: prices(),
            fixed_income: None,
        };
        history.prices.push(HistoricalMarketPrice {
            date: "2026-03-03".into(),
            ..prices()[0].clone()
        });
        reconstruct_portfolio_evolution(
            &original,
            &mut base,
            &history,
            &[],
            &rates(),
            date("2026-03-03"),
        )
        .unwrap();
        assert_eq!(base.points[0].portfolio_result.as_deref(), Some("777"));
        assert!(base.points[0].provenance.is_none());
        assert!(base.points[2].portfolio_result.is_none());
    }

    #[test]
    fn fixed_nav_reconstruction_uses_real_ratio_and_discloses_gross_basis() {
        let mut original = snapshot();
        original.broker_position = Some(fixed());
        let before = serde_json::to_value(&original).unwrap();
        let actual = evolution(
            &original,
            &HistoricalValuationData {
                prices: vec![],
                fixed_income: Some(navs()),
            },
            &[],
        );
        assert_eq!(actual.points[0].portfolio_result.as_deref(), Some("10"));
        assert_eq!(actual.points[1].portfolio_result.as_deref(), Some("20"));
        let provenance = actual.points[0].provenance.as_ref().unwrap();
        assert_eq!(provenance.components[0].as_of, "2026-01-30");
        assert!(provenance
            .assumptions
            .iter()
            .any(|issue| issue.contains("impostos")));
        assert!(provenance
            .assumptions
            .iter()
            .any(|issue| issue.contains("data da aplicação foi usada como data de cotização")));
        assert!(actual
            .assumptions
            .iter()
            .any(|issue| issue.contains("fechamentos públicos reais")));
        assert!(!actual
            .assumptions
            .iter()
            .any(|issue| issue.starts_with("Cada mês usa o último saldo real observado")));
        assert_eq!(serde_json::to_value(&original).unwrap(), before);
    }

    #[test]
    fn later_fixed_contributions_do_not_affect_prior_month_and_nav_exact_date_is_required() {
        let mut original = snapshot();
        original.broker_position = Some(fixed());
        let observation = original
            .broker_position
            .as_mut()
            .unwrap()
            .fixed_income
            .latest_observation
            .as_mut()
            .unwrap();
        observation.invested = "150".into();
        observation
            .contributions
            .push(crate::BrokerFixedIncomeContribution {
                application_date: "2026-02-02".into(),
                invested: "50".into(),
                current_value: "50".into(),
                earnings: "0".into(),
                return_percent: "0".into(),
            });
        let actual = evolution(
            &original,
            &HistoricalValuationData {
                prices: vec![],
                fixed_income: Some(navs()),
            },
            &[],
        );
        assert_eq!(actual.points[0].portfolio_result.as_deref(), Some("10"));
        assert!(actual.points[1].portfolio_result.is_none());
        assert!(actual.points[1]
            .issues
            .iter()
            .any(|issue| issue.contains("data de um aporte")));
    }

    #[test]
    fn fixed_class_mismatch_and_future_observations_are_rejected() {
        let original = fii();
        let mut history = HistoricalValuationData {
            prices: prices(),
            fixed_income: Some(navs()),
        };
        history.fixed_income.as_mut().unwrap().observations[0].cnpj = "97521225000125".into();
        let mut base =
            calculate_portfolio_evolution(&original, &rates(), &[], &[], None, date("2026-03-03"))
                .unwrap();
        assert!(reconstruct_portfolio_evolution(
            &original,
            &mut base,
            &history,
            &[],
            &rates(),
            date("2026-03-03")
        )
        .is_err());
        history.fixed_income = None;
        history.prices[0].date = "2026-03-04".into();
        assert!(reconstruct_portfolio_evolution(
            &original,
            &mut base,
            &history,
            &[],
            &rates(),
            date("2026-03-03")
        )
        .is_err());
    }

    #[test]
    fn any_unvalued_cash_pocket_prevents_incomplete_wallet_from_being_plotted_as_whole() {
        let mut original = fii();
        original.cash_pockets.push(serde_json::from_value(json!({"id":"cash","name":"Caixinha","institution":"Teste","balance":"110","totalDeposits":"100","totalWithdrawals":"0","balanceDate":"2026-02-27", "cashFlows":[{"date":"2026-01-02","kind":"deposit","amount":"100"}],"cdiPercent":"120","annualCdiRate":"9999","notes":"","createdAt":"2026-01-02T00:00:00Z","updatedAt":"2026-02-27T00:00:00Z"})).unwrap());
        let actual = evolution(
            &original,
            &HistoricalValuationData {
                prices: prices(),
                fixed_income: None,
            },
            &[],
        );
        assert!(actual.points[0].portfolio_result.is_none());
        assert!(actual.points[1].portfolio_result.is_none());
        assert!(actual.points[0]
            .issues
            .iter()
            .any(|issue| issue.contains("Caixinhas")));
    }

    #[test]
    fn all_assets_are_summed_once_without_broker_aggregate_or_twelve_month_metric() {
        let mut original = fii();
        original.broker_position = Some(fixed());
        let actual = evolution(
            &original,
            &HistoricalValuationData {
                prices: prices(),
                fixed_income: Some(navs()),
            },
            &[],
        );
        assert_eq!(actual.points[0].portfolio_result.as_deref(), Some("2"));
        assert_eq!(actual.points[1].portfolio_result.as_deref(), Some("35"));
        assert_eq!(actual.points[1].invested.as_deref(), Some("200"));
        assert_eq!(
            actual.points[1]
                .provenance
                .as_ref()
                .unwrap()
                .components
                .len(),
            2
        );
        assert!(actual.points[1].difference.is_some());
    }

    #[test]
    fn calendar_month_close_preserves_each_sources_real_business_date() {
        let mut original = fii();
        original.broker_position = Some(fixed());
        let mut fixed_history = navs();
        fixed_history.observations[1].date = "2026-01-29".into();
        let actual = evolution(
            &original,
            &HistoricalValuationData {
                prices: prices(),
                fixed_income: Some(fixed_history),
            },
            &[],
        );
        let point = &actual.points[0];
        assert_eq!(point.as_of, "2026-01-31");
        assert_eq!(point.portfolio_result.as_deref(), Some("2"));
        let components = &point.provenance.as_ref().unwrap().components;
        assert_eq!(components[0].as_of, "2026-01-30");
        assert_eq!(components[1].as_of, "2026-01-29");
        assert!(point.cdi_result.is_some());
        assert!(point.difference.is_some());
    }

    #[test]
    fn easter_month_end_keeps_original_flows_and_uses_only_published_cdi_without_covering_live_or_long_gaps(
    ) {
        let mut original = fii();
        for entry in &mut original.funds[0].entries {
            entry.month = "2024-03".into();
            entry.cotas_added = "10".into();
            entry.price_per_cota = "10".into();
            entry.dividend_total = Some("0".into());
        }
        original.funds[0].entries[0].purchase_date = Some("2024-03-01".into());
        let before = serde_json::to_value(&original).unwrap();
        let mut rates = Vec::new();
        let mut current = date("2024-02-29");
        while current <= date("2024-03-28") {
            if !matches!(
                current.weekday(),
                chrono::Weekday::Sat | chrono::Weekday::Sun
            ) {
                rates.push(DailyCdiRate {
                    date: current.to_string(),
                    value: if current == date("2024-03-28") {
                        "1"
                    } else {
                        "0"
                    }
                    .into(),
                });
            }
            current = current.succ_opt().unwrap();
        }
        rates.push(DailyCdiRate {
            date: "2024-04-01".into(),
            value: "0".into(),
        });
        let today = date("2024-04-03");
        let history = HistoricalValuationData {
            prices: vec![HistoricalMarketPrice {
                symbol: "TEST11".into(),
                date: "2024-03-28".into(),
                raw_close: "11".into(),
                source: "Teste raw close".into(),
                source_url: "https://example.test/close".into(),
                fetched_at: "2024-04-03T12:00:00Z".into(),
            }],
            fixed_income: None,
        };
        let mut base =
            calculate_portfolio_evolution(&original, &rates, &[], &[], None, today).unwrap();
        reconstruct_portfolio_evolution(&original, &mut base, &history, &[], &rates, today)
            .unwrap();
        let point = &base.points[0];
        assert_eq!(point.as_of, "2024-03-31");
        assert_eq!(point.invested.as_deref(), Some("200"));
        assert_eq!(point.portfolio_result.as_deref(), Some("20"));
        // Only the March 1 purchase earns the published 1% on March 28. The
        // month-only purchase remains a March 31 flow and earns nothing yet.
        assert_eq!(point.cdi_result.as_deref(), Some("1"));
        assert_eq!(point.difference.as_deref(), Some("19"));
        assert!(point
            .issues
            .iter()
            .any(|issue| issue.contains("2024-03-28") && issue.contains("datas originais")));
        assert_eq!(serde_json::to_value(&original).unwrap(), before);
        let without_resumption = rates
            .iter()
            .filter(|rate| rate.date.as_str() <= "2024-03-31")
            .cloned()
            .collect::<Vec<_>>();
        let (pending_publication, _) =
            reconstructed_month_cdi(&original, &without_resumption, date("2024-03-31"), today)
                .unwrap();
        assert!(pending_publication.is_none());
        let (live, _) = reconstructed_month_cdi(
            &original,
            &without_resumption,
            date("2024-03-31"),
            date("2024-03-31"),
        )
        .unwrap();
        assert!(live.is_none());
        rates
            .retain(|rate| rate.date.as_str() <= "2024-03-20" || rate.date.as_str() > "2024-03-31");
        let (stale, _) =
            reconstructed_month_cdi(&original, &rates, date("2024-03-31"), today).unwrap();
        assert!(stale.is_none());
    }
}
