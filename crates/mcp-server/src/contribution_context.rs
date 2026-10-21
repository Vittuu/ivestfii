//! Existing dated performance components, presented for allocation reasoning.
//! This is context, never an additional valuation or a new portfolio position.
use serde_json::{json, Value};

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
