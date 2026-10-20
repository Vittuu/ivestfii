//! Read-only, cumulative FII result and an explicitly modelled 100% CDI scenario.
//! Neither result is a time-weighted return or an annualized rate. Cash pockets,
//! imported fixed income, CVM NAV and imported aggregate wealth are not inputs.
use super::{
    add, decimal_string, div, money, mul, normalize_purchase_date, normalize_ticker,
    parse_nonnegative, CoreError, PortfolioSnapshot, Result,
};
use crate::income::{calculate_monthly_income, IncomeEvent};
use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceQuote {
    pub ticker: String,
    pub price: String,
    /// Market observation date, not the retrieval date.
    pub as_of: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyCdiRate {
    pub date: String,
    /// SGS 12: percentage per day. This is not an annual percentage.
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CdiPerformance {
    pub ending_value: String,
    pub result: String,
    pub return_percent: String,
}

/// Provenance of cumulative known income. API income is calculated from public
/// distributions and eligible holdings, never persisted as a received payment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FiiIncomeBreakdown {
    pub recorded_amount: String,
    pub api_amount: String,
    pub as_of: String,
    pub start_month: String,
    pub end_month: String,
    /// `complete` or `partial`: known income does not prove absent payments.
    pub status: String,
    pub missing_fund_month_count: usize,
    pub date_required_fund_month_count: usize,
    pub ignored_event_count: usize,
    pub by_fund: Vec<FundIncomeBreakdown>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FundIncomeBreakdown {
    pub fund_id: String,
    pub symbol: String,
    pub recorded_amount: String,
    pub api_amount: String,
    pub total_amount: String,
    pub missing_month_count: usize,
    pub date_required_month_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FiiPerformance {
    pub status: String,
    pub as_of: Option<String>,
    pub start_month: Option<String>,
    pub invested: String,
    pub dividends: String,
    /// Absent for legacy/manual-only calculations and old serialized reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub income: Option<FiiIncomeBreakdown>,
    pub market_value: Option<String>,
    pub price_result: Option<String>,
    pub total_result: Option<String>,
    pub total_return_percent: Option<String>,
    pub cdi: Option<CdiPerformance>,
    pub difference: Option<String>,
    pub issues: Vec<String>,
    pub assumptions: Vec<String>,
}

fn invalid(message: &str) -> CoreError {
    CoreError::Validation(message.into())
}

fn date(value: &str) -> Result<NaiveDate> {
    let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| invalid("Data inválida no cálculo de resultado."))?;
    if parsed.year() < 1 || parsed.format("%Y-%m-%d").to_string() != value {
        return Err(invalid("Data inválida no cálculo de resultado."));
    }
    Ok(parsed)
}

fn month(value: &str) -> Result<NaiveDate> {
    if value.len() != 7 {
        return Err(invalid("Mês inválido no histórico de FIIs."));
    }
    date(&format!("{value}-01"))
}

fn end_of_month(start: NaiveDate) -> Result<NaiveDate> {
    let (year, month) = if start.month() == 12 {
        (start.year() + 1, 1)
    } else {
        (start.year(), start.month() + 1)
    };
    NaiveDate::from_ymd_opt(year, month, 1)
        .and_then(|next| next.pred_opt())
        .ok_or_else(|| invalid("Mês fora do intervalo suportado."))
}

fn subtract(left: Decimal, right: Decimal) -> Result<Decimal> {
    left.checked_sub(right).ok_or(CoreError::ArithmeticOverflow)
}

fn percent(result: Decimal, invested: Decimal) -> Result<Decimal> {
    mul(div(result, invested)?, Decimal::ONE_HUNDRED)
}

fn issue(issues: &mut Vec<String>, message: String) {
    if !issues.contains(&message) {
        issues.push(message);
    }
}

/// Recomputes dated ledger quantities instead of trusting aggregates that may
/// include future records. Exact explicit dividends, even zero, take precedence.
pub fn calculate_fii_performance(
    snapshot: &PortfolioSnapshot,
    quotes: &[PerformanceQuote],
    rates: &[DailyCdiRate],
    today: NaiveDate,
) -> Result<FiiPerformance> {
    let mut output = FiiPerformance {
        status: "incomplete".into(), as_of: None, start_month: None,
        invested: "0".into(), dividends: "0".into(), income: None, market_value: None,
        price_result: None, total_result: None, total_return_percent: None,
        cdi: None, difference: None, issues: vec![],
        assumptions: vec![
            "Somente os FIIs e demais fundos listados do histórico da carteira; caixinhas, caixa e pós-fixado ficam fora.".into(),
            "Resultado bruto acumulado: valor de mercado mais proventos menos custo das compras, sem impostos ou taxas.".into(),
            "A porcentagem é ganho dividido pelo custo das compras; não é TWR, taxa anualizada nem resultado ajustado pelo tempo dos fluxos.".into(),
            "Compras são custo de aquisição; sua origem externa ou reinvestida não é conhecida.".into(),
            "Proventos são somados sem remuneração ou reinvestimento no modelo; o CDI capitaliza diariamente.".into(),
            "Compras com data usam o dia informado no cenário CDI. Sem o dia, a compra é estimada no fim do mês; no mês da cotação, usa a própria data final e não recebe juros nesse dia.".into(),
            "O cenário usa 100% do CDI diário histórico SGS 12, estritamente depois da data informada ou estimada de cada compra, até a mesma data do mercado.".into(),
        ],
    };
    let mut prices = BTreeMap::new();
    let mut common_date = None;
    let mut mixed_dates = false;
    for quote in quotes {
        let ticker = normalize_ticker(&quote.ticker)?;
        let observed = date(&quote.as_of)?;
        if observed > today {
            return Err(invalid("Uma cotação possui data futura."));
        }
        let price = parse_nonnegative(&quote.price, "Cotação")?;
        if price.is_zero() {
            return Err(invalid("Uma cotação não possui preço positivo."));
        }
        if prices.insert(ticker, price).is_some() {
            return Err(invalid("Há cotações duplicadas para o mesmo ativo."));
        }
        if common_date.is_some_and(|previous| previous != observed) {
            mixed_dates = true;
        }
        common_date.get_or_insert(observed);
    }
    if mixed_dates {
        common_date = None;
        output.issues.push("As cotações possuem datas diferentes. Atualize todos os ativos para comparar na mesma data.".into());
    }
    output.as_of = common_date.map(|value| value.to_string());
    let as_of = common_date.unwrap_or(today);
    let cutoff_month = as_of.with_day(1).expect("valid calendar date");
    let broker_date = snapshot
        .broker_position
        .as_ref()
        .map(|position| date(&position.reference_date))
        .transpose()?;
    let anchor_month = broker_date
        .filter(|value| *value <= as_of)
        .map(|value| value.with_day(1).expect("valid calendar date"));
    let mut holdings = BTreeMap::new();
    let mut anchor_holdings = BTreeMap::new();
    let mut purchases = BTreeMap::new();
    let mut invested = Decimal::ZERO;
    let mut dividends = Decimal::ZERO;
    let mut seen = HashSet::new();
    let mut complete = common_date.is_some();
    let mut computed_dividend = false;
    let mut any_record = false;
    let mut future_records = false;
    for fund in &snapshot.funds {
        let ticker = normalize_ticker(&fund.ticker)?;
        if !seen.insert(ticker.clone()) {
            return Err(invalid("Há ativos duplicados no histórico da carteira."));
        }
        let mut entries = fund
            .entries
            .iter()
            .map(|entry| Ok((month(&entry.month)?, entry)))
            .collect::<Result<Vec<_>>>()?;
        entries.sort_by_key(|(month, _)| *month);
        let mut monthly_holdings = BTreeMap::new();
        let mut held = Decimal::ZERO;
        let mut anchor_held = Decimal::ZERO;
        for (entry_month, entry) in &entries {
            if *entry_month > cutoff_month {
                complete = false;
                future_records = true;
                issue(&mut output.issues, "Há lançamentos posteriores à data das cotações; eles foram excluídos e o resultado de mercado não foi calculado.".into());
                continue;
            }
            let quantity = parse_nonnegative(&entry.cotas_added, "Quantidade de cotas")?;
            let price = parse_nonnegative(&entry.price_per_cota, "Preço por cota")?;
            let purchase_date =
                normalize_purchase_date(entry.purchase_date.as_deref(), &entry.month, quantity)?
                    .as_deref()
                    .map(date)
                    .transpose()?;
            let after_cutoff = purchase_date.is_some_and(|day| day > as_of);
            if after_cutoff {
                complete = false;
                future_records = true;
                issue(&mut output.issues, "Há compras posteriores à data das cotações; elas foram excluídas e o resultado de mercado não foi calculado.".into());
            } else {
                held = add(held, quantity)?;
            }
            monthly_holdings.insert(*entry_month, held);
            if !after_cutoff
                && anchor_month.is_some_and(|anchor| *entry_month <= anchor)
                && purchase_date.is_none_or(|day| broker_date.is_some_and(|broker| day <= broker))
            {
                anchor_held = add(anchor_held, quantity)?;
            }
            if !quantity.is_zero() && !after_cutoff {
                any_record = true;
                let key = entry_month.format("%Y-%m").to_string();
                if output.start_month.as_ref().is_none_or(|start| key < *start) {
                    output.start_month = Some(key);
                }
                if price.is_zero() {
                    complete = false;
                    issue(&mut output.issues, format!("{ticker} possui cotas adicionadas sem preço de compra; o custo está incompleto."));
                }
                let cost = mul(quantity, price)?;
                invested = add(invested, cost)?;
                if !cost.is_zero() {
                    let purchase_date =
                        purchase_date.unwrap_or(end_of_month(*entry_month)?.min(as_of));
                    let previous = purchases
                        .get(&purchase_date)
                        .copied()
                        .unwrap_or(Decimal::ZERO);
                    purchases.insert(purchase_date, add(previous, cost)?);
                }
            }
        }
        for (entry_month, entry) in &entries {
            if *entry_month > cutoff_month {
                continue;
            }
            let dividend = match &entry.dividend_total {
                Some(explicit) => parse_nonnegative(explicit, "Rendimento total")?,
                None => {
                    let value = money(mul(
                        monthly_holdings
                            .get(entry_month)
                            .copied()
                            .unwrap_or(Decimal::ZERO),
                        parse_nonnegative(&entry.dividend_per_cota, "Rendimento por cota")?,
                    )?);
                    computed_dividend |= !value.is_zero();
                    value
                }
            };
            any_record |= !dividend.is_zero();
            dividends = add(dividends, dividend)?;
        }
        holdings.insert(ticker.clone(), held);
        anchor_holdings.insert(ticker, anchor_held);
    }
    output.invested = decimal_string(invested);
    output.dividends = decimal_string(dividends);
    if computed_dividend {
        output.assumptions.push("Parte dos proventos foi calculada pelas cotas finais do mês e pelo rendimento por cota; não corresponde a um comprovante de pagamento.".into());
    }
    if let Some(position) = &snapshot.broker_position {
        if anchor_month.is_some() {
            output.assumptions.push("A conciliação respeita as datas de compra informadas; compras sem dia usam as cotas finais do mês da posição importada.".into());
            let mut broker_tickers = HashSet::new();
            for holding in &position.listed_funds.holdings {
                let ticker = normalize_ticker(&holding.ticker)?;
                if !broker_tickers.insert(ticker.clone()) {
                    return Err(invalid("Há ativos duplicados na posição importada."));
                }
                let shares = parse_nonnegative(&holding.shares, "Cotas da posição importada")?;
                if anchor_holdings
                    .get(&ticker)
                    .copied()
                    .unwrap_or(Decimal::ZERO)
                    != shares
                {
                    complete = false;
                    issue(&mut output.issues, format!("{ticker}: as cotas do histórico não conferem com a posição importada na data dela."));
                }
            }
            for (ticker, held) in &anchor_holdings {
                if !held.is_zero() && !broker_tickers.contains(ticker) {
                    complete = false;
                    issue(&mut output.issues, format!("{ticker} está no histórico, mas não na posição importada naquela data."));
                }
            }
        } else {
            complete = false;
            issue(&mut output.issues, "A posição importada é posterior às cotações e não permite reconciliar as quantidades.".into());
        }
    }
    let imported_holdings = snapshot
        .broker_position
        .as_ref()
        .is_some_and(|position| !position.listed_funds.holdings.is_empty());
    if !any_record && !imported_holdings && !future_records {
        output.status = "empty".into();
        output.issues.clear();
        return Ok(output);
    }
    if invested.is_zero() {
        complete = false;
        issue(
            &mut output.issues,
            "Não há custo de compras suficiente para apurar o resultado acumulado.".into(),
        );
    }
    let mut market_value = Decimal::ZERO;
    for (ticker, held) in &holdings {
        if held.is_zero() {
            continue;
        }
        match prices.get(ticker) {
            Some(price) => market_value = add(market_value, mul(*held, *price)?)?,
            None => {
                complete = false;
                issue(
                    &mut output.issues,
                    format!("Falta uma cotação datada de {ticker}."),
                );
            }
        }
    }
    if common_date.is_none() && !mixed_dates {
        issue(
            &mut output.issues,
            "Atualize as cotações para apurar o resultado na mesma data.".into(),
        );
    }
    if !complete {
        return Ok(output);
    }
    let price_result = subtract(market_value, invested)?;
    let total_result = add(price_result, dividends)?;
    output.market_value = Some(decimal_string(market_value));
    output.price_result = Some(decimal_string(price_result));
    output.total_result = Some(decimal_string(total_result));
    output.total_return_percent = Some(decimal_string(percent(total_result, invested)?));
    output.status = "available".into();
    match calculate_cdi(&purchases, rates, as_of, today, invested) {
        Ok(cdi) => {
            output.difference = Some(decimal_string(subtract(
                total_result,
                super::parse_decimal(&cdi.result)?,
            )?));
            output.cdi = Some(cdi);
        }
        Err(error) => {
            output.status = "incomplete".into();
            output
                .issues
                .push(format!("Comparação CDI indisponível: {error}"));
        }
    }
    Ok(output)
}

/// Include only the API complement to the manual ledger. A fund/month already
/// supplied by the user (including an explicit zero) is never added a second
/// time. Public payments after the common market date are excluded from the
/// valuation and from its CDI comparison, even if the cache was fetched later.
pub fn calculate_fii_performance_with_income(
    snapshot: &PortfolioSnapshot,
    quotes: &[PerformanceQuote],
    rates: &[DailyCdiRate],
    events: &[IncomeEvent],
    today: NaiveDate,
) -> Result<FiiPerformance> {
    let mut output = calculate_fii_performance(snapshot, quotes, rates, today)?;
    let as_of = output
        .as_of
        .as_deref()
        .map(date)
        .transpose()?
        .unwrap_or(today);
    let end_month = as_of.format("%Y-%m").to_string();
    let Some(start_month) = snapshot
        .funds
        .iter()
        .flat_map(|fund| &fund.entries)
        .map(|entry| entry.month.as_str())
        .filter(|month| *month <= end_month.as_str())
        .min()
    else {
        return Ok(output);
    };
    let monthly = calculate_monthly_income(snapshot, events, start_month, &end_month, as_of)?;
    let mut by_fund = snapshot
        .funds
        .iter()
        .map(|fund| FundIncomeBreakdown {
            fund_id: fund.id.clone(),
            symbol: fund.ticker.clone(),
            recorded_amount: "0".into(),
            api_amount: "0".into(),
            total_amount: "0".into(),
            missing_month_count: 0,
            date_required_month_count: 0,
        })
        .collect::<Vec<_>>();
    let mut recorded = Decimal::ZERO;
    let mut api = Decimal::ZERO;
    let mut missing_fund_month_count = 0;
    let mut date_required_fund_month_count = 0;
    for point in &monthly.months {
        recorded = add(
            recorded,
            parse_nonnegative(&point.recorded_amount, "Proventos registrados")?,
        )?;
        // known_total includes the manual amount. Adding it here would duplicate
        // the ledger; api_amount is precisely the missing fund/month complement.
        api = add(
            api,
            parse_nonnegative(&point.api_amount, "Proventos da API")?,
        )?;
        for (fund, summary) in point.funds.iter().zip(&mut by_fund) {
            match fund.status.as_str() {
                "recorded" | "api" => {
                    let amount = parse_nonnegative(
                        fund.amount
                            .as_deref()
                            .ok_or_else(|| invalid("Rendimento conhecido sem valor."))?,
                        "Rendimento conhecido",
                    )?;
                    let target = if fund.status == "api" {
                        &mut summary.api_amount
                    } else {
                        &mut summary.recorded_amount
                    };
                    *target = decimal_string(add(
                        parse_nonnegative(target, "Rendimento conhecido")?,
                        amount,
                    )?);
                    summary.total_amount = decimal_string(add(
                        parse_nonnegative(&summary.total_amount, "Rendimento conhecido")?,
                        amount,
                    )?);
                }
                "missing" | "date_required" => {
                    missing_fund_month_count += 1;
                    summary.missing_month_count += 1;
                    if fund.status == "date_required" {
                        date_required_fund_month_count += 1;
                        summary.date_required_month_count += 1;
                    }
                }
                _ => {}
            }
        }
    }
    if recorded != parse_nonnegative(&output.dividends, "Proventos registrados")? {
        return Err(invalid(
            "Os cálculos do histórico de rendimentos não conferem no período das cotações.",
        ));
    }
    output.dividends = decimal_string(add(recorded, api)?);
    if let Some(result) = output
        .total_result
        .as_deref()
        .map(super::parse_decimal)
        .transpose()?
    {
        let result = add(result, api)?;
        output.total_result = Some(decimal_string(result));
        output.total_return_percent = Some(decimal_string(percent(
            result,
            parse_nonnegative(&output.invested, "Custo dos FIIs")?,
        )?));
        if let Some(cdi) = &output.cdi {
            output.difference = Some(decimal_string(subtract(
                result,
                super::parse_decimal(&cdi.result)?,
            )?));
        }
    }
    output.income = Some(FiiIncomeBreakdown {
        recorded_amount: decimal_string(recorded),
        api_amount: decimal_string(api),
        as_of: as_of.to_string(),
        start_month: start_month.into(),
        end_month,
        status: if missing_fund_month_count == 0 {
            "complete"
        } else {
            "partial"
        }
        .into(),
        missing_fund_month_count,
        date_required_fund_month_count,
        ignored_event_count: monthly.ignored_event_count,
        by_fund,
    });
    output.assumptions.push("Proventos públicos complementam somente os meses de cada fundo sem rendimento informado; um valor manual, inclusive zero, tem prioridade. Os valores da API não alteram os lançamentos nem comprovam crédito na corretora.".into());
    output.assumptions.push(format!("Os proventos da API incluem apenas pagamentos até {as_of}, a mesma data usada no resultado dos FIIs quando há cotações comuns."));
    if date_required_fund_month_count > 0 {
        issue(&mut output.issues, "Informe a data das compras antigas para completar os proventos da API. Os valores sem cotas confirmadas na data-com não foram presumidos.".into());
    }
    Ok(output)
}

fn calculate_cdi(
    purchases: &BTreeMap<NaiveDate, Decimal>,
    rates: &[DailyCdiRate],
    as_of: NaiveDate,
    today: NaiveDate,
    invested: Decimal,
) -> Result<CdiPerformance> {
    let first_purchase = purchases
        .keys()
        .next()
        .copied()
        .ok_or_else(|| invalid("Não há compras com custo conhecido."))?;
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
    if daily.keys().next_back().copied() != Some(as_of) {
        return Err(invalid(
            "O histórico diário ainda não cobre a data das cotações.",
        ));
    }
    if daily
        .keys()
        .next()
        .is_none_or(|first| *first > first_purchase)
    {
        return Err(invalid(
            "O histórico diário não cobre o início das compras.",
        ));
    }
    let mut balance = Decimal::ZERO;
    let mut flows = purchases.iter().peekable();
    for (day, rate) in daily {
        while flows
            .peek()
            .is_some_and(|(purchase_day, _)| **purchase_day < day)
        {
            balance = add(balance, *flows.next().expect("checked flow").1)?;
        }
        balance = mul(
            balance,
            add(Decimal::ONE, div(rate, Decimal::ONE_HUNDRED)?)?,
        )?;
        // End-of-day purchases do not receive the rate of their own day.
        while flows
            .peek()
            .is_some_and(|(purchase_day, _)| **purchase_day == day)
        {
            balance = add(balance, *flows.next().expect("checked flow").1)?;
        }
    }
    for (_, cost) in flows {
        balance = add(balance, *cost)?;
    }
    // Preserve the same acquisition-cost precision as the ledger. Rounding the
    // ending balance before subtracting a sub-cent cost could invent a loss
    // even when every daily rate is zero. The UI formats currency for display.
    let result = subtract(balance, invested)?;
    Ok(CdiPerformance {
        ending_value: decimal_string(balance),
        result: decimal_string(result),
        return_percent: decimal_string(percent(result, invested)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{calculate_fund, BrokerPosition, MonthlyEntry, PortfolioTotals};
    use serde_json::json;

    fn row(
        month: &str,
        quantity: &str,
        price: &str,
        per_share: &str,
        total: Option<&str>,
    ) -> MonthlyEntry {
        MonthlyEntry {
            id: format!("fixture-{month}-{quantity}-{price}-{per_share}"),
            month: month.into(),
            cotas_added: quantity.into(),
            purchase_date: None,
            price_per_cota: price.into(),
            dividend_per_cota: per_share.into(),
            dividend_total: total.map(str::to_owned),
            notes: String::new(),
        }
    }

    fn snapshot(entries: Vec<MonthlyEntry>) -> PortfolioSnapshot {
        let count = entries.len();
        PortfolioSnapshot {
            funds: vec![calculate_fund(
                "synthetic-id".into(),
                "TEST11".into(),
                "Fundo sintético".into(),
                "Teste".into(),
                entries,
            )
            .unwrap()],
            // Deliberately unrelated aggregates prove this calculation uses the
            // dated acquisition ledger, never the wealth/cash headline totals.
            totals: PortfolioTotals {
                invested: "999999".into(),
                cotas: "888888".into(),
                dividends_received: "777777".into(),
                estimated_monthly_income: "666666".into(),
            },
            record_count: count,
            broker_position: None,
            cash_pockets: vec![],
            cash_balance: "555555".into(),
        }
    }

    fn fixture() -> PortfolioSnapshot {
        snapshot(vec![
            row("2026-01", "10", "10", "0", Some("2")),
            row("2026-02", "5", "12", "1", Some("1")),
        ])
    }

    fn quotes(day: &str) -> Vec<PerformanceQuote> {
        vec![PerformanceQuote {
            ticker: "TEST11".into(),
            price: "12".into(),
            as_of: day.into(),
        }]
    }

    fn rates() -> Vec<DailyCdiRate> {
        // January 31 is a Saturday. Its end-of-month purchase starts earning
        // on the next published observation, February 2, never January 30.
        vec![
            DailyCdiRate {
                date: "2026-01-30".into(),
                value: "1".into(),
            },
            DailyCdiRate {
                date: "2026-02-02".into(),
                value: "1".into(),
            },
            DailyCdiRate {
                date: "2026-02-03".into(),
                value: "2".into(),
            },
        ]
    }

    fn today() -> NaiveDate {
        date("2026-02-03").unwrap()
    }

    fn broker(shares: &str, reference_date: &str) -> BrokerPosition {
        serde_json::from_value(json!({
            "referenceDate":reference_date,"institution":"Corretora sintética", "importedAt":"2026-02-03T12:00:00Z",
            "sourcePath":"synthetic.json","sourceHash":"a".repeat(64),
            "overview":{"updatedTime":null,"capturedTime":null,"totalWithCash":"100","invested":"100","cash":"0","listedFunds":"100","fixedIncome":"0","pendingIncome":"0"},
            "listedFunds":{"capturedTime":null,"marketValue":"100","costBasis":"100","unrealizedResult":"0","dailyResult":"0",
                "holdings":[{"ticker":"TEST11","shares":shares,"averagePrice":"10","price":"10","marketValue":"100","unrealizedResult":"0","costBasis":"100"}]},
            "fixedIncome":{"name":"Referência fora do cálculo","nameTruncated":false,"balance":"500000","netBalance":"500000","reportedEarnings":"400000","return12Months":"20","redemptionConversion":"D+0","redemptionSettlement":"D+0"},
            "warnings":[]
        })).unwrap()
    }

    #[test]
    fn cumulative_result_and_weekend_cdi_use_same_end_date_and_current_month_cost_gets_no_interest()
    {
        let original = fixture();
        let bytes = serde_json::to_vec(&original).unwrap();
        let result =
            calculate_fii_performance(&original, &quotes("2026-02-03"), &rates(), today()).unwrap();
        assert_eq!(result.status, "available");
        assert_eq!(result.as_of.as_deref(), Some("2026-02-03"));
        assert_eq!(result.start_month.as_deref(), Some("2026-01"));
        assert_eq!(result.invested, "160");
        assert_eq!(result.dividends, "3");
        assert_eq!(result.market_value.as_deref(), Some("180"));
        assert_eq!(result.price_result.as_deref(), Some("20"));
        assert_eq!(result.total_result.as_deref(), Some("23"));
        assert_eq!(result.total_return_percent.as_deref(), Some("14.375"));
        let cdi = result.cdi.unwrap();
        assert_eq!(cdi.ending_value, "163.02");
        assert_eq!(cdi.result, "3.02");
        assert_eq!(result.difference.as_deref(), Some("19.98"));
        assert_eq!(serde_json::to_vec(&original).unwrap(), bytes);
    }

    #[test]
    fn repeated_months_use_final_holdings_and_explicit_zero_overrides_a_positive_rate() {
        let original = snapshot(vec![
            row("2026-02", "0", "0", "0.1", None),
            row("2026-01", "10", "10", "99", Some("0")),
            row("2026-02", "2", "10", "0.1", None),
        ]);
        let result =
            calculate_fii_performance(&original, &quotes("2026-02-03"), &rates(), today()).unwrap();
        assert_eq!(result.invested, "120");
        assert_eq!(result.dividends, "2.4");
        assert_eq!(result.market_value.as_deref(), Some("144"));
        assert_eq!(result.total_result.as_deref(), Some("26.4"));
        assert!(result
            .assumptions
            .iter()
            .any(|note| note.contains("calculada pelas cotas finais")));
        assert_eq!(original.record_count, 3);
    }

    #[test]
    fn future_records_are_excluded_and_cannot_be_used_to_value_present_holdings() {
        let mut original = fixture();
        original.funds[0]
            .entries
            .push(row("2026-03", "100", "100", "1", Some("10000")));
        let result =
            calculate_fii_performance(&original, &quotes("2026-02-03"), &rates(), today()).unwrap();
        assert_eq!(result.status, "incomplete");
        assert_eq!(result.invested, "160");
        assert_eq!(result.dividends, "3");
        assert!(result.total_result.is_none());
        assert!(result.market_value.is_none());
        assert!(result.cdi.is_none());
        assert!(result
            .issues
            .iter()
            .any(|note| note.contains("posteriores")));
    }

    #[test]
    fn zero_cost_quantity_blocks_profit_but_price_only_information_does_not() {
        let mut original = fixture();
        original.funds[0]
            .entries
            .push(row("2026-01", "0", "123", "0", Some("0")));
        assert!(
            calculate_fii_performance(&original, &quotes("2026-02-03"), &rates(), today())
                .unwrap()
                .total_result
                .is_some()
        );
        original.funds[0]
            .entries
            .push(row("2026-01", "3", "0", "0", Some("0")));
        let result =
            calculate_fii_performance(&original, &quotes("2026-02-03"), &rates(), today()).unwrap();
        assert_eq!(result.status, "incomplete");
        assert_eq!(result.invested, "160");
        assert!(result.total_result.is_none());
        assert!(result
            .issues
            .iter()
            .any(|note| note.contains("custo está incompleto")));
    }

    #[test]
    fn quote_scope_dates_duplicates_and_overflow_do_not_create_a_partial_market_total() {
        let original = fixture();
        let absent = calculate_fii_performance(&original, &[], &rates(), today()).unwrap();
        assert_eq!(absent.status, "incomplete");
        assert!(absent.total_result.is_none());
        let mut duplicate = quotes("2026-02-03");
        duplicate.push(duplicate[0].clone());
        assert!(calculate_fii_performance(&original, &duplicate, &rates(), today()).is_err());
        assert!(
            calculate_fii_performance(&original, &quotes("2026-02-04"), &rates(), today()).is_err()
        );
        let mut divergent = quotes("2026-02-03");
        divergent.push(PerformanceQuote {
            ticker: "OTHR11".into(),
            price: "1".into(),
            as_of: "2026-02-02".into(),
        });
        let result = calculate_fii_performance(&original, &divergent, &rates(), today()).unwrap();
        assert!(result.as_of.is_none());
        assert!(result.total_result.is_none());
        let mut overflow = quotes("2026-02-03");
        overflow[0].price = "79228162514264337593543950335".into();
        assert!(matches!(
            calculate_fii_performance(&original, &overflow, &rates(), today()),
            Err(CoreError::ArithmeticOverflow)
        ));
    }

    #[test]
    fn missing_cdi_beginning_or_terminal_and_invalid_dates_keep_the_fii_result() {
        let original = fixture();
        let mut duplicate = rates();
        duplicate.push(duplicate[0].clone());
        let mut future = rates();
        future.push(DailyCdiRate {
            date: "2026-02-04".into(),
            value: "1".into(),
        });
        for observations in [
            vec![],
            rates()[1..].to_vec(),
            rates()[..2].to_vec(),
            duplicate,
            future,
        ] {
            let result =
                calculate_fii_performance(&original, &quotes("2026-02-03"), &observations, today())
                    .unwrap();
            assert_eq!(result.status, "incomplete");
            assert_eq!(result.total_result.as_deref(), Some("23"));
            assert!(result.cdi.is_none());
            assert!(result.difference.is_none());
            assert!(result
                .issues
                .iter()
                .any(|note| note.starts_with("Comparação CDI indisponível:")));
        }
    }

    #[test]
    fn broker_reconciliation_uses_its_own_date_and_does_not_reject_later_legitimate_buys() {
        let mut original = fixture();
        original.broker_position = Some(broker("10", "2026-01-31"));
        let result =
            calculate_fii_performance(&original, &quotes("2026-02-03"), &rates(), today()).unwrap();
        assert_eq!(result.status, "available");
        assert_eq!(result.total_result.as_deref(), Some("23"));
        original
            .broker_position
            .as_mut()
            .unwrap()
            .listed_funds
            .holdings[0]
            .shares = "9".into();
        let mismatch =
            calculate_fii_performance(&original, &quotes("2026-02-03"), &rates(), today()).unwrap();
        assert_eq!(mismatch.status, "incomplete");
        assert!(mismatch.total_result.is_none());
        assert!(mismatch
            .issues
            .iter()
            .any(|note| note.contains("cotas do histórico não conferem")));
        original
            .broker_position
            .as_mut()
            .unwrap()
            .listed_funds
            .holdings[0]
            .ticker = "MISSING11".into();
        assert!(
            calculate_fii_performance(&original, &quotes("2026-02-03"), &rates(), today())
                .unwrap()
                .total_result
                .is_none()
        );
    }

    #[test]
    fn zero_cdi_never_creates_rounding_loss_on_exact_sub_cent_acquisition_cost() {
        let original = snapshot(vec![row("2026-01", "1", "1.23456789", "0", Some("0"))]);
        let observations = rates()
            .into_iter()
            .map(|observation| DailyCdiRate {
                value: "0".into(),
                ..observation
            })
            .collect::<Vec<_>>();
        let result =
            calculate_fii_performance(&original, &quotes("2026-02-03"), &observations, today())
                .unwrap();
        let cdi = result.cdi.unwrap();
        assert_eq!(result.invested, "1.23456789");
        assert_eq!(cdi.ending_value, "1.23456789");
        assert_eq!(cdi.result, "0");
        assert_eq!(cdi.return_percent, "0");
    }

    #[test]
    fn no_fii_history_is_empty_even_when_cash_has_a_large_balance() {
        let result = calculate_fii_performance(&snapshot(vec![]), &[], &[], today()).unwrap();
        assert_eq!(result.status, "empty");
        assert_eq!(result.invested, "0");
        assert_eq!(result.dividends, "0");
        assert!(result.total_result.is_none());
        assert!(result.cdi.is_none());
        assert!(result.issues.is_empty());
    }
}
