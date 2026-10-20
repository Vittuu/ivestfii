//! A broker's dated position is independent of the acquisition/dividend ledger.
use super::*;
use chrono::{DateTime, NaiveTime};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerPosition {
    pub reference_date: String,
    pub institution: String,
    pub imported_at: String,
    pub source_path: String,
    pub source_hash: String,
    pub overview: BrokerOverview,
    pub listed_funds: BrokerListedFunds,
    pub fixed_income: BrokerFixedIncome,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerOverview {
    pub updated_time: Option<String>,
    pub captured_time: Option<String>,
    pub total_with_cash: String,
    pub invested: String,
    pub cash: String,
    pub listed_funds: String,
    pub fixed_income: String,
    pub pending_income: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerListedFunds {
    pub captured_time: Option<String>,
    pub market_value: String,
    pub cost_basis: String,
    pub unrealized_result: String,
    pub daily_result: String,
    pub holdings: Vec<BrokerHolding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerHolding {
    pub ticker: String,
    pub shares: String,
    pub average_price: String,
    pub price: String,
    pub market_value: String,
    pub unrealized_result: String,
    pub cost_basis: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerFixedIncome {
    pub name: String,
    pub name_truncated: bool,
    pub balance: String,
    pub net_balance: String,
    pub reported_earnings: String,
    /// Percentage points, not a fractional return and not a monetary amount.
    pub return12_months: String,
    pub redemption_conversion: String,
    pub redemption_settlement: String,
}

impl Repository {
    /// Import a Portuguese transcription or an exported normalized position.
    /// The source is backed up byte for byte before any SQLite mutation.
    /// Repeating any previously imported file never resets a newer position.
    pub fn import_broker_position_file(&self, path: impl AsRef<Path>) -> Result<BrokerPosition> {
        let source_path = fs::canonicalize(path.as_ref())?;
        let bytes = fs::read(&source_path)?;
        let hash = format!("{:x}", Sha256::digest(&bytes));
        let backup_path = backup_source(&self.database_path, &hash, &bytes)?;
        let value: Value =
            serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))?;
        let position = if value.get("broker_position").is_some() {
            parse_normalized(value.get("broker_position").cloned().unwrap())?
        } else if value.get("referenceDate").is_some() {
            parse_normalized(value)?
        } else {
            parse_portuguese(&value, &source_path, &hash)?
        };
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let already_imported: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM broker_position_imports WHERE sha256 = ?1)",
            [&hash],
            |r| r.get(0),
        )?;
        if already_imported {
            if let Some(current) = load_position(&tx)? {
                return Ok(current);
            }
        }
        store_position(&tx, &position)?;
        tx.execute(
            "INSERT OR IGNORE INTO broker_position_imports(sha256,source_path,backup_path,imported_at) VALUES (?1,?2,?3,?4)",
            params![hash, source_path.to_string_lossy(), backup_path.to_string_lossy(), Utc::now().to_rfc3339()],
        )?;
        tx.commit()?;
        Ok(position)
    }
}

fn backup_source(database: &Path, hash: &str, bytes: &[u8]) -> Result<PathBuf> {
    let directory = database
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("backups");
    fs::create_dir_all(&directory)?;
    let path = directory.join(format!("broker-position-{hash}.json"));
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if fs::read(&path)? != bytes {
                return Err(invalid(
                    "O backup da posição não corresponde ao arquivo de origem.",
                ));
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(fs::canonicalize(path)?)
}

pub(super) fn load_position(connection: &Connection) -> Result<Option<BrokerPosition>> {
    let payload: Option<String> = connection
        .query_row(
            "SELECT payload FROM broker_position WHERE singleton = 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    payload
        .map(|value| parse_normalized(serde_json::from_str(&value)?))
        .transpose()
}

pub(super) fn store_position(tx: &Transaction<'_>, position: &BrokerPosition) -> Result<()> {
    tx.execute(
        "INSERT INTO broker_position(singleton,payload) VALUES (1,?1) ON CONFLICT(singleton) DO UPDATE SET payload=excluded.payload",
        [serde_json::to_string(position)?],
    )?;
    Ok(())
}

pub(super) fn parse_embedded(bytes: &[u8]) -> Result<Option<BrokerPosition>> {
    let root: Value =
        serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes))?;
    root.get("broker_position")
        .filter(|value| !value.is_null())
        .cloned()
        .map(parse_normalized)
        .transpose()
}

fn invalid(message: &str) -> CoreError {
    CoreError::Validation(message.into())
}

