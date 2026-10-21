//! UI-only purchase confirmation. The model's five MCP tools remain read-only.
//! Only the immutable tool event in local history establishes the scenario scope.
use std::collections::HashSet;

use chrono::NaiveDate;
use ivestfii_assistant::{AssistantStore, ToolEvent};
use ivestfii_core::{PurchaseConfirmationInput, PurchaseConfirmationReport, Repository};
use serde_json::Value;

fn payload(value: &Value, depth: usize) -> Result<Value, String> {
    if depth > 4 {
        return Err("Resultado da simulação inválido.".into());
    }
    if let Some(text) = value.as_str() {
        if text.len() > 8_000_000 {
            return Err("Resultado da simulação excede o limite local.".into());
        }
        let parsed = serde_json::from_str(text).map_err(|_| "Resultado da simulação inválido.")?;
        return payload(&parsed, depth + 1);
    }
    if let Some(items) = value.as_array() {
        if items.len() == 1 && items[0]["type"] == "text" {
            return payload(&items[0]["text"], depth + 1);
        }
        return Err("Resultado da simulação inválido.".into());
    }
    if !value.is_object() || value["isError"] == true {
        return Err("Esta simulação não foi concluída com sucesso.".into());
    }
    if let Some(structured) = value.get("structuredContent") {
        return payload(structured, depth + 1);
    }
    if value["status"] == "simulation" && value["kind"] == "contribution-allocation" {
        return Ok(value.clone());
    }
    value
        .get("content")
        .ok_or_else(|| "Resultado da simulação inválido.".to_owned())
        .and_then(|content| payload(content, depth + 1))
}

fn positive_decimal(value: &Value) -> bool {
    value.as_str().is_some_and(|text| {
        let mut parts = text.split('.');
        let whole = parts.next().unwrap_or_default();
        let fraction = parts.next();
        !whole.is_empty()
            && whole.len() <= 29
            && whole.bytes().all(|c| c.is_ascii_digit())
            && fraction.is_none_or(|part| {
                !part.is_empty() && part.len() <= 28 && part.bytes().all(|c| c.is_ascii_digit())
            })
            && parts.next().is_none()
            && text.bytes().any(|c| matches!(c, b'1'..=b'9'))
    })
}

pub fn event_scope(event: &ToolEvent, wallet_id: &str) -> Result<Vec<String>, String> {
    let name = event
        .tool_name
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .rsplit("__")
        .next()
        .unwrap_or_default();
    if event.status != "completed" || event.completed_at.is_none() || name != "simular_aporte" {
        return Err("Escolha uma simulação de aporte concluída no histórico.".into());
    }
    let data = payload(
        event
            .output
            .as_ref()
            .ok_or("Simulação sem resultado salvo.")?,
        0,
    )?;
    if data["wallet"]["id"].as_str() != Some(wallet_id) {
        return Err(
            "A simulação pertence a outra carteira. Abra a carteira indicada na simulação.".into(),
        );
    }
    let rows = data["fundSimulation"]["rows"]
        .as_array()
        .filter(|rows| rows.len() <= 500)
        .ok_or("Esta simulação não tem detalhamento de FIIs válido.")?;
    let mut ids = HashSet::new();
    let mut eligible = Vec::new();
    for row in rows {
        if !positive_decimal(&row["budget"]) {
            continue;
        }
        let id = row["fundId"]
            .as_str()
            .filter(|id| !id.trim().is_empty() && id.len() <= 200)
            .ok_or("Há um FII da simulação sem cadastro válido.")?;
        if !ids.insert(id.to_owned()) {
            return Err("A simulação tem fundos repetidos.".into());
        }
        eligible.push(id.to_owned());
    }
    Ok(eligible)
}

