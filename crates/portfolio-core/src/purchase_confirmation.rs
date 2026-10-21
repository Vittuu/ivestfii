//! Explicit, local confirmation of purchases actually made after a simulation.
//! A trusted desktop adapter supplies the recorded simulation's fund scope.
//! Preview never writes; confirmation and its idempotency receipts are atomic.

use crate::{
    decimal_string, enqueue, entry_payload, insert_entry, parse_decimal, snapshot_connection,
    CoreError, MonthlyEntry, Repository, Result,
};
use chrono::{Datelike, NaiveDate, Utc};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PurchaseConfirmationItem {
    pub fund_id: String,
    pub purchase_date: String,
    pub shares: String,
    pub price_per_share: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PurchaseConfirmationInput {
    pub source_tool_id: String,
    pub purchases: Vec<PurchaseConfirmationItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseConfirmationRow {
    pub fund_id: String,
    pub ticker: String,
    pub purchase_date: String,
    pub shares: String,
    pub price_per_share: String,
    pub total: String,
    pub status: String,
    pub record_id: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseConfirmationReport {
    pub source_tool_id: String,
    pub rows: Vec<PurchaseConfirmationRow>,
    pub can_confirm: bool,
    pub created_count: usize,
    pub matched_count: usize,
    pub preview_token: String,
}

#[derive(Debug)]
struct Receipt {
    record_id: String,
    purchase_date: String,
    shares: String,
    price_per_share: String,
}

impl Repository {
    pub fn preview_simulation_purchases(
        &self,
        input: PurchaseConfirmationInput,
        allowed_fund_ids: &[String],
        today: NaiveDate,
    ) -> Result<PurchaseConfirmationReport> {
        let input = validate(input, allowed_fund_ids, today)?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let report = preview(&tx, &input)?;
        tx.commit()?;
        Ok(report)
    }

    pub fn confirm_simulation_purchases(
        &self,
        input: PurchaseConfirmationInput,
        allowed_fund_ids: &[String],
        today: NaiveDate,
        expected_preview_token: &str,
    ) -> Result<PurchaseConfirmationReport> {
        let input = validate(input, allowed_fund_ids, today)?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut report = preview(&tx, &input)?;
        if report.rows.iter().any(|row| row.status == "conflict") {
            return Ok(report);
        }
        // A completed retry is an acknowledgement, even if the caller lost the
        // first response and therefore only has the original preview token.
        if report
            .rows
            .iter()
            .all(|row| row.status == "already_confirmed")
        {
            return Ok(report);
        }
        if expected_preview_token != report.preview_token {
            for row in &mut report.rows {
                if row.status != "already_confirmed" {
                    row.status = "conflict".into();
                    row.message = Some("O histórico mudou desde a conferência. Revise as compras novamente antes de salvar.".into());
                }
            }
            report.can_confirm = false;
            return Ok(report);
        }
        let now = Utc::now().to_rfc3339();
        let baseline = snapshot_connection(&tx)?;
        let mut all_shares = parse_decimal(&baseline.totals.cotas)?;
        let mut all_invested = parse_decimal(&baseline.totals.invested)?;
        for row in report.rows.iter().filter(|row| row.status == "new") {
            let fund = baseline
                .funds
                .iter()
                .find(|fund| fund.id == row.fund_id)
                .ok_or_else(|| {
                    CoreError::Validation("Fundo não encontrado nesta carteira.".into())
                })?;
            let shares = parse_decimal(&row.shares)?;
            let cost = parse_decimal(&row.total)?;
            exact_add(parse_decimal(&fund.total_cotas)?, shares)?;
            exact_add(parse_decimal(&fund.total_invested)?, cost)?;
            all_shares = exact_add(all_shares, shares)?;
            all_invested = exact_add(all_invested, cost)?;
        }
        for row in &mut report.rows {
            if row.status == "already_confirmed" {
                continue;
            }
            if row.status == "new" {
                let entry = MonthlyEntry {
                    id: Uuid::new_v4().to_string(),
                    month: row.purchase_date[..7].into(),
                    purchase_date: Some(row.purchase_date.clone()),
                    cotas_added: row.shares.clone(),
                    price_per_cota: row.price_per_share.clone(),
                    dividend_per_cota: "0".into(),
                    // None is essential: explicit zero would override the
                    // public dividend calculation for the entire fund/month.
                    dividend_total: None,
                    notes: "Compra realizada e confirmada a partir de uma simulação de aporte."
                        .into(),
                };
                let sequence: i64 = tx.query_row(
                    "SELECT COALESCE(MAX(sequence), -1) + 1 FROM monthly_records WHERE fund_id=?1",
                    [&row.fund_id],
                    |record| record.get(0),
                )?;
                insert_entry(&tx, &row.fund_id, &entry, sequence, &now)?;
                enqueue(
                    &tx,
                    "monthlyRecord",
                    &entry.id,
                    &entry_payload(&row.fund_id, &entry, 1),
                    &now,
                )?;
                row.record_id = Some(entry.id);
                report.created_count += 1;
            }
            tx.execute(
                "INSERT INTO simulation_purchase_receipts(source_tool_id,fund_id,record_id,purchase_date,shares,price_per_share,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![input.source_tool_id,row.fund_id,row.record_id,row.purchase_date,row.shares,row.price_per_share,now],
            )?;
            row.status = "already_confirmed".into();
            row.message = Some(
                "Compra confirmada e vinculada ao histórico. Nenhum lançamento será duplicado."
                    .into(),
            );
        }
        // Overflow or any database failure rolls back purchases, receipts and
        // outbox together; no partial selection can be persisted.
        snapshot_connection(&tx)?;
        tx.commit()?;
        report.can_confirm = false;
        report.preview_token = token(&report.source_tool_id, &report.rows)?;
        Ok(report)
    }

    /// Restore confirmation badges from the selected wallet after reopening a
    /// conversation. This reads saved receipts; it never recreates purchases.
    pub fn simulation_purchase_confirmations(
        &self,
        source_tool_id: &str,
        allowed_fund_ids: &[String],
    ) -> Result<PurchaseConfirmationReport> {
        validate_source(source_tool_id)?;
        let allowed: HashSet<_> = allowed_fund_ids.iter().map(String::as_str).collect();
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let purchases = {
            let mut statement = tx.prepare(
                "SELECT fund_id,purchase_date,shares,price_per_share FROM simulation_purchase_receipts WHERE source_tool_id=?1 ORDER BY fund_id",
            )?;
            let rows = statement.query_map([source_tool_id], |row| {
                Ok(PurchaseConfirmationItem {
                    fund_id: row.get(0)?,
                    purchase_date: row.get(1)?,
                    shares: row.get(2)?,
                    price_per_share: row.get(3)?,
                })
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        if purchases
            .iter()
            .any(|item| !allowed.contains(item.fund_id.as_str()))
        {
            return Err(CoreError::Validation(
                "As confirmações não correspondem aos fundos desta simulação.".into(),
            ));
        }
        let mut report = preview(
            &tx,
            &PurchaseConfirmationInput {
                source_tool_id: source_tool_id.into(),
                purchases,
            },
        )?;
        report.can_confirm = false;
        tx.commit()?;
        Ok(report)
    }
}

fn validate_source(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 300
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(CoreError::Validation(
            "Identificador da simulação inválido.".into(),
        ));
    }
    Ok(())
}

fn validate(
    mut input: PurchaseConfirmationInput,
    allowed_fund_ids: &[String],
    today: NaiveDate,
) -> Result<PurchaseConfirmationInput> {
    validate_source(&input.source_tool_id)?;
    if input.purchases.is_empty() || input.purchases.len() > 500 {
        return Err(CoreError::Validation(
            "Selecione de 1 a 500 compras para conferir.".into(),
        ));
    }
    let allowed: HashSet<_> = allowed_fund_ids.iter().map(String::as_str).collect();
    let mut seen = HashSet::new();
    let mut total = Decimal::ZERO;
    for item in &mut input.purchases {
        if !allowed.contains(item.fund_id.as_str()) || !seen.insert(item.fund_id.clone()) {
            return Err(CoreError::Validation(
                "Selecione cada fundo apenas uma vez e somente entre os fundos desta simulação."
                    .into(),
            ));
        }
        let day = NaiveDate::parse_from_str(&item.purchase_date, "%Y-%m-%d").ok();
        if day.is_none_or(|day| {
            day.year() < 1
                || day > today
                || day.format("%Y-%m-%d").to_string() != item.purchase_date
        }) {
            return Err(CoreError::Validation(
                "Informe a data real da compra, válida e até hoje, no formato AAAA-MM-DD.".into(),
            ));
        }
        if item.shares.is_empty()
            || item.shares.len() > 29
            || !item.shares.bytes().all(|digit| digit.is_ascii_digit())
        {
            return Err(CoreError::Validation(
                "Informe uma quantidade inteira e positiva de cotas.".into(),
            ));
        }
        let shares = parse_decimal(&item.shares)?;
        if shares <= Decimal::ZERO {
            return Err(CoreError::Validation(
                "Informe uma quantidade inteira e positiva de cotas.".into(),
            ));
        }
        let mut price_parts = item.price_per_share.split('.');
        let whole = price_parts.next().unwrap_or_default();
        let fraction = price_parts.next();
        if whole.is_empty()
            || whole.len() > 29
            || !whole.bytes().all(|digit| digit.is_ascii_digit())
            || fraction.is_some_and(|fraction| {
                fraction.is_empty()
                    || fraction.len() > 2
                    || !fraction.bytes().all(|digit| digit.is_ascii_digit())
            })
            || price_parts.next().is_some()
        {
            return Err(CoreError::Validation(
                "Informe o preço real por cota, positivo e com até duas casas decimais.".into(),
            ));
        }
        let price = parse_decimal(&item.price_per_share)?;
        if price <= Decimal::ZERO {
            return Err(CoreError::Validation(
                "Informe o preço real por cota, positivo e com até duas casas decimais.".into(),
            ));
        }
        let cost = exact_cost(shares, price)?;
        total = exact_add(total, cost)?;
        item.shares = decimal_string(shares);
        item.price_per_share = decimal_string(price);
    }
    Ok(input)
}

fn exact_cost(shares: Decimal, price: Decimal) -> Result<Decimal> {
    let mut coefficient = shares
        .mantissa()
        .checked_mul(price.mantissa())
        .ok_or(CoreError::ArithmeticOverflow)?;
    let mut scale = price.scale();
    while coefficient > Decimal::MAX.mantissa() && scale > 0 && coefficient % 10 == 0 {
        coefficient /= 10;
        scale -= 1;
    }
    if coefficient > Decimal::MAX.mantissa() {
        return Err(CoreError::ArithmeticOverflow);
    }
    Ok(Decimal::from_i128_with_scale(coefficient, scale))
}

fn exact_add(left: Decimal, right: Decimal) -> Result<Decimal> {
    let mut scale = left.scale().max(right.scale());
    let align = |value: Decimal| {
        value
            .mantissa()
            .checked_mul(10_i128.pow(scale - value.scale()))
            .ok_or(CoreError::ArithmeticOverflow)
    };
    let mut coefficient = align(left)?
        .checked_add(align(right)?)
        .ok_or(CoreError::ArithmeticOverflow)?;
    while coefficient > Decimal::MAX.mantissa() && scale > 0 && coefficient % 10 == 0 {
        coefficient /= 10;
        scale -= 1;
    }
    if coefficient > Decimal::MAX.mantissa() {
        return Err(CoreError::ArithmeticOverflow);
    }
    Ok(Decimal::from_i128_with_scale(coefficient, scale))
}

fn receipt(connection: &Connection, source: &str, fund_id: &str) -> Result<Option<Receipt>> {
    Ok(connection.query_row(
        "SELECT record_id,purchase_date,shares,price_per_share FROM simulation_purchase_receipts WHERE source_tool_id=?1 AND fund_id=?2",
        params![source,fund_id],
        |row| Ok(Receipt { record_id:row.get(0)?,purchase_date:row.get(1)?,shares:row.get(2)?,price_per_share:row.get(3)? }),
    ).optional()?)
}

fn preview(
    connection: &Connection,
    input: &PurchaseConfirmationInput,
) -> Result<PurchaseConfirmationReport> {
    let mut rows = Vec::with_capacity(input.purchases.len());
    for item in &input.purchases {
        let ticker: String = connection
            .query_row(
                "SELECT ticker FROM funds WHERE id=?1",
                [&item.fund_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| {
                CoreError::Validation("Fundo da simulação não encontrado nesta carteira.".into())
            })?;
        let shares = parse_decimal(&item.shares)?;
        let price = parse_decimal(&item.price_per_share)?;
        let mut row = PurchaseConfirmationRow {
            fund_id: item.fund_id.clone(),
            ticker,
            purchase_date: item.purchase_date.clone(),
            shares: decimal_string(shares),
            price_per_share: decimal_string(price),
            total: decimal_string(exact_cost(shares, price)?),
            status: "new".into(),
            record_id: None,
            message: None,
        };
        if let Some(saved) = receipt(connection, &input.source_tool_id, &item.fund_id)? {
            row.record_id = Some(saved.record_id.clone());
            let same_input = saved.purchase_date == item.purchase_date
                && parse_decimal(&saved.shares)? == shares
                && parse_decimal(&saved.price_per_share)? == price;
            let record: Option<(String, Option<String>, String, String, String)> = connection.query_row(
                "SELECT fund_id,purchase_date,cotas_added,price_per_cota,month FROM monthly_records WHERE id=?1",
                [&saved.record_id],|record|Ok((record.get(0)?,record.get(1)?,record.get(2)?,record.get(3)?,record.get(4)?)),
            ).optional()?;
            let current_matches = match record {
                Some((fund_id, day, quantity, cost, month)) => {
                    fund_id == item.fund_id
                        && month == saved.purchase_date[..7]
                        && day.as_deref() == Some(saved.purchase_date.as_str())
                        && parse_decimal(&quantity)? == parse_decimal(&saved.shares)?
                        && parse_decimal(&cost)? == parse_decimal(&saved.price_per_share)?
                }
                None => false,
            };
            if same_input && current_matches {
                row.status = "already_confirmed".into();
                row.message = Some(
                    "Compra já confirmada nesta simulação. Nenhum lançamento será duplicado."
                        .into(),
                );
            } else {
                row.status = "conflict".into();
                row.message = Some(if !same_input {
                    "Este fundo já foi confirmado nesta simulação com outros valores. Para corrigir, edite o lançamento existente no histórico."
                } else {
                    "O lançamento confirmado foi alterado no histórico. Confira e corrija o lançamento existente; ele não será recriado."
                }.into());
            }
            rows.push(row);
            continue;
        }
        let mut matches = Vec::new();
        let mut undated = false;
        let mut statement = connection.prepare(
            "SELECT id,purchase_date,cotas_added,price_per_cota FROM monthly_records WHERE fund_id=?1 AND month=?2",
        )?;
        let candidates =
            statement.query_map(params![item.fund_id, &item.purchase_date[..7]], |record| {
                Ok((
                    record.get::<_, String>(0)?,
                    record.get::<_, Option<String>>(1)?,
                    record.get::<_, String>(2)?,
                    record.get::<_, String>(3)?,
                ))
            })?;
        for candidate in candidates {
            let (id, day, quantity, cost) = candidate?;
            if parse_decimal(&quantity)? != shares || parse_decimal(&cost)? != price {
                continue;
            }
            match day {
                Some(day) if day == item.purchase_date => matches.push(id),
                None => undated = true,
                _ => (),
            }
        }
        if matches.len() > 1 || undated {
            row.status = "conflict".into();
            row.message = Some(if undated {
                "Há uma compra com as mesmas cotas e preço neste mês, sem dia informado. Confira a data no histórico antes de registrar para evitar duplicação."
            } else {
                "Há mais de uma compra com a mesma data, cotas e preço. Confira os lançamentos no histórico antes de continuar."
            }.into());
        } else if let Some(id) = matches.pop() {
            row.status = "already_saved".into();
            row.record_id = Some(id);
            row.message = Some(
                "Já salvo no histórico; será vinculado à simulação, sem novo lançamento.".into(),
            );
        }
        rows.push(row);
    }
    let can_confirm = !rows.iter().any(|row| row.status == "conflict")
        && rows
            .iter()
            .any(|row| row.status == "new" || row.status == "already_saved");
    let matched_count = rows
        .iter()
        .filter(|row| row.status == "already_saved" || row.status == "already_confirmed")
        .count();
    let preview_token = token(&input.source_tool_id, &rows)?;
    Ok(PurchaseConfirmationReport {
        source_tool_id: input.source_tool_id.clone(),
        rows,
        can_confirm,
        created_count: 0,
        matched_count,
        preview_token,
    })
}

fn token(source_tool_id: &str, rows: &[PurchaseConfirmationRow]) -> Result<String> {
    let mut canonical = rows.to_vec();
    canonical.sort_by(|left, right| left.fund_id.cmp(&right.fund_id));
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(
            &json!({"sourceToolId":source_tool_id,"rows":canonical})
        )?)
    ))
}