fn required<'a>(root: &'a Value, path: &str) -> Result<&'a Value> {
    root.pointer(path)
        .ok_or_else(|| invalid(&format!("Campo obrigatório ausente na posição: {path}.")))
}
fn text(root: &Value, path: &str) -> Result<String> {
    required(root, path)?
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| invalid(&format!("Texto inválido na posição: {path}.")))
}
fn numeric(root: &Value, path: &str) -> Result<String> {
    let value = required(root, path)?;
    let raw = match value {
        Value::String(raw) => raw.clone(),
        Value::Number(raw) => raw.to_string(),
        _ => return Err(invalid(&format!("Número inválido na posição: {path}."))),
    };
    Ok(decimal_string(parse_decimal(&raw)?))
}
fn optional_time(root: &Value, path: &str) -> Result<Option<String>> {
    match root.pointer(path) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(time)) => {
            validate_time(time)?;
            Ok(Some(time.clone()))
        }
        _ => Err(invalid("Horário inválido na posição.")),
    }
}
fn array<'a>(root: &'a Value, path: &str) -> Result<&'a Vec<Value>> {
    required(root, path)?
        .as_array()
        .ok_or_else(|| invalid(&format!("Lista inválida na posição: {path}.")))
}

fn parse_portuguese(root: &Value, source: &Path, hash: &str) -> Result<BrokerPosition> {
    if root.get("versao_formato").and_then(Value::as_u64) != Some(1)
        || text(root, "/moeda")? != "BRL"
    {
        return Err(invalid("Formato de posição ou moeda não suportados."));
    }
    let reference_date = text(root, "/data_referencia")?;
    validate_date(&reference_date)?;
    for path in [
        "/panorama_carteira/data_atualizacao_exibida",
        "/posicao_pos_fixado/data_atualizacao_exibida",
        "/posicao_fundos_listados/data_atualizacao_explicita_na_tela",
    ] {
        if let Some(date) = root.pointer(path).filter(|v| !v.is_null()) {
            let date = date
                .as_str()
                .ok_or_else(|| invalid("Data inválida na posição."))?;
            validate_date(date)?;
            if date != reference_date {
                return Err(invalid("Datas de referência divergentes na posição."));
            }
        }
    }
    let mut classes = HashMap::new();
    for class in array(root, "/panorama_carteira/classes")? {
        let name = text(class, "/nome_exibido")?;
        if classes.insert(name, numeric(class, "/valor")?).is_some() {
            return Err(invalid("Classe duplicada no panorama."));
        }
    }
    if classes.len() != 3 {
        return Err(invalid("O panorama deve identificar suas três classes."));
    }
    let mut class = |name: &str| {
        classes
            .remove(name)
            .ok_or_else(|| invalid("Classe ausente ou não reconhecida no panorama."))
    };
    let overview = BrokerOverview {
        updated_time: optional_time(root, "/panorama_carteira/horario_atualizacao_exibido")?,
        captured_time: optional_time(root, "/panorama_carteira/horario_captura_exibido")?,
        total_with_cash: numeric(
            root,
            "/panorama_carteira/total_investido_mais_conta_investimento",
        )?,
        invested: numeric(root, "/panorama_carteira/total_investido_em_classes")?,
        cash: numeric(root, "/panorama_carteira/saldo_conta_investimento_nacional")?,
        listed_funds: class("Fundos Listados")?,
        fixed_income: class("Pós-Fixado")?,
        pending_income: class("Proventos")?,
    };
    let mut holdings = Vec::new();
    for item in array(root, "/posicao_fundos_listados/ativos")? {
        holdings.push(BrokerHolding {
            ticker: text(item, "/ticker")?,
            shares: numeric(item, "/quantidade_cotas")?,
            average_price: numeric(item, "/preco_medio_exibido")?,
            price: numeric(item, "/preco_atual_exibido")?,
            market_value: numeric(item, "/posicao_exibida")?,
            unrealized_result: numeric(item, "/resultado_total_exibido")?,
            cost_basis: numeric(item, "/custo_total_inferido")?,
        });
    }
    let listed_funds = BrokerListedFunds {
        captured_time: optional_time(root, "/posicao_fundos_listados/horario_captura_exibido")?,
        market_value: numeric(root, "/posicao_fundos_listados/total_em_bolsa_exibido")?,
        cost_basis: numeric(root, "/posicao_fundos_listados/valor_aplicado_exibido")?,
        unrealized_result: numeric(root, "/posicao_fundos_listados/resultado_total_exibido")?,
        daily_result: numeric(root, "/posicao_fundos_listados/resultado_do_dia_exibido")?,
        holdings,
    };
    let fixed = required(root, "/posicao_pos_fixado/ativo")?;
    let fixed_income = BrokerFixedIncome {
        name: text(fixed, "/nome_exibido")?,
        name_truncated: required(fixed, "/nome_exibido_esta_truncado")?
            .as_bool()
            .ok_or_else(|| invalid("Indicador de nome truncado inválido."))?,
        balance: numeric(fixed, "/saldo")?,
        net_balance: numeric(fixed, "/saldo_liquido")?,
        reported_earnings: numeric(fixed, "/rendimento_exibido")?,
        return12_months: numeric(fixed, "/rentabilidade_12_meses_percentual")?,
        redemption_conversion: text(fixed, "/cotizacao_de_resgate")?,
        redemption_settlement: text(fixed, "/liquidacao_de_resgate")?,
    };
    check_equal(
        parse_decimal(&numeric(
            root,
            "/posicao_pos_fixado/total_investido_exibido",
        )?)?,
        parse_decimal(&fixed_income.balance)?,
        "Total pós-fixado",
    )?;
    for (path, expected) in [
        ("/panorama_carteira/quantidade_classes", 3),
        (
            "/posicao_fundos_listados/quantidade_ativos",
            listed_funds.holdings.len(),
        ),
        ("/posicao_pos_fixado/quantidade_ativos", 1),
    ] {
        if required(root, path)?.as_u64() != Some(expected as u64) {
            return Err(invalid(
                "Quantidade de ativos/classes divergente na posição.",
            ));
        }
    }
    let mut warnings = Vec::new();
    for path in [
        "/panorama_carteira/observacoes",
        "/posicao_pos_fixado/observacoes",
    ] {
        if let Some(items) = root.pointer(path).and_then(Value::as_array) {
            for item in items {
                warnings.push(
                    item.as_str()
                        .ok_or_else(|| invalid("Observação inválida na posição."))?
                        .to_string(),
                );
            }
        }
    }
    if let Some(items) = root.get("divergencias_e_limites").and_then(Value::as_array) {
        for item in items {
            for key in ["explicacao", "tratamento"] {
                if let Some(value) = item.get(key) {
                    warnings.push(
                        value
                            .as_str()
                            .ok_or_else(|| invalid("Advertência inválida na posição."))?
                            .to_string(),
                    );
                }
            }
        }
    }
    warnings.push("Os custos informados na posição da corretora não substituem o histórico de compras registrado no aplicativo.".into());
    warnings.push("O nome completo do pós-fixado e o período do rendimento informado devem ser conferidos no documento da corretora; esse rendimento não foi somado aos dividendos recebidos.".into());
    parse_normalized(serde_json::to_value(BrokerPosition {
        reference_date,
        institution: text(root, "/instituicao")?,
        imported_at: Utc::now().to_rfc3339(),
        source_path: source.to_string_lossy().into_owned(),
        source_hash: hash.into(),
        overview,
        listed_funds,
        fixed_income,
        warnings,
    })?)
}

