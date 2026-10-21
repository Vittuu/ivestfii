//! Existing dated performance components, presented for allocation reasoning.
//! This is context, never an additional valuation or a new portfolio position.
use serde_json::{json, Value};
use ivestfii_core::{contribution_simulation::ContributionClassBase, PortfolioSnapshot};

fn text(value: &Value) -> Option<String> {
    value.as_str().map(str::to_owned)
}

/// Reuses dated current balances, never purchase cost, gross deposits, earnings
/// or the broker's aggregate total. Empty classes are verified from the wallet;
/// unavailable valuations remain unknown instead of being coerced to zero.
pub(crate) fn simulation_bases(snapshot: &PortfolioSnapshot, performance: &Value) -> Vec<ContributionClassBase> {
    let report = &performance["report"];
    let portfolio = &report["portfolio"];
    let no_fund_shares = snapshot.funds.iter().all(|fund| fund.total_cotas == "0")
        && snapshot.broker_position.as_ref().is_none_or(|position| {
            position.listed_funds.holdings.iter().all(|holding| holding.shares == "0")
        });
    let mut fund = ContributionClassBase {
        kind: "fiis".into(), name: "FIIs e fundos listados".into(),
        current_value: text(&report["marketValue"]).or_else(|| no_fund_shares.then(|| "0".into())),
        dates: text(&report["asOf"]).into_iter().collect(),
        issues: vec!["Base: valor de mercado dos fundos do histórico; proventos e custo de compra não são acrescentados ao saldo.".into()],
    };
    if fund.current_value.is_none() {
        fund.issues.push("Falta valor de mercado completo dos fundos na carteira. Confira as cotações e a conciliação com a posição importada; nenhum saldo foi estimado.".into());
    }
    let mut cash = ContributionClassBase {
        kind: "cashPockets".into(), name: "Caixinhas".into(),
        current_value: text(&portfolio["cash"]["balance"]).or_else(|| snapshot.cash_pockets.is_empty().then(|| "0".into())),
        dates: snapshot.cash_pockets.iter().filter(|pocket| pocket.balance != "0").filter_map(|pocket| pocket.balance_date.clone()).collect(),
        issues: vec!["Base: saldos cadastrados nas caixinhas. Depósitos brutos, retiradas e rendimentos projetados não substituem o saldo.".into()],
    };
    if snapshot.cash_pockets.iter().any(|pocket| pocket.balance != "0" && pocket.balance_date.is_none()) {
        // Retaining only the dates of other pockets would falsely date the
        // aggregate. Empty dates keep this entire class explicitly undated.
        cash.dates.clear();
        cash.issues.push("Há caixinha com saldo sem data informada; o saldo agregado permanece sem data.".into());
    }
    let fixed = snapshot.broker_position.as_ref().map(|position| &position.fixed_income);
    let observation = fixed.and_then(|fixed| fixed.latest_observation.as_ref());
    let fixed_empty = fixed.is_none_or(|fixed| fixed.balance == "0" && fixed.latest_observation.as_ref().is_none_or(|observation| observation.balance == "0"));
    let mut post = ContributionClassBase {
        kind: "fixedIncome".into(), name: "Pós-fixado".into(),
        current_value: text(&portfolio["fixedIncome"]["balance"])
            .or_else(|| observation.map(|observation| observation.balance.clone()))
            .or_else(|| fixed_empty.then(|| "0".into())),
        dates: text(&portfolio["fixedIncome"]["referenceDate"])
            .or_else(|| observation.map(|observation| observation.reference_date.clone())).into_iter().collect(),
        issues: vec!["Base: saldo observado do pós-fixado. Seu rendimento já está no saldo e os aportes detalhados não são novas posições.".into()],
    };
    if post.current_value.is_none() {
        // An older broker balance is still a real dated observation, even if
        // its original invested/earnings split has not been identified.
        if let Some(position) = &snapshot.broker_position {
            post.current_value = Some(position.fixed_income.balance.clone());
            post.dates = vec![position.reference_date.clone()];
            post.issues.push("Usado o saldo datado da posição importada; confirme sua atualização antes de interpretar a composição como atual.".into());
        }
    }
    let today = chrono::Utc::now().with_timezone(&chrono::FixedOffset::west_opt(3 * 3600).expect("Brasília offset")).date_naive();
    if snapshot.cash_pockets.iter().any(|pocket| pocket.balance != "0" && pocket.balance_date.as_deref().is_some_and(|value| chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok_and(|date| date > today))) {
        cash.current_value = None;
        cash.issues.push("Há saldo de caixinha com data futura; o agregado não é uma posição disponível hoje.".into());
    }
    for base in [&mut fund, &mut cash, &mut post] {
        if base.dates.iter().any(|value| chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok_and(|date| date > today)) {
            base.current_value = None;
            base.issues.push("A data do saldo é futura; não foi usada como uma posição disponível hoje.".into());
        }
    }
    vec![fund, post, cash]
}

