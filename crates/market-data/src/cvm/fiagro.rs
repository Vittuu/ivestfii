//! FIAGRO has its own monthly class-level archive. It must never be parsed as
//! an FII report or joined to a historical canceled FII with the same CNPJ.
use super::*;
use chrono::Months;

pub(super) const SOURCE: &str = "CVM · Informe Mensal de FIAGRO";
const CONFLICTING_REVISION: &str = "conflicting_revision";
const PREFIX: &str =
    "https://dados.cvm.gov.br/dados/FIAGRO/DOC/INF_MENSAL/DADOS/inf_mensal_fiagro_";

pub(super) fn is_type(value: &str) -> bool {
    matches!(value, "FIAGRO" | "Classes de Cotas de Fundos FIAGRO")
}

fn month_date(month: &str) -> Result<NaiveDate, String> {
    if month.len() != 6 || !month.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("Competência do arquivo FIAGRO inválida.".into());
    }
    let date = NaiveDate::parse_from_str(&format!("{month}01"), "%Y%m%d")
        .map_err(|_| "Competência do arquivo FIAGRO inválida.".to_string())?;
    if !(2025..=2100).contains(&date.year()) || date.format("%Y%m").to_string() != month {
        return Err("Competência FIAGRO fora da série pública suportada.".into());
    }
    Ok(date)
}

pub(super) fn url(month: &str) -> String {
    format!("{PREFIX}{month}.zip")
}

pub(super) fn valid_url(value: &str) -> bool {
    value
        .strip_prefix(PREFIX)
        .and_then(|tail| tail.strip_suffix(".zip"))
        .is_some_and(|month| month_date(month).is_ok())
}

pub(super) fn recent_months(at: DateTime<Utc>, count: u32) -> Vec<String> {
    let today = brasilia_date(at);
    let first = today.with_day(1).expect("first day of a valid month");
    (1..=count.min(12))
        .filter_map(|offset| first.checked_sub_months(Months::new(offset)))
        .filter(|date| date.year() >= 2025)
        .map(|date| date.format("%Y%m").to_string())
        .collect()
}

pub(super) fn parse(bytes: &[u8], month: &str, at: DateTime<Utc>) -> Result<AnnualReports, String> {
    let expected_reference = month_date(month)?.format("%Y-%m-%d").to_string();
    date(&expected_reference, at)?;
    let main_name = format!("inf_mensal_fiagro_{month}.csv");
    let sub_name = format!("inf_mensal_fiagro_subclasse_{month}.csv");
    let members = zip_members(bytes, &[main_name.clone(), sub_name.clone()])?;
    let main = Table::read(
        &members[&main_name],
        &[
            "CNPJ_Classe",
            "Nome_Classe",
            "Data_Referencia",
            "Data_Entrega",
            "Versao",
            "Classe_Unica",
            "Numero_Cotistas",
            "Patrimonio_Liquido",
            "Cotas_Emitidas",
            "Valor_Patrimonial_Cotas",
        ],
    )?;
    let subclasses = Table::read(
        &members[&sub_name],
        &[
            "CNPJ_Classe",
            "Nome_Classe",
            "Data_Referencia",
            "Nome_Subclasse",
            "Numero_Cotas",
            "Valor_Patrimonial_Cota",
        ],
    )?;
    let mut reports: BTreeMap<String, StoredReport> = BTreeMap::new();
    let mut ambiguous = BTreeSet::new();
    for row in &main.rows {
        let cnpj = normalize_cnpj(main.get(row, "CNPJ_Classe"))?;
        let reference = date(main.get(row, "Data_Referencia"), at)?;
        if reference != expected_reference {
            return Err("Competência FIAGRO não corresponde ao arquivo mensal.".into());
        }
        let version: u32 = main
            .get(row, "Versao")
            .parse()
            .map_err(|_| "Versão do informe FIAGRO inválida.".to_string())?;
        if version == 0 || version > 255 || !matches!(main.get(row, "Classe_Unica"), "S" | "N") {
            return Err("Versão ou identificação de classe FIAGRO inválida.".into());
        }
        let name = main.get(row, "Nome_Classe");
        if name.len() > 1024 {
            return Err("Nome da classe FIAGRO inválido.".into());
        }
        let filed = main.get(row, "Data_Entrega");
        let filed_at = if filed.is_empty() {
            None
        } else {
            Some(date(filed, at)?)
        };
        if filed_at.as_ref().is_some_and(|filed| filed < &reference) {
            return Err("Entrega FIAGRO anterior à competência.".into());
        }
        let candidate = StoredReport {
            version,
            report: CvmFundReport {
                fund_type: "FIAGRO".into(),
                cnpj: cnpj.clone(),
                fund_name: name.into(),
                reference_date: reference,
                filed_at,
                net_assets: decimal(main.get(row, "Patrimonio_Liquido"), true)?,
                equity_per_share: decimal(main.get(row, "Valor_Patrimonial_Cotas"), true)?,
                shareholders: integer(main.get(row, "Numero_Cotistas"))?,
                issued_shares: decimal(main.get(row, "Cotas_Emitidas"), false)?,
                source: SOURCE.into(),
                source_url: url(month),
                fetched_at: stamp(at),
            },
        };
        match reports.get(&cnpj) {
            Some(previous) if report_order(previous) > report_order(&candidate) => {}
            Some(previous) if report_order(previous) == report_order(&candidate) => {
                if previous.report != candidate.report {
                    ambiguous.insert(cnpj);
                }
            }
            _ => {
                ambiguous.remove(&cnpj);
                reports.insert(cnpj, candidate);
            }
        }
    }
    let mut rejected = BTreeMap::new();
    for cnpj in ambiguous {
        reports.remove(&cnpj);
        rejected.insert(cnpj, CONFLICTING_REVISION.into());
    }
    // The subclass table has no version or trading symbol. It can establish
    // ambiguity/consistency, but must never supply an arbitrary subclass's VP.
    type SubclassValues = (Option<String>, Option<String>);
    let mut by_class: BTreeMap<String, BTreeMap<String, SubclassValues>> = BTreeMap::new();
    let mut ambiguous_subclasses = BTreeSet::new();
    for row in &subclasses.rows {
        let cnpj = normalize_cnpj(subclasses.get(row, "CNPJ_Classe"))?;
        if date(subclasses.get(row, "Data_Referencia"), at)? != expected_reference {
            return Err(
                "Competência da subclasse FIAGRO não corresponde ao arquivo mensal.".into(),
            );
        }
        let name = subclasses.get(row, "Nome_Subclasse");
        if name.is_empty() || name.len() > 1024 {
            return Err("Identificação da subclasse FIAGRO ausente ou inválida.".into());
        }
        let values = (
            decimal(subclasses.get(row, "Numero_Cotas"), false)?,
            decimal(subclasses.get(row, "Valor_Patrimonial_Cota"), true)?,
        );
        let class = by_class.entry(cnpj.clone()).or_default();
        if let Some(previous) = class.insert(name.into(), values.clone()) {
            if previous != values {
                ambiguous_subclasses.insert(cnpj);
            }
        }
    }
    for (cnpj, stored) in &mut reports {
        match by_class.get(cnpj) {
            Some(subclasses) if subclasses.len() > 1 || ambiguous_subclasses.contains(cnpj) => {
                stored.report.equity_per_share = None;
            }
            Some(subclasses) => {
                let (shares, vp) = subclasses.values().next().expect("nonempty subclass group");
                let diverges = [
                    (&stored.report.issued_shares, shares),
                    (&stored.report.equity_per_share, vp),
                ]
                .into_iter()
                .any(|(main_value, sub_value)| {
                    matches!((main_value, sub_value), (Some(main), Some(sub)) if main != sub)
                });
                if diverges {
                    stored.report.equity_per_share = None;
                }
            }
            None => {
                stored.report.equity_per_share = None;
            }
        }
    }
    Ok(AnnualReports {
        fetched_at: stamp(at),
        reports,
        rejected,
    })
}