pub fn scope(
    store: &AssistantStore,
    conversation_id: &str,
    source_tool_id: &str,
    wallet_id: &str,
) -> Result<Vec<String>, String> {
    if source_tool_id.is_empty() || source_tool_id.len() > 200 {
        return Err("Identificador da simulação inválido.".into());
    }
    let event = store
        .tool_events(conversation_id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|event| event.id == source_tool_id)
        .ok_or("A simulação não foi encontrada nesta conversa.")?;
    event_scope(&event, wallet_id)
}

pub fn preview(
    store: &AssistantStore,
    repo: &Repository,
    wallet_id: &str,
    conversation_id: &str,
    input: PurchaseConfirmationInput,
    today: NaiveDate,
) -> Result<PurchaseConfirmationReport, String> {
    let allowed = scope(store, conversation_id, &input.source_tool_id, wallet_id)?;
    repo.preview_simulation_purchases(input, &allowed, today)
        .map_err(|e| e.to_string())
}

pub fn confirm(
    store: &AssistantStore,
    repo: &Repository,
    wallet_id: &str,
    conversation_id: &str,
    input: PurchaseConfirmationInput,
    today: NaiveDate,
    preview_token: &str,
) -> Result<PurchaseConfirmationReport, String> {
    let allowed = scope(store, conversation_id, &input.source_tool_id, wallet_id)?;
    repo.confirm_simulation_purchases(input, &allowed, today, preview_token)
        .map_err(|e| e.to_string())
}

