//! Local, dated quote eligibility for whole-share contribution scenarios.
use chrono::{DateTime, Duration, FixedOffset, Utc};
use ivestfii_core::{fund_contribution::FundContributionBase, PortfolioSnapshot};
use ivestfii_market_data::{purchase_analysis::build_purchase_analysis, quotes::QuoteItem, QuotePanel};
use serde_json::Value;

const CACHE_SECONDS: i64 = 30 * 60;
const MARKET_DAYS: i64 = 7;

fn zero_quantity(value: &str) -> bool {
    let mut parts = value.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next();
    !whole.is_empty()
        && whole.bytes().all(|byte| byte == b'0')
        && parts.next().is_none()
        && fraction
            .is_none_or(|digits| !digits.is_empty() && digits.bytes().all(|byte| byte == b'0'))
}

fn cent_price(value: &str) -> bool {
    let mut parts = value.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next();
    !whole.is_empty()
        && whole.len() <= 29
        && whole.bytes().all(|byte| byte.is_ascii_digit())
        && parts.next().is_none()
        && fraction.is_none_or(|fraction| {
            !fraction.is_empty()
                && fraction.len() <= 2
                && fraction.bytes().all(|byte| byte.is_ascii_digit())
        })
        && value
            .bytes()
            .any(|byte| byte.is_ascii_digit() && byte != b'0')
}

fn quote_basis(
    item: Option<&QuoteItem>,
    symbol: &str,
    now: DateTime<Utc>,
) -> (Option<String>, Option<String>, String, Vec<String>) {
    let Some(item) = item else {
        return (
            None,
            None,
            "missing".into(),
            vec!["Falta cotação deste fundo no cache local.".into()],
        );
    };
    let Some(observation) = &item.observation else {
        return (
            None,
            None,
            "missing".into(),
            vec![item
                .error
                .clone()
                .unwrap_or_else(|| "Falta cotação deste fundo no cache local.".into())],
        );
    };
    let quoted = observation
        .quoted_at
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|date| date.with_timezone(&Utc));
    let fetched = DateTime::parse_from_rfc3339(&observation.fetched_at)
        .ok()
        .map(|date| date.with_timezone(&Utc));
    let date = quoted.map(|date| {
        date.with_timezone(&FixedOffset::west_opt(3 * 3600).expect("Brasília offset"))
            .date_naive()
            .to_string()
    });
    let invalid = |message: &str| (None, date.clone(), "invalid".into(), vec![message.into()]);
    if item.symbol.to_uppercase() != symbol
        || observation.symbol.to_uppercase() != symbol
        || observation.currency != "BRL"
    {
        return invalid(
            "A cotação não corresponde ao mesmo fundo em reais; nenhum preço foi substituído.",
        );
    }
    if !cent_price(&observation.price) {
        return invalid("O preço precisa ser positivo e expresso em centavos; preço inválido ou com fração de centavo não foi arredondado.");
    }
    let (Some(quoted), Some(fetched)) = (quoted, fetched) else {
        return invalid("A cotação precisa dos horários originais de mercado e de consulta.");
    };
    if quoted > now || fetched > now || quoted > fetched {
        return invalid(
            "Há horário futuro ou inconsistente na cotação; ele não foi usado no cenário.",
        );
    }
    if now - fetched >= Duration::seconds(CACHE_SECONDS)
        || now - quoted > Duration::days(MARKET_DAYS)
    {
        return (None, date, "stale".into(), vec!["A cotação salva não atende à janela do cenário: consulta há menos de 30 minutos e horário de mercado há no máximo 7 dias. Prepare os dados novamente, respeitando os limites gratuitos; se a consulta falhar, mantenha a lacuna.".into()]);
    }
    let mut issues = vec!["Última cotação pública disponível, com atraso da fonte; não é preço de execução nem garantia de disponibilidade na corretora.".into()];
    if let Some(error) = &item.error {
        issues.push(format!("A fonte também informou: {error}"));
    }
    (
        Some(observation.price.clone()),
        date,
        "usable".into(),
        issues,
    )
}