pub(crate) fn allocation_context(performance: &Value) -> Value {
    let report = &performance["report"];
    let portfolio = &report["portfolio"];
    let available = report.is_object() && portfolio.is_object();
    let mut issues = vec![
        "Os pesos de purchaseAnalysis.allocation usam o custo das compras de fundos registradas até asOf; não são pesos atuais de mercado nem a divisão da carteira inteira.",
        "Os componentes abaixo já estão em performance.report e nos totais de report.portfolio. Não some posições importadas, aportes detalhados ou estes valores novamente aos totais.",
        "Cada componente mantém a data de seu saldo. Datas distintas não representam uma posição consolidada no mesmo instante.",
        "A composição entre classes deve usar currentValue com datas comparáveis. invested tem bases distintas: compras de fundos, depósitos brutos das caixinhas (incluindo dinheiro já retirado) e valor aplicado informado no pós-fixado. Não use esses invested como pesos do capital atualmente alocado.",
        "Setores são os rótulos cadastrados pelo usuário. Rótulos genéricos ou ausentes não comprovam diversificação por segmento, emissor, gestor ou risco.",
        "Não há metas de alocação cadastradas neste contexto. Sugestões de metas devem ser apresentadas como cenários propostos e justificados pelo perfil, sem inferir metas pela posição atual.",
    ];
    if !available {
        issues.push("O resultado completo está indisponível. Componentes ausentes permanecem desconhecidos; use apenas as informações locais que puder conferir.");
    }
    let asset_classes = if available {
        json!([
            {"kind":"fiis","name":"Fundos listados do histórico","status":report["status"],"invested":report["invested"],"currentValue":report["marketValue"],"asOf":report["asOf"],"valuePath":"performance.report.marketValue","issues":report["issues"]},
            {"kind":"cashPockets","name":"Caixinhas","status":if portfolio["cash"]["missingBasisCount"].as_u64()==Some(0){"reported"}else{"incomplete"},"invested":portfolio["cash"]["invested"],"investedBasis":"gross-deposits-including-withdrawn-principal","withdrawals":portfolio["cash"]["withdrawals"],"currentValue":portfolio["cash"]["balance"],"asOf":null,"items":portfolio["cash"]["items"],"valuePath":"performance.report.portfolio.cash.balance"},
            {"kind":"fixedIncome","name":portfolio["fixedIncome"]["name"],"status":portfolio["fixedIncome"]["status"],"invested":portfolio["fixedIncome"]["invested"],"currentValue":portfolio["fixedIncome"]["balance"],"asOf":portfolio["fixedIncome"]["referenceDate"],"valuePath":"performance.report.portfolio.fixedIncome.balance","issues":portfolio["fixedIncome"]["issues"]}
        ])
    } else {
        json!([])
    };
    json!({
        "scope":"selected-wallet-only",
        "basis":"reported-components-not-additional-positions",
        "status":if available {"reported"}else{"unavailable"},
        "fundCostAllocationPath":"purchaseAnalysis.allocation",
        "portfolioTotalsPath":"performance.report.portfolio",
        "assetClasses":asset_classes,
        "issues":issues
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_future_cash_balance_is_unknown_even_when_another_pocket_has_no_date() {
        let snapshot: PortfolioSnapshot = serde_json::from_value(json!({
            "funds":[],"totals":{"invested":"0","cotas":"0","dividendsReceived":"0","estimatedMonthlyIncome":"0"},"recordCount":0,
            "cashPockets":[
                {"id":"undated","name":"Sem data","institution":"Teste","balance":"10","cdiPercent":"100","annualCdiRate":null,"rateDate":null,"notes":"","createdAt":"2020-01-01T00:00:00Z","updatedAt":"2020-01-01T00:00:00Z"},
                {"id":"future","name":"Data futura","institution":"Teste","balance":"20","balanceDate":"9999-01-01","cdiPercent":"100","annualCdiRate":null,"rateDate":null,"notes":"","createdAt":"2020-01-01T00:00:00Z","updatedAt":"2020-01-01T00:00:00Z"}
            ]
        })).unwrap();
        let bases = simulation_bases(&snapshot, &json!({"report":{"portfolio":{"cash":{"balance":"30"}}}}));
        let cash = bases.iter().find(|base| base.kind == "cashPockets").unwrap();
        assert!(cash.current_value.is_none());
        assert!(cash.dates.is_empty());
        assert!(cash.issues.iter().any(|issue| issue.contains("data futura")));
    }

    #[test]
    fn empty_wallet_has_known_zero_classes_without_manufactured_dates() {
        let snapshot: PortfolioSnapshot = serde_json::from_value(json!({
            "funds":[],"totals":{"invested":"0","cotas":"0","dividendsReceived":"0","estimatedMonthlyIncome":"0"},"recordCount":0
        })).unwrap();
        let bases = simulation_bases(&snapshot, &Value::Null);
        assert_eq!(bases.len(), 3);
        assert!(bases.iter().all(|base| base.current_value.as_deref() == Some("0") && base.dates.is_empty()));
    }

    #[test]
    fn dated_components_preserve_unknowns_without_recounting_gains_or_broker_total() {
        let performance = json!({"report":{"status":"partial","invested":"100","marketValue":"90","asOf":"2026-10-21","issues":[],"portfolio":{
            "cash":{"invested":null,"balance":"50","withdrawals":null,"missingBasisCount":1,"items":[{"balanceDate":"2026-10-19","balance":"50"}]},
            "fixedIncome":{"status":"available","name":"Pós sintético","invested":"80","balance":"100","referenceDate":"2026-10-20","income":"20","issues":[]}
        }}});
        let before = performance.clone();
        let context = allocation_context(&performance);
        assert_eq!(context["status"], "reported");
        let classes = context["assetClasses"].as_array().unwrap();
        assert_eq!(classes.len(), 3);
        assert_eq!(classes[0]["currentValue"], "90");
        assert_eq!(classes[0]["asOf"], "2026-10-21");
        assert!(classes[1]["invested"].is_null());
        assert_eq!(classes[1]["status"], "incomplete");
        assert!(classes[1]["asOf"].is_null());
        assert_eq!(classes[1]["items"][0]["balanceDate"], "2026-10-19");
        assert_eq!(classes[2]["currentValue"], "100");
        assert_eq!(classes[2]["asOf"], "2026-10-20");
        assert_eq!(performance, before);
        assert!(context.get("total").is_none());
    }

    #[test]
    fn missing_performance_never_creates_zero_valued_classes() {
        let context = allocation_context(&json!({"status":"unavailable","report":null}));
        assert_eq!(context["status"], "unavailable");
        assert_eq!(context["assetClasses"], json!([]));
    }

    #[test]
    fn redeemed_deposits_remain_separate_from_the_capital_currently_allocated() {
        let context = allocation_context(&json!({"report":{"portfolio":{
            "cash":{"invested":"1000","withdrawals":"900","balance":"100","missingBasisCount":0,"items":[]}
        }}}));
        let cash = &context["assetClasses"][1];
        assert_eq!(cash["invested"], "1000");
        assert_eq!(cash["withdrawals"], "900");
        assert_eq!(cash["currentValue"], "100");
        assert_eq!(cash["investedBasis"], "gross-deposits-including-withdrawn-principal");
    }
}
