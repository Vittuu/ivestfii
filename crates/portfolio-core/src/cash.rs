//! Manually reconciled cash pockets and explicit, gross CDI scenarios.
//!
//! A projection never changes the recorded balance. The annual reference rate
//! belongs to the pocket rather than being an implicit, possibly stale quote.
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CashPocket {
    pub id: String,
    pub name: String,
    pub institution: String,
    pub balance: String,
    /// Cumulative gross deposits and withdrawals, manually reconciled together.
    #[serde(default)]
    pub total_deposits: Option<String>,
    #[serde(default)]
    pub total_withdrawals: Option<String>,
    /// Effective date of the recorded balance, independent of the CDI rate date.
    #[serde(default)]
    pub balance_date: Option<String>,
    /// Percentage of the daily CDI rate; 120 means 120% of CDI.
    pub cdi_percent: String,
    pub annual_cdi_rate: Option<String>,
    pub rate_date: Option<String>,
    #[serde(default = "manual_source")]
    pub rate_source: String,
    #[serde(default)]
    pub rate_source_url: Option<String>,
    #[serde(default)]
    pub rate_fetched_at: Option<String>,
    pub notes: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CashPocketInput {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub institution: String,
    pub balance: String,
    #[serde(default)]
    pub total_deposits: Option<String>,
    #[serde(default)]
    pub total_withdrawals: Option<String>,
    #[serde(default)]
    pub balance_date: Option<String>,
    /// Omitted reconciliation values preserve an existing group. Only this
    /// explicit flag clears it, keeping older clients safe when editing a row.
    #[serde(default)]
    pub clear_reconciliation: bool,
    pub cdi_percent: String,
    #[serde(default)]
    pub annual_cdi_rate: Option<String>,
    #[serde(default)]
    pub rate_date: Option<String>,
    #[serde(default)]
    pub rate_source: Option<String>,
    #[serde(default)]
    pub rate_source_url: Option<String>,
    #[serde(default)]
    pub rate_fetched_at: Option<String>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CashSimulationInput {
    pub pocket_id: String,
    /// Explicit business days; no assumption about an actual holiday calendar.
    pub business_days: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CashSimulation {
    pub pocket_id: String,
    pub starting_balance: String,
    pub ending_balance: String,
    pub gross_earnings: String,
    pub annual_cdi_rate: String,
    pub cdi_percent: String,
    pub business_days: u32,
    pub rate_date: Option<String>,
    pub rate_source: String,
    pub rate_source_url: Option<String>,
    pub rate_fetched_at: Option<String>,
    pub assumptions: Vec<String>,
}

impl Repository {
    pub fn cash_pockets(&self) -> Result<Vec<CashPocket>> {
        let connection = self.connection()?;
        load_cash_pockets(&connection)
    }

    pub fn upsert_cash_pocket(&self, input: CashPocketInput) -> Result<CashPocket> {
        let values = validate_input(&input)?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let now = Utc::now().to_rfc3339();
        let (id, created_at, preserved) = match input.id.filter(|id| !id.trim().is_empty()) {
            Some(id) => {
                type Previous = (String, Option<String>, Option<String>, Option<String>);
                let existing: Option<Previous> = tx
                    .query_row(
                        "SELECT created_at,total_deposits,total_withdrawals,balance_date FROM cash_pockets WHERE id=?1",
                        [&id],
                        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
                    )
                    .optional()?;
                let (created_at, deposits, withdrawals, date) = existing.ok_or_else(|| {
                    CoreError::Validation("Caixinha não encontrada nesta carteira.".into())
                })?;
                let preserved = if values.reconciliation.is_none() && !input.clear_reconciliation {
                    validate_reconciliation(
                        deposits.as_deref(),
                        withdrawals.as_deref(),
                        date.as_deref(),
                        values.balance,
                    )?
                } else {
                    None
                };
                (id, created_at, preserved)
            }
            None => (Uuid::new_v4().to_string(), now.clone(), None),
        };
        let (total_deposits, total_withdrawals, balance_date) = values
            .reconciliation
            .or(preserved)
            .map(|group| {
                (
                    Some(group.deposits),
                    Some(group.withdrawals),
                    Some(group.balance_date),
                )
            })
            .unwrap_or_default();
        let pocket = CashPocket {
            id,
            name: values.name,
            institution: values.institution,
            balance: decimal_string(values.balance),
            total_deposits,
            total_withdrawals,
            balance_date,
            cdi_percent: decimal_string(values.percent),
            annual_cdi_rate: values.rate.map(decimal_string),
            rate_date: values.rate_date,
            rate_source: input.rate_source.unwrap_or_else(manual_source),
            rate_source_url: input.rate_source_url,
            rate_fetched_at: input.rate_fetched_at,
            notes: input.notes.trim().to_string(),
            created_at,
            updated_at: now.clone(),
        };
        tx.execute(
            "INSERT INTO cash_pockets(id,name,institution,balance,cdi_percent,annual_cdi_rate,rate_date,notes,created_at,updated_at,rate_source,rate_source_url,rate_fetched_at,total_deposits,total_withdrawals,balance_date)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
             ON CONFLICT(id) DO UPDATE SET name=excluded.name,institution=excluded.institution,
             balance=excluded.balance,cdi_percent=excluded.cdi_percent,annual_cdi_rate=excluded.annual_cdi_rate,
             rate_date=excluded.rate_date,notes=excluded.notes,rate_source=excluded.rate_source,rate_source_url=excluded.rate_source_url,rate_fetched_at=excluded.rate_fetched_at,total_deposits=excluded.total_deposits,total_withdrawals=excluded.total_withdrawals,balance_date=excluded.balance_date,revision=cash_pockets.revision+1,updated_at=excluded.updated_at",
            params![pocket.id,pocket.name,pocket.institution,pocket.balance,pocket.cdi_percent,
                pocket.annual_cdi_rate,pocket.rate_date,pocket.notes,pocket.created_at,pocket.updated_at,pocket.rate_source,pocket.rate_source_url,pocket.rate_fetched_at,pocket.total_deposits,pocket.total_withdrawals,pocket.balance_date],
        )?;
        // Validate aggregate totals before committing either the row or outbox.
        snapshot_connection(&tx)?;
        let revision: i64 = tx.query_row(
            "SELECT revision FROM cash_pockets WHERE id=?1",
            [&pocket.id],
            |row| row.get(0),
        )?;
        let mut payload = serde_json::to_value(&pocket)?;
        payload["revision"] = json!(revision);
        enqueue(&tx, "cashPocket", &pocket.id, &payload, &now)?;
        tx.commit()?;
        Ok(pocket)
    }

    pub fn simulate_cash_pocket(&self, input: CashSimulationInput) -> Result<CashSimulation> {
        if input.business_days > 2_520 {
            return Err(CoreError::Validation(
                "A simulação aceita de 0 a 2.520 dias úteis (dez anos na base 252).".into(),
            ));
        }
        let pocket = self
            .cash_pockets()?
            .into_iter()
            .find(|pocket| pocket.id == input.pocket_id)
            .ok_or_else(|| {
                CoreError::Validation("Caixinha não encontrada nesta carteira.".into())
            })?;
        let annual_rate = pocket.annual_cdi_rate.as_deref().ok_or_else(|| {
            CoreError::Validation(
                "Informe a taxa anual do CDI e sua data de referência para simular esta caixinha."
                    .into(),
            )
        })?;
        let rate = parse_annual_rate(annual_rate)?;
        let percent = parse_cdi_percent(&pocket.cdi_percent)?;
        let starting_balance = parse_nonnegative(&pocket.balance, "Saldo da caixinha")?;
        // The product's percentage applies to the DAILY CDI return, not to the
        // annual rate. All roots, powers and balances stay in Decimal arithmetic.
        let daily_cdi = daily_root(add(Decimal::ONE, div(rate, Decimal::ONE_HUNDRED)?)?)?
            .checked_sub(Decimal::ONE)
            .ok_or(CoreError::ArithmeticOverflow)?;
        let daily_factor = add(
            Decimal::ONE,
            mul(daily_cdi, div(percent, Decimal::ONE_HUNDRED)?)?,
        )?;
        let accumulated = checked_power(daily_factor, input.business_days)?;
        let ending_balance = money(mul(starting_balance, accumulated)?);
        let earnings = ending_balance
            .checked_sub(starting_balance)
            .ok_or(CoreError::ArithmeticOverflow)?;
        Ok(CashSimulation {
            pocket_id: pocket.id,
            starting_balance: decimal_string(starting_balance),
            ending_balance: decimal_string(ending_balance),
            gross_earnings: decimal_string(earnings),
            annual_cdi_rate: decimal_string(rate),
            cdi_percent: pocket.cdi_percent,
            business_days: input.business_days,
            rate_date: pocket.rate_date,
            rate_source: pocket.rate_source,
            rate_source_url: pocket.rate_source_url,
            rate_fetched_at: pocket.rate_fetched_at,
            assumptions: vec![
                "Taxa anual do CDI fixada no cadastro, informada manualmente ou consultada na fonte indicada, e mantida constante no cenário; não é previsão nem atualização contínua.".into(),
                "Base de 252 dias úteis por ano; o percentual contratado é aplicado à taxa diária equivalente do CDI, com capitalização diária.".into(),
                "Valores brutos, sem IR, IOF, taxas, aportes ou resgates; regras, limites e arredondamentos específicos da instituição não estão incluídos.".into(),
                "O saldo cadastrado é manual. Esta simulação não credita rendimentos nem altera o saldo da caixinha.".into(),
            ],
        })
    }
}

pub(super) fn load_cash_pockets(connection: &Connection) -> Result<Vec<CashPocket>> {
    let mut statement = connection.prepare(
        "SELECT id,name,institution,balance,cdi_percent,annual_cdi_rate,rate_date,notes,created_at,updated_at,rate_source,rate_source_url,rate_fetched_at,total_deposits,total_withdrawals,balance_date
         FROM cash_pockets ORDER BY institution COLLATE NOCASE,name COLLATE NOCASE,id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(CashPocket {
            id: row.get(0)?,
            name: row.get(1)?,
            institution: row.get(2)?,
            balance: row.get(3)?,
            cdi_percent: row.get(4)?,
            annual_cdi_rate: row.get(5)?,
            rate_date: row.get(6)?,
            notes: row.get(7)?,
            created_at: row.get(8)?,
            updated_at: row.get(9)?,
            rate_source: row.get(10)?,
            rate_source_url: row.get(11)?,
            rate_fetched_at: row.get(12)?,
            total_deposits: row.get(13)?,
            total_withdrawals: row.get(14)?,
            balance_date: row.get(15)?,
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

pub(super) fn parse_embedded(bytes: &[u8]) -> Result<Vec<CashPocket>> {
    let root: Value =
        serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes))?;
    let Some(value) = root.get("cash_pockets") else {
        return Ok(Vec::new());
    };
    let pockets: Vec<CashPocket> = serde_json::from_value(value.clone())?;
    let mut ids = HashSet::new();
    for pocket in &pockets {
        let id = Uuid::parse_str(&pocket.id).map_err(|_| {
            CoreError::Validation("Identificador de caixinha inválido no backup.".into())
        })?;
        if !ids.insert(id) {
            return Err(CoreError::Validation(
                "O backup contém identificadores de caixinha duplicados.".into(),
            ));
        }
        let validated = validate_input(&CashPocketInput {
            id: Some(pocket.id.clone()),
            name: pocket.name.clone(),
            institution: pocket.institution.clone(),
            balance: pocket.balance.clone(),
            total_deposits: pocket.total_deposits.clone(),
            total_withdrawals: pocket.total_withdrawals.clone(),
            balance_date: pocket.balance_date.clone(),
            clear_reconciliation: false,
            cdi_percent: pocket.cdi_percent.clone(),
            annual_cdi_rate: pocket.annual_cdi_rate.clone(),
            rate_date: pocket.rate_date.clone(),
            rate_source: Some(pocket.rate_source.clone()),
            rate_source_url: pocket.rate_source_url.clone(),
            rate_fetched_at: pocket.rate_fetched_at.clone(),
            notes: pocket.notes.clone(),
        })?;
        if pocket.annual_cdi_rate.is_some() != validated.rate.is_some()
            || pocket.rate_date.is_some() != validated.rate_date.is_some()
        {
            return Err(CoreError::Validation(
                "Taxa e data do CDI vazias devem ser nulas ou ausentes no backup.".into(),
            ));
        }
        let created_at =
            chrono::DateTime::parse_from_rfc3339(&pocket.created_at).map_err(|_| {
                CoreError::Validation("Data de criação da caixinha inválida no backup.".into())
            })?;
        let updated_at =
            chrono::DateTime::parse_from_rfc3339(&pocket.updated_at).map_err(|_| {
                CoreError::Validation("Data de atualização da caixinha inválida no backup.".into())
            })?;
        if updated_at < created_at {
            return Err(CoreError::Validation(
                "A atualização da caixinha não pode anteceder sua criação.".into(),
            ));
        }
    }
    Ok(pockets)
}

pub(super) fn store_imported(
    tx: &Transaction<'_>,
    pockets: &[CashPocket],
    now: &str,
) -> Result<()> {
    for pocket in pockets {
        tx.execute(
            "INSERT INTO cash_pockets(id,name,institution,balance,cdi_percent,annual_cdi_rate,rate_date,notes,created_at,updated_at,rate_source,rate_source_url,rate_fetched_at,total_deposits,total_withdrawals,balance_date)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
            params![pocket.id,pocket.name,pocket.institution,pocket.balance,pocket.cdi_percent,
                pocket.annual_cdi_rate,pocket.rate_date,pocket.notes,pocket.created_at,pocket.updated_at,pocket.rate_source,pocket.rate_source_url,pocket.rate_fetched_at,pocket.total_deposits,pocket.total_withdrawals,pocket.balance_date],
        )?;
        let mut payload = serde_json::to_value(pocket)?;
        payload["revision"] = json!(1);
        enqueue(tx, "cashPocket", &pocket.id, &payload, now)?;
    }
    Ok(())
}

struct ValidatedInput {
    name: String,
    institution: String,
    balance: Decimal,
    percent: Decimal,
    rate: Option<Decimal>,
    rate_date: Option<String>,
    reconciliation: Option<Reconciliation>,
}

struct Reconciliation {
    deposits: String,
    withdrawals: String,
    balance_date: String,
}

fn validate_reconciliation(
    deposits: Option<&str>,
    withdrawals: Option<&str>,
    balance_date: Option<&str>,
    balance: Decimal,
) -> Result<Option<Reconciliation>> {
    let (deposits,withdrawals,balance_date) = match (deposits,withdrawals,balance_date) {
        (None,None,None) => return Ok(None),
        (Some(deposits),Some(withdrawals),Some(date)) => (deposits,withdrawals,date),
        _ => return Err(CoreError::Validation("Informe aportes acumulados, resgates acumulados e data do saldo juntos, ou deixe os três ausentes.".into())),
    };
    let deposits = parse_nonnegative(deposits, "Aportes acumulados da caixinha")?;
    let withdrawals = parse_nonnegative(withdrawals, "Resgates acumulados da caixinha")?;
    if deposits.normalize().scale() > 2 || withdrawals.normalize().scale() > 2 {
        return Err(CoreError::Validation(
            "Aportes e resgates devem ser informados em reais, com até duas casas decimais.".into(),
        ));
    }
    let date = NaiveDate::parse_from_str(balance_date, "%Y-%m-%d")
        .map_err(|_| CoreError::Validation("Data efetiva do saldo da caixinha inválida.".into()))?;
    let today = Utc::now()
        .with_timezone(&chrono::FixedOffset::west_opt(3 * 3600).expect("Brasilia UTC-3"))
        .date_naive();
    if date.year() < 1 || date.format("%Y-%m-%d").to_string() != balance_date || date > today {
        return Err(CoreError::Validation(
            "Informe a data efetiva do saldo no formato AAAA-MM-DD, sem uma data futura.".into(),
        ));
    }
    // Fail before saving a reconciliation whose future result cannot be
    // represented exactly. Negative earnings remain a valid, recorded loss.
    add(balance, withdrawals)?
        .checked_sub(deposits)
        .ok_or(CoreError::ArithmeticOverflow)?;
    Ok(Some(Reconciliation {
        deposits: decimal_string(deposits),
        withdrawals: decimal_string(withdrawals),
        balance_date: balance_date.into(),
    }))
}

fn validate_input(input: &CashPocketInput) -> Result<ValidatedInput> {
    let name = required_label(&input.name, "Nome da caixinha")?;
    let institution = required_label(&input.institution, "Instituição")?;
    if input.notes.chars().count() > 4_000 {
        return Err(CoreError::Validation(
            "As observações da caixinha devem ter até 4.000 caracteres.".into(),
        ));
    }
    let balance = parse_nonnegative(&input.balance, "Saldo da caixinha")?;
    if balance.normalize().scale() > 2 {
        return Err(CoreError::Validation(
            "O saldo deve ser informado em reais, com até duas casas decimais.".into(),
        ));
    }
    if input.clear_reconciliation
        && (input.total_deposits.is_some()
            || input.total_withdrawals.is_some()
            || input.balance_date.is_some())
    {
        return Err(CoreError::Validation(
            "Não é possível limpar a reconciliação e informar seus valores na mesma operação."
                .into(),
        ));
    }
    let reconciliation = validate_reconciliation(
        input.total_deposits.as_deref(),
        input.total_withdrawals.as_deref(),
        input.balance_date.as_deref(),
        balance,
    )?;
    let percent = parse_cdi_percent(&input.cdi_percent)?;
    let rate = input
        .annual_cdi_rate
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(parse_annual_rate)
        .transpose()?;
    let rate_date = input
        .rate_date
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| {
            let trimmed = value.trim();
            let date = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
                .map_err(|_| CoreError::Validation("Data de referência do CDI inválida.".into()))?;
            if date.format("%Y-%m-%d").to_string() != trimmed {
                return Err(CoreError::Validation(
                    "Informe a data de referência do CDI no formato AAAA-MM-DD.".into(),
                ));
            }
            Ok(trimmed.to_string())
        })
        .transpose()?;
    if rate.is_some() != rate_date.is_some() {
        return Err(CoreError::Validation(
            "Informe a taxa anual do CDI junto com sua data de referência, ou deixe ambos vazios."
                .into(),
        ));
    }
    validate_provenance(input, rate_date.as_deref())?;
    Ok(ValidatedInput {
        name,
        institution,
        balance,
        percent,
        rate,
        rate_date,
        reconciliation,
    })
}

fn manual_source() -> String {
    "manual".into()
}

fn validate_provenance(input: &CashPocketInput, reference_date: Option<&str>) -> Result<()> {
    match input.rate_source.as_deref().unwrap_or("manual") {
        "manual" if input.rate_source_url.is_none() && input.rate_fetched_at.is_none() => Ok(()),
        "bcb-sgs" => {
            const CDI_SOURCE: &str =
                "https://api.bcb.gov.br/dados/serie/bcdata.sgs.4389/dados/ultimos/1?formato=json";
            if input.rate_source_url.as_deref() != Some(CDI_SOURCE) || reference_date.is_none() {
                return Err(CoreError::Validation(
                    "A referência BCB deve identificar a série oficial do CDI, sua taxa e data."
                        .into(),
                ));
            }
            let fetched = chrono::DateTime::parse_from_rfc3339(
                input.rate_fetched_at.as_deref().ok_or_else(|| {
                    CoreError::Validation("Informe quando o CDI foi consultado na fonte.".into())
                })?,
            )
            .map_err(|_| CoreError::Validation("Horário da consulta do CDI inválido.".into()))?;
            let reference = NaiveDate::parse_from_str(reference_date.unwrap(), "%Y-%m-%d")
                .map_err(|_| CoreError::Validation("Data do CDI inválida.".into()))?;
            if fetched.date_naive() < reference
                || fetched > Utc::now() + chrono::Duration::minutes(5)
            {
                return Err(CoreError::Validation(
                    "O horário da consulta não corresponde à referência do CDI.".into(),
                ));
            }
            Ok(())
        }
        _ => Err(CoreError::Validation(
            "Fonte da taxa inválida. Use entrada manual ou a consulta oficial do CDI.".into(),
        )),
    }
}

fn required_label(value: &str, label: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 120 {
        return Err(CoreError::Validation(format!(
            "{label} deve ter entre 1 e 120 caracteres."
        )));
    }
    Ok(value.to_string())
}

fn parse_cdi_percent(value: &str) -> Result<Decimal> {
    let percent = parse_nonnegative(value, "Percentual do CDI")?;
    if percent <= Decimal::ZERO || percent > Decimal::from(1_000) || percent.normalize().scale() > 4
    {
        return Err(CoreError::Validation(
            "O percentual do CDI deve ser maior que zero, de até 1.000%, com até quatro casas decimais.".into(),
        ));
    }
    Ok(percent)
}

fn parse_annual_rate(value: &str) -> Result<Decimal> {
    let rate = parse_nonnegative(value, "Taxa anual do CDI")?;
    if rate > Decimal::ONE_HUNDRED || rate.normalize().scale() > 8 {
        return Err(CoreError::Validation(
            "A taxa anual do CDI deve estar entre 0% e 100%, com até oito casas decimais.".into(),
        ));
    }
    Ok(rate)
}

fn checked_power(mut base: Decimal, mut exponent: u32) -> Result<Decimal> {
    let mut product = Decimal::ONE;
    while exponent != 0 {
        if exponent & 1 == 1 {
            product = mul(product, base)?;
        }
        exponent >>= 1;
        if exponent != 0 {
            base = mul(base, base)?;
        }
    }
    Ok(product)
}

/// Decimal bisection suffices for a target in [1, 2]. Comparisons stop as soon
/// as the product exceeds the target, so upper bracket powers cannot overflow.
fn daily_root(target: Decimal) -> Result<Decimal> {
    if target == Decimal::ONE {
        return Ok(Decimal::ONE);
    }
    let mut low = Decimal::ONE;
    let mut high = Decimal::TWO;
    for _ in 0..100 {
        let midpoint = div(add(low, high)?, Decimal::TWO)?;
        if midpoint == low || midpoint == high {
            break;
        }
        let mut power = Decimal::ONE;
        for _ in 0..252 {
            power = mul(power, midpoint)?;
            if power > target {
                break;
            }
        }
        if power > target {
            high = midpoint;
        } else {
            low = midpoint;
        }
    }
    div(add(low, high)?, Decimal::TWO)
}