pub(crate) fn fund_bases(
    snapshot: &PortfolioSnapshot,
    quotes: &QuotePanel,
    now: DateTime<Utc>,
) -> Vec<FundContributionBase> {
    let today = now
        .with_timezone(&FixedOffset::west_opt(3 * 3600).expect("Brasília offset"))
        .date_naive();
    let analysis = build_purchase_analysis(snapshot, quotes, today);
    let broker_basis = snapshot.broker_position.as_ref().and_then(|position| {
        chrono::NaiveDate::parse_from_str(&position.reference_date, "%Y-%m-%d")
            .ok()
            .map(|date| build_purchase_analysis(snapshot, quotes, date))
    });
    let mut bases = Vec::new();
    for row in analysis.rows.iter().filter(|row| row.fund_id.is_some()) {
        let (price, quote_date, mut quote_status, mut issues) =
            quote_basis(row.quote.as_ref(), &row.symbol, now);
        issues.extend(row.issues.clone());
        issues.extend(quotes.errors.clone());
        let mut current_shares = row.shares.clone();
        if !row.quote_candidates.is_empty() {
            quote_status = "invalid".into();
        }
        if let Some(quote_date) = &quote_date {
            let purchases_after_quote = snapshot
                .funds
                .iter()
                .filter(|fund| Some(&fund.id) == row.fund_id.as_ref())
                .flat_map(|fund| &fund.entries)
                .any(|entry| {
                    entry.cotas_added != "0"
                        && entry.month <= today.format("%Y-%m").to_string()
                        && entry
                            .purchase_date
                            .as_deref()
                            .map_or(entry.month > quote_date[..7].to_string(), |date| {
                                date > quote_date.as_str() && date <= today.to_string().as_str()
                            })
                });
            if purchases_after_quote {
                current_shares = None;
                issues.push("Há compras informadas depois do horário de mercado desta cotação; as cotas e os pesos anteriores não foram presumidos.".into());
            }
            let ambiguous_purchase = snapshot
                .funds
                .iter()
                .filter(|fund| Some(&fund.id) == row.fund_id.as_ref())
                .flat_map(|fund| &fund.entries)
                .any(|entry| {
                    entry.cotas_added != "0"
                        && entry.purchase_date.is_none()
                        && entry.month == quote_date[..7]
                });
            if ambiguous_purchase {
                current_shares = None;
                issues.push("Há compra sem dia informado no mesmo mês da cotação; não foi possível confirmar as cotas nesse corte. Informe a data da compra para calcular os pesos.".into());
            }
        }
        if let Some(position) = &snapshot.broker_position {
            let anchor = broker_basis
                .as_ref()
                .and_then(|basis| {
                    basis
                        .rows
                        .iter()
                        .find(|anchor| anchor.fund_id == row.fund_id)
                })
                .and_then(|anchor| anchor.shares.as_deref());
            let reported = position
                .listed_funds
                .holdings
                .iter()
                .find(|holding| holding.ticker.eq_ignore_ascii_case(&row.symbol))
                .map_or("0", |holding| holding.shares.as_str());
            if anchor != Some(reported)
                || position.reference_date > today.to_string()
                || quote_date
                    .as_ref()
                    .is_some_and(|date| position.reference_date > *date)
            {
                current_shares = None;
                issues.push("As cotas do histórico não estão conciliadas com a posição da corretora na data dela, ou a posição é posterior à cotação. Nenhuma quantidade anterior foi escolhida como correta.".into());
            }
        }
        bases.push(FundContributionBase {
            fund_id: row.fund_id.clone(),
            symbol: row.symbol.clone(),
            name: row.name.clone().unwrap_or_else(|| row.symbol.clone()),
            declared_sector: row.declared_sector.clone().unwrap_or_default(),
            classification_known: row.classification_known,
            current_shares,
            price,
            quote: row
                .quote
                .as_ref()
                .and_then(|quote| serde_json::to_value(quote).ok())
                .unwrap_or(Value::Null),
            quote_status,
            quote_date,
            issues,
        });
    }
    if let Some(position) = &snapshot.broker_position {
        for holding in &position.listed_funds.holdings {
            if zero_quantity(&holding.shares) {
                continue;
            }
            let symbol = holding.ticker.to_uppercase();
            if bases.iter().any(|base| base.symbol == symbol) {
                continue;
            }
            let item = quotes
                .items
                .iter()
                .find(|item| item.symbol.to_uppercase() == symbol);
            let (price, quote_date, quote_status, mut issues) = quote_basis(item, &symbol, now);
            issues.push("Fundo da posição importada sem histórico reconciliado de compras nesta carteira; não foram presumidas cotas atuais nem pesos.".into());
            bases.push(FundContributionBase {
                fund_id: None,
                symbol: symbol.clone(),
                name: symbol,
                declared_sector: String::new(),
                classification_known: false,
                current_shares: None,
                price,
                quote: item
                    .and_then(|item| serde_json::to_value(item).ok())
                    .unwrap_or(Value::Null),
                quote_status,
                quote_date,
                issues,
            });
        }
    }
    bases
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-21T20:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }
    fn quote() -> QuoteItem {
        serde_json::from_value(json!({"symbol":"TEST11","status":"cached","error":null,"observation":{"symbol":"TEST11","price":"9.46","currency":"BRL","quotedAt":"2026-10-21T19:40:00Z","fetchedAt":"2026-10-21T19:50:00Z","source":"brapi","sourceUrl":"https://brapi.dev/api/v2/stocks/quote?symbols=TEST11","delayMinutes":30}})).unwrap()
    }
    fn snapshot() -> PortfolioSnapshot {
        serde_json::from_value(json!({"funds":[{
        "id":"fund-a","ticker":"TEST11","name":"Fundo de teste","sector":"Papel","entries":[{"id":"entry-a","month":"2026-01","purchaseDate":"2026-01-10","cotasAdded":"10","pricePerCota":"10","dividendPerCota":"0","dividendTotal":"0","notes":""}],
        "totalCotas":"10","totalInvested":"100","averagePrice":"10","dividendsReceived":"0","estimatedMonthlyIncome":"0"
    }],"totals":{"invested":"100","cotas":"10","dividendsReceived":"0","estimatedMonthlyIncome":"0"},"recordCount":1})).unwrap()
    }
    fn panel() -> QuotePanel {
        QuotePanel {
            items: vec![quote()],
            ..Default::default()
        }
    }
    fn position(shares: &str) -> ivestfii_core::BrokerPosition {
        serde_json::from_value(json!({
        "referenceDate":"2026-09-20","institution":"Teste","importedAt":"2026-09-21T12:00:00Z","sourcePath":"fixture.json","sourceHash":"a".repeat(64),
        "overview":{"updatedTime":null,"capturedTime":null,"totalWithCash":"100","invested":"100","cash":"0","listedFunds":"100","fixedIncome":"0","pendingIncome":"0"},
        "listedFunds":{"capturedTime":null,"marketValue":"100","costBasis":"100","unrealizedResult":"0","dailyResult":"0","holdings":[{"ticker":"TEST11","shares":shares,"averagePrice":"10","price":"10","marketValue":"100","costBasis":"100","unrealizedResult":"0"}]},
        "fixedIncome":{"name":"Pós","nameTruncated":false,"balance":"0","netBalance":"0","reportedEarnings":"0","return12Months":"0","redemptionConversion":"D+0","redemptionSettlement":"D+0"},"warnings":[]
    })).unwrap()
    }
    #[test]
    fn cached_quotes_preserve_original_times_and_are_not_mistaken_for_a_live_price() {
        let item = quote();
        let before = item.clone();
        let (price, date, status, issues) = quote_basis(Some(&item), "TEST11", now());
        assert_eq!(price.as_deref(), Some("9.46"));
        assert_eq!(date.as_deref(), Some("2026-10-21"));
        assert_eq!(status, "usable");
        assert!(issues
            .iter()
            .any(|issue| issue.contains("preço de execução")));
        assert_eq!(item, before);
    }
    #[test]
    fn stale_market_time_is_not_revalidated_by_a_recent_fetch() {
        let mut item = quote();
        item.observation.as_mut().unwrap().quoted_at = Some("2026-10-01T19:40:00Z".into());
        assert_eq!(quote_basis(Some(&item), "TEST11", now()).2, "stale");
        item = quote();
        item.observation.as_mut().unwrap().quoted_at =
            Some((now() - Duration::days(7)).to_rfc3339());
        assert_eq!(quote_basis(Some(&item), "TEST11", now()).2, "usable");
        item.observation.as_mut().unwrap().quoted_at =
            Some((now() - Duration::days(7) - Duration::seconds(1)).to_rfc3339());
        assert_eq!(quote_basis(Some(&item), "TEST11", now()).2, "stale");
        item = quote();
        item.observation.as_mut().unwrap().fetched_at = "2026-10-21T19:30:00Z".into();
        item.observation.as_mut().unwrap().quoted_at = Some("2026-10-21T19:20:00Z".into());
        assert_eq!(quote_basis(Some(&item), "TEST11", now()).2, "stale");
    }
    #[test]
    fn missing_mismatched_fractional_or_future_quotes_never_supply_a_price() {
        assert!(quote_basis(None, "TEST11", now()).0.is_none());
        for mutation in 0..5 {
            let mut item = quote();
            let observation = item.observation.as_mut().unwrap();
            match mutation {
                0 => observation.currency = "USD".into(),
                1 => observation.symbol = "OTHER11".into(),
                2 => observation.price = "9.461".into(),
                3 => observation.quoted_at = None,
                _ => observation.fetched_at = "2026-10-21T20:01:00Z".into(),
            }
            assert!(quote_basis(Some(&item), "TEST11", now()).0.is_none());
        }
    }
    #[test]
    fn broker_reconciliation_uses_the_original_position_date_not_the_latest_total() {
        let mut snapshot = snapshot();
        snapshot.broker_position = Some(position("55"));
        let result = fund_bases(&snapshot, &panel(), now());
        assert!(result[0].current_shares.is_none());
        assert!(result[0].price.is_some());
        snapshot.broker_position = Some(position("10"));
        let mut later = snapshot.funds[0].entries[0].clone();
        later.id = "later".into();
        later.month = "2026-10".into();
        later.purchase_date = Some("2026-10-01".into());
        later.cotas_added = "45".into();
        snapshot.funds[0].entries.push(later);
        let result = fund_bases(&snapshot, &panel(), now());
        assert_eq!(result[0].current_shares.as_deref(), Some("55"));
    }
    #[test]
    fn unknown_purchase_day_at_quote_cutoff_is_not_assumed_and_future_buys_are_excluded() {
        let mut snapshot = snapshot();
        let mut extra = snapshot.funds[0].entries[0].clone();
        extra.id = "new".into();
        extra.month = "2026-10".into();
        extra.purchase_date = None;
        extra.cotas_added = "5".into();
        snapshot.funds[0].entries.push(extra);
        assert!(fund_bases(&snapshot, &panel(), now())[0]
            .current_shares
            .is_none());
        snapshot.funds[0].entries[1].purchase_date = Some("2026-10-22".into());
        assert_eq!(
            fund_bases(&snapshot, &panel(), now())[0]
                .current_shares
                .as_deref(),
            Some("10")
        );
        snapshot.funds[0].entries[1].purchase_date = Some("2026-10-21".into());
        let mut quotes = panel();
        quotes.items[0].observation.as_mut().unwrap().quoted_at =
            Some("2026-10-20T19:40:00Z".into());
        assert!(fund_bases(&snapshot, &quotes, now())[0]
            .current_shares
            .is_none());
    }
    #[test]
    fn zero_broker_only_holding_does_not_create_an_unknown_position() {
        let mut snapshot = snapshot();
        let mut broker = position("10");
        let mut empty = broker.listed_funds.holdings[0].clone();
        empty.ticker = "EMPTY11".into();
        empty.shares = "0.00".into();
        broker.listed_funds.holdings.push(empty);
        snapshot.broker_position = Some(broker);
        let bases = fund_bases(&snapshot, &panel(), now());
        assert_eq!(bases.len(), 1);
        assert_eq!(bases[0].current_shares.as_deref(), Some("10"));
    }
}
