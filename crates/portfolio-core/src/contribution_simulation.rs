//! Deterministic budget arithmetic over existing reported class balances.
//! No order, forecast, financial write or suggested allocation is produced.

use crate::{CoreError, Result};
use chrono::{Datelike, NaiveDate};
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const KINDS: [&str; 3] = ["fiis", "fixedIncome", "cashPockets"];
const MAX_COEFFICIENT: i128 = 79_228_162_514_264_337_593_543_950_335;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContributionSimulationInput {
    pub amount: String,
    pub allocations: ContributionAllocations,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContributionAllocations {
    pub fiis: String,
    pub fixed_income: String,
    pub cash_pockets: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContributionClassBase {
    pub kind: String,
    pub name: String,
    /// Reported market value or balance, never total deposits or historic cost.
    pub current_value: Option<String>,
    /// Dates of the positive balances represented by this class, when known.
    pub dates: Vec<String>,
    pub issues: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContributionSimulation {
    pub status: String,
    pub scope: String,
    pub budget: String,
    pub total_before: Option<String>,
    pub total_after: Option<String>,
    /// Only a complete known position with the same known holding date.
    pub as_of: Option<String>,
    pub from_date: Option<String>,
    pub to_date: Option<String>,
    pub valuation_basis: String,
    pub rows: Vec<ContributionSimulationRow>,
    pub issues: Vec<String>,
    pub assumptions: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContributionSimulationRow {
    pub kind: String,
    pub name: String,
    pub current_value: Option<String>,
    pub contribution: String,
    pub after_value: Option<String>,
    pub before_percent: Option<String>,
    pub after_percent: Option<String>,
    pub change_percentage_points: Option<String>,
    pub dates: Vec<String>,
    pub issues: Vec<String>,
}

fn invalid(message: &str) -> CoreError {
    CoreError::Validation(message.into())
}

fn text(value: Decimal) -> String {
    value.normalize().to_string()
}

/// Exact, unsigned decimal input. Contributions accept at most two places;
/// reported values keep every source place supported by rust_decimal.
fn decimal(value: &str, max_scale: usize, label: &str) -> Result<Decimal> {
    let bad = || {
        invalid(&format!("{label}: informe um valor decimal não negativo, com ponto e sem separador de milhar (até {max_scale} casas decimais)."))
    };
    let mut parts = value.split('.');
    let whole = parts.next().ok_or_else(bad)?;
    let fraction = parts.next();
    if whole.is_empty()
        || whole.len() > 29
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || parts.next().is_some()
        || fraction.is_some_and(|fraction| {
            fraction.is_empty()
                || fraction.len() > max_scale
                || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return Err(bad());
    }
    Decimal::from_str_exact(value)
        .map(|value| value.normalize())
        .map_err(|_| bad())
}

/// Reject rescaling/overflow instead of silently rounding a reported balance
/// when a contribution or class total is added.
fn add(left: Decimal, right: Decimal) -> Result<Decimal> {
    let mut scale = left.scale().max(right.scale());
    let left = left
        .mantissa()
        .checked_mul(10_i128.pow(scale - left.scale()))
        .ok_or(CoreError::ArithmeticOverflow)?;
    let right = right
        .mantissa()
        .checked_mul(10_i128.pow(scale - right.scale()))
        .ok_or(CoreError::ArithmeticOverflow)?;
    let mut coefficient = left
        .checked_add(right)
        .ok_or(CoreError::ArithmeticOverflow)?;
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

fn percentage(value: Decimal, total: Decimal) -> Result<Option<Decimal>> {
    if total.is_zero() {
        return Ok(None);
    }
    let percentage = value
        .checked_div(total)
        .and_then(|ratio| ratio.checked_mul(Decimal::ONE_HUNDRED))
        .ok_or(CoreError::ArithmeticOverflow)?;
    Ok(Some(percentage.round_dp_with_strategy(
        2,
        RoundingStrategy::MidpointAwayFromZero,
    )))
}

fn validate_date(value: &str) -> Result<()> {
    let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
        invalid("Data do saldo inválida; use AAAA-MM-DD sem presumir uma data ausente.")
    })?;
    if value.len() != 10 || parsed.year() < 1 || parsed.format("%Y-%m-%d").to_string() != value {
        return Err(invalid(
            "Data do saldo inválida; use AAAA-MM-DD sem presumir uma data ausente.",
        ));
    }
    Ok(())
}

/// Adds the explicitly supplied class budget to existing current values. Every
/// monetary operation is checked decimal arithmetic; there is no clock, I/O,
/// forecast, order generation or selection of a recommended split.
pub fn simulate_contribution(
    input: &ContributionSimulationInput,
    bases: &[ContributionClassBase],
) -> Result<ContributionSimulation> {
    let amount = decimal(&input.amount, 2, "Valor do aporte")?;
    if amount <= Decimal::ZERO {
        return Err(invalid(
            "O valor total do aporte precisa ser maior que zero.",
        ));
    }
    let contributions = [
        decimal(&input.allocations.fiis, 2, "Aporte em FIIs")?,
        decimal(&input.allocations.fixed_income, 2, "Aporte no pós-fixado")?,
        decimal(&input.allocations.cash_pockets, 2, "Aporte nas caixinhas")?,
    ];
    let allocated = contributions
        .iter()
        .try_fold(Decimal::ZERO, |total, value| add(total, *value))?;
    if allocated != amount {
        return Err(invalid("A soma dos valores destinados às três classes precisa ser exatamente igual ao valor total do aporte; não pode faltar nem exceder o orçamento."));
    }

    let mut selected = BTreeMap::new();
    for base in bases {
        if !KINDS.contains(&base.kind.as_str()) {
            return Err(invalid(
                "Classe desconhecida na simulação. Use somente fiis, fixedIncome e cashPockets.",
            ));
        }
        if selected.insert(base.kind.as_str(), base).is_some() {
            return Err(invalid(
                "Há bases duplicadas para a mesma classe; nenhuma foi escolhida para a simulação.",
            ));
        }
        for date in &base.dates {
            validate_date(date)?;
        }
    }
    if selected.len() != KINDS.len() {
        return Err(invalid("Informe uma base para cada uma das três classes, inclusive valor zero para classe comprovadamente vazia ou valor desconhecido como null."));
    }

    let mut rows = Vec::with_capacity(KINDS.len());
    let mut total_before = Decimal::ZERO;
    let mut complete = true;
    let mut dates = BTreeSet::new();
    let mut undated = false;
    let mut issues = vec![];
    for (kind, contribution) in KINDS.iter().zip(contributions) {
        let base = selected[kind];
        let current = base
            .current_value
            .as_deref()
            .map(|value| decimal(value, 28, "Saldo atual informado"))
            .transpose()?;
        let after = current.map(|value| add(value, contribution)).transpose()?;
        let mut row_issues = base.issues.clone();
        if let Some(current) = current {
            total_before = add(total_before, current)?;
            if current > Decimal::ZERO {
                if base.dates.is_empty() {
                    undated = true;
                    row_issues.push("Este saldo positivo não tem data informada; a simulação não presume uma data nem o atualiza.".into());
                }
                dates.extend(base.dates.iter().cloned());
            }
        } else {
            complete = false;
            undated |= base.dates.is_empty();
            dates.extend(base.dates.iter().cloned());
            row_issues.push("O saldo desta classe é desconhecido. O valor após o aporte e todos os pesos da carteira permanecem indisponíveis; desconhecido não significa zero.".into());
        }
        issues.extend(
            row_issues
                .iter()
                .map(|issue| format!("{}: {issue}", base.name)),
        );
        rows.push(ContributionSimulationRow {
            kind: (*kind).into(),
            name: base.name.clone(),
            current_value: current.map(text),
            contribution: text(contribution),
            after_value: after.map(text),
            before_percent: None,
            after_percent: None,
            change_percentage_points: None,
            dates: base.dates.clone(),
            issues: row_issues,
        });
    }
    let total_after = complete.then(|| add(total_before, amount)).transpose()?;
    if complete {
        let after_total = total_after.expect("complete known total");
        for row in &mut rows {
            let current = decimal(
                row.current_value.as_deref().expect("complete known class"),
                28,
                "Saldo atual informado",
            )?;
            let after = decimal(
                row.after_value.as_deref().expect("complete known class"),
                28,
                "Saldo após o aporte",
            )?;
            let before_percent = percentage(current, total_before)?;
            let after_percent = percentage(after, after_total)?;
            row.before_percent = before_percent.map(text);
            row.after_percent = after_percent.map(text);
            row.change_percentage_points = before_percent
                .zip(after_percent)
                .map(|(before, after)| {
                    after
                        .checked_sub(before)
                        .map(text)
                        .ok_or(CoreError::ArithmeticOverflow)
                })
                .transpose()?;
        }
        if total_before.is_zero() {
            issues.push("Não há saldo positivo anterior. Os pesos anteriores e a variação em pontos percentuais são indefinidos; os pesos após o aporte usam apenas o orçamento informado.".into());
        }
    } else {
        issues.push("Faltam saldos de uma ou mais classes. Nenhum total ou peso foi normalizado somente pelas classes conhecidas.".into());
    }
    let valuation_basis = if undated || dates.is_empty() {
        issues.push("Há saldos sem data suficiente ou não há posição positiva datada. Os valores não foram extrapolados até hoje.".into());
        "undated"
    } else if dates.len() > 1 {
        issues.push("Os saldos possuem datas diferentes. Esta é uma simulação sobre os últimos valores informados, sem uma posição consolidada no mesmo dia e sem atualização de rendimentos entre as datas.".into());
        "mixed-dates"
    } else {
        "same-date"
    };
    let from_date = dates.first().cloned();
    let to_date = dates.last().cloned();
    let as_of = (complete && valuation_basis == "same-date")
        .then(|| from_date.clone())
        .flatten();
    Ok(ContributionSimulation {
        status: if complete { "calculated" } else { "incomplete" }.into(),
        scope: "selected-wallet-reported-current-values-only".into(),
        budget: text(amount), total_before: complete.then(|| text(total_before)), total_after: total_after.map(text), as_of, from_date, to_date, valuation_basis: valuation_basis.into(), rows, issues,
        assumptions: vec![
            "Os valores por classe são escolhas explícitas do solicitante; a ferramenta não recomenda uma divisão nem executa ordens.".into(),
            "As bases são os valores de mercado ou saldos atuais informados. Custos históricos, depósitos brutos, retiradas e rendimentos não são somados novamente ao capital alocado.".into(),
            "O aporte é adicionado em reais a cada classe; não representa quantidade de cotas nem preço de execução.".into(),
            "Nenhum retorno, imposto, taxa, prazo de liquidação ou variação de cotação é simulado. Os saldos e suas datas não são atualizados pela simulação.".into(),
            "Uma classe comprovadamente vazia pode ter saldo zero e nenhuma data; isso não remove a data das outras posições positivas. Um saldo desconhecido permanece null.".into(),
            "Os pesos são arredondados a duas casas e podem não somar exatamente 100%. A variação em pontos percentuais é a diferença entre os pesos apresentados, sem ajuste artificial de nenhuma classe.".into(),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn input(amount: &str, allocations: [&str; 3]) -> ContributionSimulationInput {
        ContributionSimulationInput {
            amount: amount.into(),
            allocations: ContributionAllocations {
                fiis: allocations[0].into(),
                fixed_income: allocations[1].into(),
                cash_pockets: allocations[2].into(),
            },
        }
    }
    fn bases(values: [Option<&str>; 3]) -> Vec<ContributionClassBase> {
        KINDS
            .iter()
            .zip(values)
            .map(|(kind, value)| ContributionClassBase {
                kind: (*kind).into(),
                name: (*kind).into(),
                current_value: value.map(str::to_owned),
                dates: if value.is_some_and(|value| value != "0") {
                    vec!["2026-10-20".into()]
                } else {
                    vec![]
                },
                issues: vec![],
            })
            .collect()
    }

    #[test]
    fn explicit_budget_preserves_value_and_calculates_before_and_after_weights() {
        let original = bases([Some("1000"), Some("500"), Some("0")]);
        let saved = serde_json::to_value(&original).unwrap();
        let result =
            simulate_contribution(&input("300", ["100", "100", "100"]), &original).unwrap();
        assert_eq!(result.status, "calculated");
        assert_eq!(result.budget, "300");
        assert_eq!(result.total_before.as_deref(), Some("1500"));
        assert_eq!(result.total_after.as_deref(), Some("1800"));
        assert_eq!(result.as_of.as_deref(), Some("2026-10-20"));
        assert_eq!(result.valuation_basis, "same-date");
        assert_eq!(
            result
                .rows
                .iter()
                .map(|row| row.kind.as_str())
                .collect::<Vec<_>>(),
            KINDS
        );
        assert_eq!(
            result
                .rows
                .iter()
                .map(|row| row.after_value.as_deref())
                .collect::<Vec<_>>(),
            [Some("1100"), Some("600"), Some("100")]
        );
        assert_eq!(
            result
                .rows
                .iter()
                .map(|row| row.before_percent.as_deref())
                .collect::<Vec<_>>(),
            [Some("66.67"), Some("33.33"), Some("0")]
        );
        assert_eq!(
            result
                .rows
                .iter()
                .map(|row| row.after_percent.as_deref())
                .collect::<Vec<_>>(),
            [Some("61.11"), Some("33.33"), Some("5.56")]
        );
        assert_eq!(
            result
                .rows
                .iter()
                .map(|row| row.change_percentage_points.as_deref())
                .collect::<Vec<_>>(),
            [Some("-5.56"), Some("0"), Some("5.56")]
        );
        let after_sum = result
            .rows
            .iter()
            .try_fold(Decimal::ZERO, |total, row| {
                add(
                    total,
                    decimal(row.after_value.as_deref().unwrap(), 28, "test").unwrap(),
                )
            })
            .unwrap();
        assert_eq!(text(after_sum), result.total_after.unwrap());
        assert_eq!(serde_json::to_value(&original).unwrap(), saved);
    }

    #[test]
    fn rejects_missing_or_excess_budget_and_more_than_two_contribution_places() {
        let base = bases([Some("1000"), Some("500"), Some("0")]);
        for request in [
            input("300", ["100", "100", "99.99"]),
            input("300", ["100", "100", "100.01"]),
            input("300.001", ["100", "100", "100.001"]),
            input("1", ["0.001", "0.999", "0"]),
        ] {
            assert!(matches!(
                simulate_contribution(&request, &base),
                Err(CoreError::Validation(_))
            ));
        }
    }

    #[test]
    fn rejects_zero_negative_and_malformed_budget_or_class_amounts() {
        let base = bases([Some("1000"), Some("500"), Some("0")]);
        for bad in [
            "0", "-1", "+1", "1e2", "NaN", "Infinity", "1,00", ".5", "1.", "", " 10",
        ] {
            assert!(
                matches!(
                    simulate_contribution(&input(bad, ["0", "0", "0"]), &base),
                    Err(CoreError::Validation(_))
                ),
                "{bad}"
            );
        }
        assert!(simulate_contribution(&input("100", ["-1", "101", "0"]), &base).is_err());
    }

    #[test]
    fn gross_deposits_and_redeemed_principal_are_not_a_supported_weighting_base() {
        // A pocket with 1000 of historic deposits and 900 withdrawn has only
        // its explicitly reported current balance of 100 in this API.
        let result = simulate_contribution(
            &input("100", ["0", "0", "100"]),
            &bases([Some("100"), Some("0"), Some("100")]),
        )
        .unwrap();
        assert_eq!(result.total_before.as_deref(), Some("200"));
        assert_eq!(result.rows[2].before_percent.as_deref(), Some("50"));
        assert_eq!(result.rows[2].after_value.as_deref(), Some("200"));
        assert_eq!(result.rows[2].after_percent.as_deref(), Some("66.67"));
        assert!(serde_json::from_value::<ContributionClassBase>(json!({"kind":"cashPockets","name":"Cash","currentValue":"100","dates":[],"issues":[],"deposits":"1000"})).is_err());
    }

    #[test]
    fn mixed_dates_preserve_real_values_and_range_without_inventing_a_common_day() {
        let mut base = bases([Some("1000"), Some("500"), Some("0")]);
        base[0].dates = vec!["2026-10-21".into()];
        base[1].dates = vec!["2026-10-19".into(), "2026-10-20".into()];
        let result = simulate_contribution(&input("300", ["100", "100", "100"]), &base).unwrap();
        assert_eq!(result.status, "calculated");
        assert_eq!(result.valuation_basis, "mixed-dates");
        assert!(result.as_of.is_none());
        assert_eq!(
            (result.from_date.as_deref(), result.to_date.as_deref()),
            (Some("2026-10-19"), Some("2026-10-21"))
        );
        assert_eq!(result.rows[1].dates, base[1].dates);
        assert_eq!(result.rows[0].after_value.as_deref(), Some("1100"));
        assert!(result
            .issues
            .iter()
            .any(|issue| issue.contains("datas diferentes")));
    }

    #[test]
    fn an_undated_positive_balance_blocks_a_common_as_of_but_empty_zero_does_not() {
        let mut base = bases([Some("1000"), Some("500"), Some("0")]);
        base[1].dates.clear();
        let result = simulate_contribution(&input("100", ["100", "0", "0"]), &base).unwrap();
        assert_eq!(result.valuation_basis, "undated");
        assert!(result.as_of.is_none());
        assert_eq!(result.from_date.as_deref(), Some("2026-10-20"));
        base[1].current_value = Some("0".into());
        let result = simulate_contribution(&input("100", ["100", "0", "0"]), &base).unwrap();
        assert_eq!(result.valuation_basis, "same-date");
        assert_eq!(result.as_of.as_deref(), Some("2026-10-20"));
    }

    #[test]
    fn unknown_class_value_blocks_whole_totals_and_every_percentage_without_subset_normalization() {
        let base = bases([Some("1000"), None, Some("0")]);
        let result = simulate_contribution(&input("300", ["100", "100", "100"]), &base).unwrap();
        assert_eq!(result.status, "incomplete");
        assert!(
            result.total_before.is_none() && result.total_after.is_none() && result.as_of.is_none()
        );
        assert_eq!(result.rows[0].after_value.as_deref(), Some("1100"));
        assert!(result.rows[1].current_value.is_none() && result.rows[1].after_value.is_none());
        assert_eq!(result.rows[1].contribution, "100");
        assert_eq!(result.rows[2].after_value.as_deref(), Some("100"));
        assert!(result.rows.iter().all(|row| row.before_percent.is_none()
            && row.after_percent.is_none()
            && row.change_percentage_points.is_none()));
    }

    #[test]
    fn all_empty_known_classes_have_no_prior_weights_and_valid_after_weights() {
        let result = simulate_contribution(
            &input("100", ["25", "25", "50"]),
            &bases([Some("0"), Some("0"), Some("0")]),
        )
        .unwrap();
        assert_eq!(result.status, "calculated");
        assert_eq!(result.valuation_basis, "undated");
        assert!(result.as_of.is_none());
        assert_eq!(result.total_before.as_deref(), Some("0"));
        assert_eq!(result.total_after.as_deref(), Some("100"));
        assert!(result
            .rows
            .iter()
            .all(|row| row.before_percent.is_none() && row.change_percentage_points.is_none()));
        assert_eq!(
            result
                .rows
                .iter()
                .map(|row| row.after_percent.as_deref())
                .collect::<Vec<_>>(),
            [Some("25"), Some("25"), Some("50")]
        );
    }

    #[test]
    fn reported_fractional_values_keep_their_exact_precision_when_adding_cents() {
        let result = simulate_contribution(
            &input("0.01", ["0.01", "0", "0"]),
            &bases([Some("0.1234567890123456789012345678"), Some("0"), Some("0")]),
        )
        .unwrap();
        assert_eq!(
            result.rows[0].after_value.as_deref(),
            Some("0.1334567890123456789012345678")
        );
        assert_eq!(
            result.total_after.as_deref(),
            Some("0.1334567890123456789012345678")
        );
        assert_eq!(result.rows[0].before_percent.as_deref(), Some("100"));
    }

    #[test]
    fn missing_duplicate_unknown_classes_or_bad_dates_are_rejected() {
        let request = input("100", ["100", "0", "0"]);
        let base = bases([Some("1000"), Some("500"), Some("0")]);
        assert!(simulate_contribution(&request, &base[..2]).is_err());
        let mut duplicate = base.clone();
        duplicate[1].kind = "fiis".into();
        assert!(simulate_contribution(&request, &duplicate).is_err());
        let mut unknown = base.clone();
        unknown[1].kind = "stocks".into();
        assert!(simulate_contribution(&request, &unknown).is_err());
        for bad in ["2026-02-30", "2026-1-20", "0000-01-01", "20/10/2026", ""] {
            let mut invalid = base.clone();
            invalid[0].dates = vec![bad.into()];
            assert!(simulate_contribution(&request, &invalid).is_err(), "{bad}");
        }
        assert!(serde_json::from_value::<ContributionSimulationInput>(json!({"amount":"100","allocations":{"fiis":"100","fixedIncome":"0","cashPockets":"0","stocks":"0"}})).is_err());
    }

    #[test]
    fn checked_overflow_never_rounds_or_saturates_a_position() {
        let request = input("1", ["1", "0", "0"]);
        assert!(matches!(
            simulate_contribution(
                &request,
                &bases([Some("79228162514264337593543950335"), Some("0"), Some("0")])
            ),
            Err(CoreError::ArithmeticOverflow)
        ));
        assert!(matches!(
            simulate_contribution(
                &input("0.01", ["0.01", "0", "0"]),
                &bases([Some("79228162514264337593543950335"), Some("0"), Some("0")])
            ),
            Err(CoreError::ArithmeticOverflow)
        ));
        assert!(matches!(
            simulate_contribution(
                &request,
                &bases([Some("79228162514264337593543950334"), Some("2"), Some("0")])
            ),
            Err(CoreError::ArithmeticOverflow)
        ));
    }
}
