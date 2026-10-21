//! Explicit, offline import of a dated supplement into an existing wallet.
//! Does not launch the application, import legacy records or contact services.
use ivestfii_core::Repository;
use serde_json::{json, Value};
use std::path::PathBuf;

fn financial_values(repository: &Repository) -> Result<Value, Box<dyn std::error::Error>> {
    let mut value = serde_json::to_value(repository.snapshot()?)?;
    value
        .as_object_mut()
        .ok_or("Snapshot inválido.")?
        .remove("brokerPosition");
    Ok(value)
}

fn base_position(repository: &Repository) -> Result<Value, Box<dyn std::error::Error>> {
    let snapshot = repository.snapshot()?;
    let mut value = serde_json::to_value(snapshot.broker_position.ok_or("Posição ausente.")?)?;
    value["fixedIncome"]
        .as_object_mut()
        .ok_or("Pós-fixado inválido.")?
        .remove("latestObservation");
    Ok(value)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let database = PathBuf::from(
        args.next()
            .ok_or("Informe o banco da carteira existente.")?,
    );
    let source = PathBuf::from(args.next().ok_or("Informe o JSON do complemento.")?);
    if args.next().is_some() {
        return Err("Argumentos extras não permitidos.".into());
    }
    let input: Value = serde_json::from_slice(&std::fs::read(&source)?)?;
    if input.get("type").and_then(Value::as_str) != Some("broker_fixed_income_observation") {
        return Err("Este comando aceita somente complementos do pós-fixado.".into());
    }
    let repository = Repository::open_existing(database)?;
    let before = financial_values(&repository)?;
    let original_position = base_position(&repository)?;
    let position = repository.import_broker_position_file(source)?;
    if before != financial_values(&repository)? || original_position != base_position(&repository)?
    {
        return Err(
            "O histórico mudou durante a verificação. Confira possíveis edições simultâneas."
                .into(),
        );
    }
    let observation = position
        .fixed_income
        .latest_observation
        .ok_or("Complemento não encontrado.")?;
    println!(
        "{}",
        json!({
            "status": "ok", "positionReferenceDate": position.reference_date,
            "observationReferenceDate": observation.reference_date,
            "contributionCount": observation.contributions.len(),
            "financialRecordsUnchanged": true, "originalPositionPreserved": true
        })
    );
    Ok(())
}