fn validate_date(value: &str) -> Result<()> {
    let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| invalid("Data de referência inválida na posição."))?;
    if parsed.year() < 1 || parsed.format("%Y-%m-%d").to_string() != value {
        return Err(invalid("Data de referência inválida na posição."));
    }
    Ok(())
}
fn validate_time(value: &str) -> Result<()> {
    let parsed = NaiveTime::parse_from_str(value, "%H:%M")
        .map_err(|_| invalid("Horário inválido na posição."))?;
    if parsed.format("%H:%M").to_string() != value {
        return Err(invalid("Horário inválido na posição."));
    }
    Ok(())
}
fn normalize_number(value: &mut String, nonnegative: bool) -> Result<Decimal> {
    let parsed = if nonnegative {
        parse_nonnegative(value, "Valor da posição")?
    } else {
        parse_decimal(value)?
    };
    *value = decimal_string(parsed);
    Ok(parsed)
}
fn check_equal(actual: Decimal, expected: Decimal, label: &str) -> Result<()> {
    if actual != expected {
        return Err(invalid(&format!(
            "{label}: valores da posição não conferem."
        )));
    }
    Ok(())
}

fn parse_normalized(value: Value) -> Result<BrokerPosition> {
    let mut position: BrokerPosition = serde_json::from_value(value)?;
    validate_date(&position.reference_date)?;
    DateTime::parse_from_rfc3339(&position.imported_at)
        .map_err(|_| invalid("Data de importação inválida na posição."))?;
    if position.institution.trim().is_empty()
        || position.source_path.trim().is_empty()
        || position.source_hash.len() != 64
        || !position.source_hash.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(invalid("Origem ou instituição inválida na posição."));
    }
    let o = &mut position.overview;
    for time in [
        &o.updated_time,
        &o.captured_time,
        &position.listed_funds.captured_time,
    ]
    .into_iter()
    .flatten()
    {
        validate_time(time)?;
    }
    let total = normalize_number(&mut o.total_with_cash, true)?;
    let invested = normalize_number(&mut o.invested, true)?;
    let cash = normalize_number(&mut o.cash, true)?;
    let listed = normalize_number(&mut o.listed_funds, true)?;
    let fixed = normalize_number(&mut o.fixed_income, true)?;
    let pending = normalize_number(&mut o.pending_income, true)?;
    check_equal(
        invested,
        add(add(listed, fixed)?, pending)?,
        "Soma das classes do panorama",
    )?;
    check_equal(total, add(invested, cash)?, "Total do panorama com caixa")?;
    let funds = &mut position.listed_funds;
    let market = normalize_number(&mut funds.market_value, true)?;
    let cost = normalize_number(&mut funds.cost_basis, true)?;
    let result = normalize_number(&mut funds.unrealized_result, false)?;
    normalize_number(&mut funds.daily_result, false)?;
    check_equal(market, add(cost, result)?, "Resultado dos fundos listados")?;
    let (mut sum_market, mut sum_cost, mut sum_result) =
        (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
    let mut tickers = HashSet::new();
    for holding in &mut funds.holdings {
        holding.ticker = normalize_ticker(&holding.ticker)?;
        if !tickers.insert(holding.ticker.clone()) {
            return Err(invalid("Ticker duplicado na posição."));
        }
        let shares = normalize_number(&mut holding.shares, true)?;
        if shares.is_zero() {
            return Err(invalid(
                "A posição de um ativo deve ter quantidade positiva.",
            ));
        }
        let average = normalize_number(&mut holding.average_price, true)?;
        let price = normalize_number(&mut holding.price, true)?;
        let market = normalize_number(&mut holding.market_value, true)?;
        let cost = normalize_number(&mut holding.cost_basis, true)?;
        let result = normalize_number(&mut holding.unrealized_result, false)?;
        check_equal(
            market,
            money(mul(shares, price)?),
            "Quantidade multiplicada pela cotação",
        )?;
        check_equal(market, add(cost, result)?, "Custo e resultado do ativo")?;
        // Displayed average prices have only two decimals; the broker's total
        // cost is authoritative. Permit precisely that display-rounding range.
        let difference = mul(shares, average)?
            .checked_sub(cost)
            .ok_or(CoreError::ArithmeticOverflow)?
            .abs();
        if difference > add(mul(shares, Decimal::new(5, 3))?, Decimal::new(1, 2))? {
            return Err(invalid("Preço médio incompatível com o custo da posição."));
        }
        sum_market = add(sum_market, market)?;
        sum_cost = add(sum_cost, cost)?;
        sum_result = add(sum_result, result)?;
    }
    check_equal(market, sum_market, "Soma das posições dos ativos")?;
    check_equal(cost, sum_cost, "Soma dos custos dos ativos")?;
    check_equal(result, sum_result, "Soma dos resultados dos ativos")?;
    let fixed = &mut position.fixed_income;
    if fixed.name.trim().is_empty()
        || fixed.redemption_conversion.trim().is_empty()
        || fixed.redemption_settlement.trim().is_empty()
    {
        return Err(invalid("Descrição do pós-fixado incompleta."));
    }
    let balance = normalize_number(&mut fixed.balance, true)?;
    let net = normalize_number(&mut fixed.net_balance, true)?;
    if net > balance {
        return Err(invalid("Saldo líquido do pós-fixado excede o saldo bruto."));
    }
    normalize_number(&mut fixed.reported_earnings, false)?;
    normalize_number(&mut fixed.return12_months, false)?;
    if listed != market
        && !position
            .warnings
            .iter()
            .any(|warning| warning.contains("horários diferentes"))
    {
        position.warnings.push("O panorama e o detalhamento dos fundos listados têm valores ou horários diferentes; seus totais foram preservados separadamente.".into());
    }
    Ok(position)
}