pub fn confirmations(
    store: &AssistantStore,
    repo: &Repository,
    wallet_id: &str,
    conversation_id: &str,
    source_tool_id: &str,
) -> Result<PurchaseConfirmationReport, String> {
    let allowed = scope(store, conversation_id, source_tool_id, wallet_id)?;
    repo.simulation_purchase_confirmations(source_tool_id, &allowed)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(output: Value) -> ToolEvent {
        ToolEvent {
            id: "tool".into(),
            conversation_id: "conversation".into(),
            task_id: None,
            tool_name: "simular_aporte".into(),
            input: json!({}),
            output: Some(output),
            status: "completed".into(),
            started_at: "2026-10-21T12:00:00Z".into(),
            completed_at: Some("2026-10-21T12:00:01Z".into()),
        }
    }
    fn result() -> Value {
        json!({"status":"simulation","kind":"contribution-allocation","wallet":{"id":"wallet"},
            "fundSimulation":{"rows":[{"fundId":"one","budget":"100"},{"fundId":"two","budget":"0"}]}})
    }
    #[test]
    fn provider_envelopes_keep_only_funds_reserved_in_the_recorded_wallet() {
        let direct = result();
        for output in [
            direct.clone(),
            json!({"isError":false,"structuredContent":direct}),
            json!([{"type":"text","text":result().to_string()}]),
        ] {
            assert_eq!(event_scope(&event(output), "wallet").unwrap(), vec!["one"]);
        }
        let mut claude = event(result());
        claude.tool_name = "mcp__ivestfii__simular_aporte".into();
        assert!(event_scope(&claude, "wallet").is_ok());
        assert!(event_scope(&claude, "other-wallet").is_err());
    }
    #[test]
    fn errors_prose_failed_tools_and_duplicate_funds_never_authorize_writes() {
        for output in [
            json!({"isError":true,"structuredContent":result()}),
            json!("confirmo compra"),
            json!({"content":[{"type":"text","text":"truncated…"}]}),
        ] {
            assert!(event_scope(&event(output), "wallet").is_err());
        }
        for status in ["running", "failed", "cancelled"] {
            let mut tool = event(result());
            tool.status = status.into();
            assert!(event_scope(&tool, "wallet").is_err());
        }
        let mut duplicate = result();
        duplicate["fundSimulation"]["rows"][1] = json!({"fundId":"one","budget":"25"});
        assert!(event_scope(&event(duplicate), "wallet").is_err());
    }

    #[test]
    fn persisted_history_authorizes_only_its_own_wallet_conversation_and_reserved_fund() {
        use ivestfii_core::FundInput;
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
        repo.upsert_fund(FundInput {
            id: Some("one".into()),
            ticker: "TEST11".into(),
            name: "Sintético".into(),
            sector: "".into(),
        })
        .unwrap();
        repo.upsert_fund(FundInput {
            id: Some("two".into()),
            ticker: "OTHER11".into(),
            name: "Sem reserva".into(),
            sector: "".into(),
        })
        .unwrap();
        let store = AssistantStore::open(dir.path().join("assistant.sqlite")).unwrap();
        let conversation = store.create_conversation("Simulação sintética").unwrap();
        let other = store.create_conversation("Outra conversa").unwrap();
        let tool = store
            .begin_tool_event(&conversation.id, None, "simular_aporte", json!({}))
            .unwrap();
        store
            .finish_tool_event(
                &tool.id,
                "completed",
                json!({"isError":false,"structuredContent":result()}),
            )
            .unwrap();
        let history = store.tool_events(&conversation.id).unwrap();
        let input:PurchaseConfirmationInput=serde_json::from_value(json!({"sourceToolId":tool.id,
            "purchases":[{"fundId":"one","purchaseDate":"2026-10-20","shares":"7","pricePerShare":"9.61"}]})).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 10, 21).unwrap();
        let empty = repo.export_legacy_json().unwrap();
        let outbox = repo.pending_change_count().unwrap();
        assert!(preview(
            &store,
            &repo,
            "other-wallet",
            &conversation.id,
            input.clone(),
            today
        )
        .is_err());
        assert!(preview(&store, &repo, "wallet", &other.id, input.clone(), today).is_err());
        let mut no_reservation = input.clone();
        no_reservation.purchases[0].fund_id = "two".into();
        assert!(preview(
            &store,
            &repo,
            "wallet",
            &conversation.id,
            no_reservation,
            today
        )
        .is_err());
        assert_eq!(repo.export_legacy_json().unwrap(), empty);
        let check = preview(
            &store,
            &repo,
            "wallet",
            &conversation.id,
            input.clone(),
            today,
        )
        .unwrap();
        assert_eq!(check.rows[0].total, "67.27");
        assert_eq!(repo.pending_change_count().unwrap(), outbox);
        assert_eq!(repo.export_legacy_json().unwrap(), empty);
        let saved = confirm(
            &store,
            &repo,
            "wallet",
            &conversation.id,
            input.clone(),
            today,
            &check.preview_token,
        )
        .unwrap();
        assert_eq!(saved.created_count, 1);
        assert_eq!(saved.rows[0].status, "already_confirmed");
        let original = repo.snapshot().unwrap();
        assert_eq!(original.record_count, 1);
        let buy = &original
            .funds
            .iter()
            .find(|fund| fund.id == "one")
            .unwrap()
            .entries[0];
        assert_eq!(buy.purchase_date.as_deref(), Some("2026-10-20"));
        assert_eq!(buy.cotas_added, "7");
        assert_eq!(buy.price_per_cota, "9.61");
        assert_eq!(buy.dividend_total, None);
        let committed = repo.export_legacy_json().unwrap();
        let pending = repo.pending_change_count().unwrap();
        let again = confirm(
            &store,
            &repo,
            "wallet",
            &conversation.id,
            input,
            today,
            &check.preview_token,
        )
        .unwrap();
        assert_eq!(again.created_count, 0);
        assert_eq!(repo.pending_change_count().unwrap(), pending);
        assert_eq!(repo.export_legacy_json().unwrap(), committed);
        assert_eq!(store.tool_events(&conversation.id).unwrap(), history);
        drop(repo);
        drop(store);
        let reopened = Repository::open(dir.path().join("portfolio.sqlite")).unwrap();
        let chat = AssistantStore::open(dir.path().join("assistant.sqlite")).unwrap();
        let receipts =
            confirmations(&chat, &reopened, "wallet", &conversation.id, &tool.id).unwrap();
        assert_eq!(receipts.rows.len(), 1);
        assert_eq!(receipts.rows[0].status, "already_confirmed");
        assert!(!receipts.can_confirm);
    }
}