pub(super) fn resolve_names(monthly: &mut AnnualReports, register: &Register) {
    monthly
        .rejected
        .retain(|cnpj, _| register.fiagro_funds.contains_key(cnpj));
    monthly.reports.retain(|cnpj, stored| {
        let Some(name) = register.fiagro_funds.get(cnpj) else {
            return false;
        };
        if stored.report.fund_name.is_empty() {
            stored.report.fund_name = name.clone();
        }
        true
    });
}

pub(super) fn validate_monthly(
    month: &str,
    monthly: &AnnualReports,
    now: DateTime<Utc>,
) -> Result<(), String> {
    let reference = month_date(month)?.format("%Y-%m-%d").to_string();
    date(&reference, now)?;
    let fetched = DateTime::parse_from_rfc3339(&monthly.fetched_at)
        .map_err(|_| "Data do arquivo FIAGRO em cache inválida.".to_string())?
        .with_timezone(&Utc);
    if fetched > now + chrono::Duration::minutes(5)
        || monthly.reports.len() + monthly.rejected.len() > 30_000
    {
        return Err("Arquivo FIAGRO em cache inválido.".into());
    }
    date(&reference, fetched)?;
    for (cnpj, reason) in &monthly.rejected {
        if normalize_cnpj(cnpj).as_deref() != Ok(cnpj.as_str())
            || monthly.reports.contains_key(cnpj)
            || reason != CONFLICTING_REVISION
        {
            return Err("Classe FIAGRO rejeitada em cache contém identificação inválida.".into());
        }
    }
    for (cnpj, stored) in &monthly.reports {
        let report = &stored.report;
        if normalize_cnpj(cnpj).as_deref() != Ok(cnpj.as_str())
            || report.cnpj != *cnpj
            || report.fund_type != "FIAGRO"
            || report.reference_date != reference
            || report.fund_name.is_empty()
            || report.fund_name.len() > 1024
            || stored.version == 0
            || stored.version > 255
            || report.source != SOURCE
            || report.source_url != url(month)
            || report.fetched_at != monthly.fetched_at
        {
            return Err("Informe FIAGRO em cache contém origem ou identificação inválida.".into());
        }
        date(&reference, fetched)?;
        if let Some(filed) = &report.filed_at {
            date(filed, fetched)?;
            if filed < &reference {
                return Err("Entrega FIAGRO em cache anterior à competência.".into());
            }
        }
        for value in [&report.net_assets, &report.equity_per_share]
            .into_iter()
            .flatten()
        {
            if decimal(value, true)?.as_ref() != Some(value) {
                return Err("Valor financeiro FIAGRO em cache inválido.".into());
            }
        }
        if let Some(shares) = &report.issued_shares {
            if decimal(shares, false)?.as_ref() != Some(shares) {
                return Err("Quantidade de cotas FIAGRO em cache inválida.".into());
            }
        }
    }
    Ok(())
}
