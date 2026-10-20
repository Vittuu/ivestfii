//! Explicit, local investment preferences, isolated in each wallet database.
//! A missing profile is unknown; holdings never create or classify a profile.
//! Profiles are included in deliberate backups, not in the financial sync queue.

use super::*;

const MAX_PROFILE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct InvestmentProfileInput {
    pub risk_tolerance: String,
    pub short_term_objective: String,
    pub short_term_months: Option<u32>,
    pub long_term_objective: String,
    pub long_term_months: Option<u32>,
    pub liquidity_needs: String,
    pub emergency_reserve_status: String,
    pub monthly_contribution: Option<String>,
    pub knowledge_level: Option<String>,
    pub constraints: String,
    pub notes: String,
}

impl Default for InvestmentProfileInput {
    fn default() -> Self {
        Self {
            risk_tolerance: "unknown".into(),
            short_term_objective: String::new(),
            short_term_months: None,
            long_term_objective: String::new(),
            long_term_months: None,
            liquidity_needs: String::new(),
            emergency_reserve_status: "unknown".into(),
            monthly_contribution: None,
            knowledge_level: None,
            constraints: String::new(),
            notes: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InvestmentProfile {
    pub risk_tolerance: String,
    pub short_term_objective: String,
    pub short_term_months: Option<u32>,
    pub long_term_objective: String,
    pub long_term_months: Option<u32>,
    pub liquidity_needs: String,
    pub emergency_reserve_status: String,
    pub monthly_contribution: Option<String>,
    pub knowledge_level: Option<String>,
    pub constraints: String,
    pub notes: String,
    pub updated_at: String,
}

impl InvestmentProfile {
    fn from_input(input: InvestmentProfileInput, updated_at: String) -> Self {
        Self {
            risk_tolerance: input.risk_tolerance,
            short_term_objective: input.short_term_objective,
            short_term_months: input.short_term_months,
            long_term_objective: input.long_term_objective,
            long_term_months: input.long_term_months,
            liquidity_needs: input.liquidity_needs,
            emergency_reserve_status: input.emergency_reserve_status,
            monthly_contribution: input.monthly_contribution,
            knowledge_level: input.knowledge_level,
            constraints: input.constraints,
            notes: input.notes,
            updated_at,
        }
    }

    fn input(&self) -> InvestmentProfileInput {
        InvestmentProfileInput {
            risk_tolerance: self.risk_tolerance.clone(),
            short_term_objective: self.short_term_objective.clone(),
            short_term_months: self.short_term_months,
            long_term_objective: self.long_term_objective.clone(),
            long_term_months: self.long_term_months,
            liquidity_needs: self.liquidity_needs.clone(),
            emergency_reserve_status: self.emergency_reserve_status.clone(),
            monthly_contribution: self.monthly_contribution.clone(),
            knowledge_level: self.knowledge_level.clone(),
            constraints: self.constraints.clone(),
            notes: self.notes.clone(),
        }
    }
}

impl Repository {
    /// Read-only: None means this wallet's profile has never been supplied.
    pub fn investment_profile(&self) -> Result<Option<InvestmentProfile>> {
        let connection = self.connection()?;
        load(&connection)
    }

    pub fn save_investment_profile(
        &self,
        input: InvestmentProfileInput,
    ) -> Result<InvestmentProfile> {
        let input = validate_input(input)?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let profile = InvestmentProfile::from_input(input, Utc::now().to_rfc3339());
        let payload = serialized(&profile)?;
        tx.execute(
            "INSERT INTO investment_profile(singleton,payload) VALUES(1,?1)
             ON CONFLICT(singleton) DO UPDATE SET payload=excluded.payload",
            [payload],
        )?;
        // Deliberately local: saving preferences neither changes financial
        // records nor adds personal preferences to the sync outbox.
        tx.commit()?;
        Ok(profile)
    }
}

impl WalletCatalog {
    pub fn investment_profile(&self, wallet_id: Option<&str>) -> Result<Option<InvestmentProfile>> {
        self.repository(wallet_id)?.investment_profile()
    }

    pub fn save_investment_profile(
        &self,
        wallet_id: Option<&str>,
        input: InvestmentProfileInput,
    ) -> Result<InvestmentProfile> {
        self.repository(wallet_id)?.save_investment_profile(input)
    }
}

fn bounded_text(value: String, label: &str, limit: usize) -> Result<String> {
    // Check the original length as well, so padding cannot bypass the limit.
    if value.chars().count() > limit || value.contains('\0') {
        return Err(CoreError::Validation(format!(
            "{label} deve ter até {limit} caracteres e não pode conter caracteres nulos."
        )));
    }
    Ok(value.trim().to_owned())
}

fn validate_input(mut input: InvestmentProfileInput) -> Result<InvestmentProfileInput> {
    if !matches!(
        input.risk_tolerance.as_str(),
        "unknown" | "conservative" | "moderate" | "aggressive"
    ) {
        return Err(CoreError::Validation(
            "Perfil de risco inválido. Use desconhecido, conservador, moderado ou arrojado.".into(),
        ));
    }
    if !matches!(
        input.emergency_reserve_status.as_str(),
        "unknown" | "not_started" | "partial" | "complete"
    ) {
        return Err(CoreError::Validation(
            "Situação da reserva de emergência inválida.".into(),
        ));
    }
    if input
        .knowledge_level
        .as_deref()
        .is_some_and(|level| !matches!(level, "beginner" | "intermediate" | "advanced"))
    {
        return Err(CoreError::Validation("Nível de conhecimento inválido. Use iniciante, intermediário, avançado ou não informado.".into()));
    }
    for months in [input.short_term_months, input.long_term_months]
        .into_iter()
        .flatten()
    {
        if !(1..=1200).contains(&months) {
            return Err(CoreError::Validation(
                "Os horizontes devem ter de 1 a 1.200 meses, ou ficar não informados.".into(),
            ));
        }
    }
    input.short_term_objective =
        bounded_text(input.short_term_objective, "Objetivo de curto prazo", 500)?;
    input.long_term_objective =
        bounded_text(input.long_term_objective, "Objetivo de longo prazo", 500)?;
    input.liquidity_needs = bounded_text(input.liquidity_needs, "Necessidade de liquidez", 1000)?;
    input.constraints = bounded_text(input.constraints, "Restrições de investimento", 1000)?;
    input.notes = bounded_text(input.notes, "Observações do perfil", 4000)?;
    input.monthly_contribution = input
        .monthly_contribution
        .map(|amount| {
            let value = parse_nonnegative(&amount, "Aporte mensal previsto")?;
            if value.normalize().scale() > 2 {
                return Err(CoreError::Validation(
                    "O aporte mensal deve ser informado em reais, com até duas casas decimais."
                        .into(),
                ));
            }
            Ok(decimal_string(value))
        })
        .transpose()?;
    Ok(input)
}

fn validate_profile(profile: &InvestmentProfile) -> Result<()> {
    validate_input(profile.input())?;
    let updated = chrono::DateTime::parse_from_rfc3339(&profile.updated_at).map_err(|_| {
        CoreError::Validation("Data de atualização do perfil de investimento inválida.".into())
    })?;
    if updated > Utc::now() + chrono::Duration::minutes(5) {
        return Err(CoreError::Validation(
            "A atualização do perfil não pode ter uma data futura.".into(),
        ));
    }
    Ok(())
}

fn serialized(profile: &InvestmentProfile) -> Result<String> {
    validate_profile(profile)?;
    let payload = serde_json::to_string(profile)?;
    if payload.len() > MAX_PROFILE_BYTES {
        return Err(CoreError::Validation(
            "O perfil de investimento excede o limite de 64 KiB.".into(),
        ));
    }
    Ok(payload)
}

pub(super) fn load(connection: &Connection) -> Result<Option<InvestmentProfile>> {
    let payload: Option<String> = connection
        .query_row(
            "SELECT payload FROM investment_profile WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    payload
        .map(|payload| {
            if payload.len() > MAX_PROFILE_BYTES {
                return Err(CoreError::Validation(
                    "O perfil de investimento salvo excede o limite de 64 KiB.".into(),
                ));
            }
            let profile: InvestmentProfile = serde_json::from_str(&payload)?;
            validate_profile(&profile)?;
            Ok(profile)
        })
        .transpose()
}

pub(super) fn parse_embedded(bytes: &[u8]) -> Result<Option<InvestmentProfile>> {
    let root: Value =
        serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes))?;
    root.get("investment_profile")
        .filter(|value| !value.is_null())
        .cloned()
        .map(|value| {
            let profile: InvestmentProfile = serde_json::from_value(value)?;
            serialized(&profile)?;
            Ok(profile)
        })
        .transpose()
}

pub(super) fn store_imported(tx: &Transaction<'_>, profile: &InvestmentProfile) -> Result<()> {
    tx.execute(
        "INSERT INTO investment_profile(singleton,payload) VALUES(1,?1)",
        [serialized(profile)?],
    )?;
    Ok(())
}
