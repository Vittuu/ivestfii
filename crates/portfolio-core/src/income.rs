//! Read-only monthly income derived from the user's ledger and dated public
//! distribution events. Derived API values never become received ledger entries.

use crate::{
    add, decimal_string, money, mul, normalize_month, normalize_purchase_date, parse_nonnegative,
    CoreError, FundView, PortfolioSnapshot, Result,
};
use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IncomeEvent {
    pub symbol: String,
    pub amount_per_share: String,
    /// Last date on which holding shares grants entitlement (data-com).
    pub record_date: String,
    pub payment_date: String,
    pub source_url: String,
    pub source: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IncomeEventCalculation {
    pub id: String,
    pub symbol: String,
    pub amount_per_share: String,
    pub record_date: String,
    pub payment_date: String,
    pub source_url: String,
    pub source: String,
    /// `api` or `date_required`. Unknown eligibility never becomes zero.
    pub status: String,
    pub eligible_shares: Option<String>,
    pub amount: Option<String>,
    pub missing_purchase_dates: Vec<String>,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FundMonthlyIncome {
    pub fund_id: String,
    pub symbol: String,
    /// `recorded`, `api`, `missing`, `date_required`, or `not_held`.
    pub status: String,
    pub amount: Option<String>,
    /// Sum of purchases through the requested month and report cutoff date.
    pub holdings: String,
    /// Set when all dated events use the same known eligible quantity.
    pub eligible_shares: Option<String>,
    pub events: Vec<IncomeEventCalculation>,
    pub manual_record_ids: Vec<String>,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MonthlyIncomePoint {
    pub month: String,
    /// Only present when every held fund is resolved. A missing fund cannot
    /// turn an incomplete month into an apparently complete monetary total.
    pub total: Option<String>,
    pub known_total: String,
    pub recorded_amount: String,
    pub api_amount: String,
    /// `complete`, `partial`, `missing`, or `not_held`.
    pub status: String,
    pub recorded_fund_count: usize,
    pub api_fund_count: usize,
    pub missing_fund_count: usize,
    pub funds: Vec<FundMonthlyIncome>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MonthlyIncomeReport {
    pub start_month: String,
    pub end_month: String,
    pub as_of: String,
    pub months: Vec<MonthlyIncomePoint>,
    /// Invalid, incomplete, conflicting, or future events were not used.
    pub ignored_event_count: usize,
}

struct ValidEvent<'a> {
    raw: &'a IncomeEvent,
    amount_per_share: Decimal,
    record_date: NaiveDate,
    payment_date: NaiveDate,
}

/// Build an inclusive month range without changing the ledger or its totals.
///
/// A user-provided monthly total (including explicit zero) or a positive
/// per-share rate takes precedence over API events for that fund and month.
/// Without a user record, dated events are calculated from purchases eligible
/// at their data-com, rather than the payment month's final share quantity.
pub fn calculate_monthly_income(
    snapshot: &PortfolioSnapshot,
    events: &[IncomeEvent],
    start_month: &str,
    end_month: &str,
    today: NaiveDate,
) -> Result<MonthlyIncomeReport> {
    let start_month = normalize_month(start_month)?;
    let end_month = normalize_month(end_month)?;
    let start = parse_date(&format!("{start_month}-01")).ok_or_else(invalid_range)?;
    let end = parse_date(&format!("{end_month}-01")).ok_or_else(invalid_range)?;
    let count = (end.year() - start.year()) * 12 + end.month() as i32 - start.month() as i32 + 1;
    if !(1..=1200).contains(&count) || end_month > today.format("%Y-%m").to_string() {
        return Err(invalid_range());
    }
    let mut symbols = HashSet::new();
    for fund in &snapshot.funds {
        if !symbols.insert(fund.ticker.to_uppercase()) {
            return Err(CoreError::Validation(
                "Há fundos duplicados no histórico usado para calcular os rendimentos.".into(),
            ));
        }
        for entry in &fund.entries {
            let month = normalize_month(&entry.month)?;
            if month != entry.month {
                return Err(CoreError::Validation("Mês do histórico inválido.".into()));
            }
            normalize_purchase_date(
                entry.purchase_date.as_deref(),
                &month,
                parse_nonnegative(&entry.cotas_added, "Quantidade de cotas")?,
            )?;
            parse_nonnegative(&entry.dividend_per_cota, "Rendimento por cota")?;
            if let Some(value) = &entry.dividend_total {
                parse_nonnegative(value, "Rendimento total")?;
            }
        }
    }

    let mut ignored_event_count = 0;
    let mut validated: BTreeMap<(String, String, String), ValidEvent<'_>> = BTreeMap::new();
    let mut conflicting = HashSet::new();
    let mut unresolved_months = HashSet::new();
    for event in events {
        let symbol = event.symbol.trim().to_uppercase();
        let Some(record_date) = parse_date(&event.record_date) else {
            ignored_event_count += 1;
            mark_unresolved_month(&mut unresolved_months, &symbols, &symbol, event, today);
            continue;
        };
        let Some(payment_date) = parse_date(&event.payment_date) else {
            ignored_event_count += 1;
            continue;
        };
        let Ok(amount_per_share) = parse_nonnegative(&event.amount_per_share, "Provento por cota")
        else {
            ignored_event_count += 1;
            mark_unresolved_month(&mut unresolved_months, &symbols, &symbol, event, today);
            continue;
        };
        if !symbols.contains(&symbol)
            || event.id.trim().is_empty()
            || event.source.trim().is_empty()
            || event.source_url.trim().is_empty()
            || record_date > payment_date
            || payment_date > today
            || record_date > today
        {
            ignored_event_count += 1;
            mark_unresolved_month(&mut unresolved_months, &symbols, &symbol, event, today);
            continue;
        }
        let key = (symbol.clone(), event.source.clone(), event.id.clone());
        if conflicting.contains(&key) {
            ignored_event_count += 1;
            continue;
        }
        if let Some(existing) = validated.get(&key) {
            ignored_event_count += 1;
            if existing.record_date != record_date
                || existing.payment_date != payment_date
                || existing.amount_per_share != amount_per_share
            {
                mark_unresolved_month(
                    &mut unresolved_months,
                    &symbols,
                    &symbol,
                    existing.raw,
                    today,
                );
                mark_unresolved_month(&mut unresolved_months, &symbols, &symbol, event, today);
                validated.remove(&key);
                conflicting.insert(key);
                ignored_event_count += 1;
            }
            continue;
        }
        validated.insert(
            key,
            ValidEvent {
                raw: event,
                amount_per_share,
                record_date,
                payment_date,
            },
        );
    }
    // Source identifiers are useful for detecting revisions, but are not an
    // economic identity: a B3 event and an administrator document can describe
    // the same payment. Reconcile those mirrors before multiplying by shares.
    let validated = reconcile_events(validated, &mut unresolved_months, &mut ignored_event_count);
    let mut months = Vec::with_capacity(count as usize);
    for offset in 0..count {
        let index = start.year() * 12 + start.month0() as i32 + offset;
        let month = format!("{:04}-{:02}", index / 12, index % 12 + 1);
        let mut funds = Vec::with_capacity(snapshot.funds.len());
        let mut recorded_amount = Decimal::ZERO;
        let mut api_amount = Decimal::ZERO;
        let mut recorded_fund_count = 0;
        let mut api_fund_count = 0;
        let mut missing_fund_count = 0;
        for fund in &snapshot.funds {
            let result = fund_month(
                fund,
                &month,
                &validated,
                unresolved_months.contains(&(fund.ticker.to_uppercase(), month.clone())),
                today,
            )?;
            match result.status.as_str() {
                "recorded" => {
                    recorded_amount = add(
                        recorded_amount,
                        parse_nonnegative(
                            result.amount.as_deref().unwrap(),
                            "Rendimento registrado",
                        )?,
                    )?;
                    recorded_fund_count += 1;
                }
                "api" => {
                    api_amount = add(
                        api_amount,
                        parse_nonnegative(
                            result.amount.as_deref().unwrap(),
                            "Rendimento calculado",
                        )?,
                    )?;
                    api_fund_count += 1;
                }
                "missing" | "date_required" => missing_fund_count += 1,
                _ => {}
            }
            funds.push(result);
        }
        let known_total = add(recorded_amount, api_amount)?;
        let known_count = recorded_fund_count + api_fund_count;
        let status = match (known_count, missing_fund_count) {
            (0, 0) => "not_held",
            (_, 0) => "complete",
            (0, _) => "missing",
            _ => "partial",
        };
        months.push(MonthlyIncomePoint {
            month,
            total: (missing_fund_count == 0).then(|| decimal_string(known_total)),
            known_total: decimal_string(known_total),
            recorded_amount: decimal_string(recorded_amount),
            api_amount: decimal_string(api_amount),
            status: status.into(),
            recorded_fund_count,
            api_fund_count,
            missing_fund_count,
            funds,
        });
    }
    Ok(MonthlyIncomeReport {
        start_month,
        end_month,
        as_of: today.to_string(),
        months,
        ignored_event_count,
    })
}

fn fund_month(
    fund: &FundView,
    month: &str,
    events: &BTreeMap<(String, String, String), ValidEvent<'_>>,
    unresolved_events: bool,
    as_of: NaiveDate,
) -> Result<FundMonthlyIncome> {
    let mut holdings = Decimal::ZERO;
    let as_of = as_of.to_string();
    for entry in &fund.entries {
        if entry.month.as_str() <= month
            && entry
                .purchase_date
                .as_deref()
                .is_none_or(|day| day <= as_of.as_str())
        {
            holdings = add(
                holdings,
                parse_nonnegative(&entry.cotas_added, "Quantidade de cotas")?,
            )?;
        }
    }
    let mut result = FundMonthlyIncome {
        fund_id: fund.id.clone(),
        symbol: fund.ticker.clone(),
        status: "missing".into(),
        amount: None,
        holdings: decimal_string(holdings),
        eligible_shares: None,
        events: Vec::new(),
        manual_record_ids: Vec::new(),
        reasons: Vec::new(),
    };
    let mut manual_total = Decimal::ZERO;
    for entry in fund.entries.iter().filter(|entry| entry.month == month) {
        let rate = parse_nonnegative(&entry.dividend_per_cota, "Rendimento por cota")?;
        let value = match &entry.dividend_total {
            Some(explicit) => Some(parse_nonnegative(explicit, "Rendimento total")?),
            None if rate > Decimal::ZERO && holdings > Decimal::ZERO => {
                Some(money(mul(holdings, rate)?))
            }
            None => None,
        };
        if let Some(value) = value {
            manual_total = add(manual_total, value)?;
            result.manual_record_ids.push(entry.id.clone());
        }
    }
    if !result.manual_record_ids.is_empty() {
        result.status = "recorded".into();
        result.amount = Some(decimal_string(manual_total));
        result.reasons.push("O rendimento informado no histórico tem prioridade; eventos da API não são somados novamente.".into());
        return Ok(result);
    }
    if holdings.is_zero() {
        result.status = "not_held".into();
        result.amount = Some("0".into());
        result
            .reasons
            .push("Não havia cotas compradas no histórico até este mês.".into());
        return Ok(result);
    }
    if unresolved_events {
        result.reasons.push("Há eventos de pagamento incompletos ou conflitantes para este mês. Aguarde uma fonte completa ou informe o rendimento no histórico.".into());
        return Ok(result);
    }
    for event in events.values().filter(|event| {
        event.raw.symbol.trim().eq_ignore_ascii_case(&fund.ticker)
            && event.payment_date.format("%Y-%m").to_string() == month
    }) {
        let mut eligible = Decimal::ZERO;
        let record_month = event.record_date.format("%Y-%m").to_string();
        let mut missing_dates = Vec::new();
        let closed_month_sources = equity_month_closed_after(event.record_date);
        let mut used_undated_month_bound = false;
        for entry in &fund.entries {
            let shares = parse_nonnegative(&entry.cotas_added, "Quantidade de cotas")?;
            if shares.is_zero() || entry.month > record_month {
                continue;
            }
            match &entry.purchase_date {
                Some(date) if date.as_str() <= event.raw.record_date.as_str() => {
                    eligible = add(eligible, shares)?
                }
                Some(_) => {}
                None if entry.month < record_month => eligible = add(eligible, shares)?,
                None if closed_month_sources.is_some() => {
                    // No possible equity-market purchase session remains after
                    // data-com in this month. This proves eligibility without
                    // assigning an invented day to the historical purchase.
                    eligible = add(eligible, shares)?;
                    used_undated_month_bound = true;
                }
                None => missing_dates.push(entry.id.clone()),
            }
        }
        let known = missing_dates.is_empty();
        let mut reasons = if known {
            Vec::new()
        } else {
            vec!["Há compras no mês da data-com sem dia informado. Informe a data dessas compras para calcular as cotas com direito ao provento.".into()]
        };
        if known && used_undated_month_bound {
            reasons.push("As compras sem dia informado deste mês eram elegíveis: não havia outro dia possível de negociação de renda variável após a data-com até o fim do mês. A data de cada compra foi preservada.".into());
            reasons.extend(closed_month_sources.unwrap().into_iter().map(|url| {
                format!("Calendário de negociação da B3 confirmado para esse período: {url}")
            }));
        }
        result.events.push(IncomeEventCalculation {
            id: event.raw.id.clone(),
            symbol: fund.ticker.clone(),
            amount_per_share: decimal_string(event.amount_per_share),
            record_date: event.raw.record_date.clone(),
            payment_date: event.raw.payment_date.clone(),
            source_url: event.raw.source_url.clone(),
            source: event.raw.source.clone(),
            status: if known { "api" } else { "date_required" }.into(),
            eligible_shares: known.then(|| decimal_string(eligible)),
            amount: if known {
                Some(decimal_string(money(mul(
                    eligible,
                    event.amount_per_share,
                )?)))
            } else {
                None
            },
            missing_purchase_dates: missing_dates,
            reasons,
        });
    }
    result.events.sort_by(|a, b| {
        (&a.payment_date, &a.record_date, &a.id).cmp(&(&b.payment_date, &b.record_date, &b.id))
    });
    if result.events.is_empty() {
        result.reasons.push("Não há rendimento informado nem evento completo de pagamento disponível para este mês.".into());
        return Ok(result);
    }
    if result.events.iter().any(|event| event.amount.is_none()) {
        result.status = "date_required".into();
        result.reasons.push("Falta a data de uma compra para conferir as cotas na data-com. O total deste fundo não foi presumido.".into());
        return Ok(result);
    }
    let mut amount = Decimal::ZERO;
    for event in &result.events {
        amount = add(
            amount,
            parse_nonnegative(event.amount.as_deref().unwrap(), "Rendimento calculado")?,
        )?;
    }
    let first_quantity = result.events[0].eligible_shares.clone();
    if result
        .events
        .iter()
        .all(|event| event.eligible_shares == first_quantity)
    {
        result.eligible_shares = first_quantity;
    }
    result.status = "api".into();
    result.amount = Some(decimal_string(amount));
    result.reasons.push("Calculado pelos proventos por cota publicados e pelas cotas elegíveis na data-com; não confirma um crédito na corretora.".into());
    Ok(result)
}

fn mark_unresolved_month(
    unresolved: &mut HashSet<(String, String)>,
    symbols: &HashSet<String>,
    symbol: &str,
    event: &IncomeEvent,
    today: NaiveDate,
) {
    if !symbols.contains(symbol) {
        return;
    }
    if let Some(payment_date) = parse_date(&event.payment_date).filter(|date| *date <= today) {
        unresolved.insert((symbol.into(), payment_date.format("%Y-%m").to_string()));
    }
}

/// Reconcile event mirrors independently from source-specific identifiers.
/// Different rates on the same dated entitlement are not separate income.
/// Different dates published by different sources are not summed unless one
/// source independently lists the complete set of distinct dated payments.
fn reconcile_events<'a>(
    mut events: BTreeMap<(String, String, String), ValidEvent<'a>>,
    unresolved: &mut HashSet<(String, String)>,
    ignored: &mut usize,
) -> BTreeMap<(String, String, String), ValidEvent<'a>> {
    type Identity = (String, String, String);
    let mut rejected = BTreeSet::<Identity>::new();
    let mut dated = BTreeMap::<(String, NaiveDate, NaiveDate), Vec<Identity>>::new();
    for (identity, event) in &events {
        dated
            .entry((identity.0.clone(), event.record_date, event.payment_date))
            .or_default()
            .push(identity.clone());
    }
    for identities in dated.values() {
        let rates = identities
            .iter()
            .map(|identity| events[identity].amount_per_share)
            .collect::<BTreeSet<_>>();
        if rates.len() > 1 {
            rejected.extend(identities.iter().cloned());
        }
    }
    // Check both kinds of boundary revision. A genuine installment can share
    // data-com with another payment; a genuine extra distribution can share its
    // payment date. A source listing both establishes that distinct set, while
    // two sources each listing a different lone boundary do not establish it.
    for group_by_payment in [true, false] {
        let mut boundaries = BTreeMap::<(String, NaiveDate), Vec<Identity>>::new();
        for (identity, event) in &events {
            let boundary = if group_by_payment {
                event.payment_date
            } else {
                event.record_date
            };
            boundaries
                .entry((identity.0.clone(), boundary))
                .or_default()
                .push(identity.clone());
        }
        for identities in boundaries.values() {
            let mut sources = BTreeMap::<&str, BTreeSet<NaiveDate>>::new();
            let mut all = BTreeSet::new();
            for identity in identities {
                let event = &events[identity];
                let counterpart = if group_by_payment {
                    event.record_date
                } else {
                    event.payment_date
                };
                all.insert(counterpart);
                sources
                    .entry(event.raw.source.trim())
                    .or_default()
                    .insert(counterpart);
            }
            if all.len() > 1 && sources.len() > 1 && !sources.values().any(|dates| dates == &all) {
                rejected.extend(identities.iter().cloned());
            }
        }
    }
    for identity in rejected {
        if let Some(event) = events.remove(&identity) {
            unresolved.insert((identity.0, event.payment_date.format("%Y-%m").to_string()));
            *ignored += 1;
        }
    }
    let mut reconciled = BTreeMap::new();
    for identities in dated.into_values() {
        let mut candidates = identities
            .into_iter()
            .filter_map(|identity| events.remove(&identity).map(|event| (identity, event)))
            .collect::<Vec<_>>();
        candidates.sort_by(|(_, left), (_, right)| {
            event_provenance_order(left.raw).cmp(&event_provenance_order(right.raw))
        });
        if !candidates.is_empty() {
            *ignored += candidates.len() - 1;
            let (identity, event) = candidates.remove(0);
            reconciled.insert(identity, event);
        }
    }
    reconciled
}

fn event_provenance_order(event: &IncomeEvent) -> (u8, &str, &str, &str) {
    let source = event.source.trim();
    let is_b3 = source.eq_ignore_ascii_case("b3")
        || source
            .get(.."B3 ·".len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("B3 ·"));
    ((!is_b3) as u8, source, &event.source_url, &event.id)
}

/// An undated purchase can be eligible without guessing its day only when no
/// equity-trading session was possible after data-com through calendar month-end.
/// Weekends are deterministic; weekday closures below are pinned to historical
/// dates explicitly confirmed by B3, never extrapolated into another year.
fn equity_month_closed_after(date: NaiveDate) -> Option<Vec<&'static str>> {
    // B3's official calendar, specifically its archived 2024 section: March 29
    // and December 31 list no equity-market trading. This is not a generated
    // holiday calendar and does not assume the same closures in any other year.
    const B3_2024_CALENDAR: &str = "https://www.b3.com.br/pt_br/solucoes/plataformas/puma-trading-system/para-participantes-e-traders/calendario-de-negociacao/feriados/";
    let mut current = date;
    let mut sources = BTreeSet::new();
    while let Some(next) = current.succ_opt() {
        if next.month() != date.month() {
            return Some(sources.into_iter().collect());
        }
        if !matches!(next.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun) {
            let source = match (next.year(), next.month(), next.day()) {
                (2024, 3, 29) | (2024, 12, 31) => B3_2024_CALENDAR,
                _ => return None,
            };
            sources.insert(source);
        }
        current = next;
    }
    None
}

fn parse_date(value: &str) -> Option<NaiveDate> {
    if value.len() != 10
        || value.as_bytes().get(4) != Some(&b'-')
        || value.as_bytes().get(7) != Some(&b'-')
        || !value
            .bytes()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
    {
        return None;
    }
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .filter(|date| date.year() > 0)
}

fn invalid_range() -> CoreError {
    CoreError::Validation(
        "Informe um intervalo de até 1200 meses, sem meses futuros, para os rendimentos.".into(),
    )
}
