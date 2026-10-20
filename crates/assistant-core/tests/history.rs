use ivestfii_assistant::{connector_specifications, AssistantError, AssistantStore};
use serde_json::json;

#[test]
fn one_conversation_survives_provider_switch_and_restart() {
    let temporary = tempfile::tempdir().unwrap();
    let database = temporary.path().join("history.sqlite3");
    let conversation;
    let originals;
    {
        let store = AssistantStore::open(&database).unwrap();
        conversation = store
            .create_conversation("Aportes para aposentadoria")
            .unwrap();
        store
            .append_message(
                &conversation.id,
                "system",
                "Considere a reserva de emergência antes de comparar aportes.",
                None,
                None,
            )
            .unwrap();
        store
            .append_message(
                &conversation.id,
                "user",
                "Meu prazo é de vinte anos e quero renda mensal.",
                Some("codex"),
                Some("modelo-a"),
            )
            .unwrap();
        store
            .append_message(
                &conversation.id,
                "assistant",
                "Vamos verificar sua concentração e suas metas.",
                Some("codex"),
                Some("modelo-a"),
            )
            .unwrap();
        store
            .append_message(
                &conversation.id,
                "user",
                "Continue com o mesmo objetivo e a mesma carteira.",
                Some("claude"),
                Some("modelo-b"),
            )
            .unwrap();
        originals = store.messages(&conversation.id).unwrap();
        assert_eq!(originals.len(), 4);
    }
    let reopened = AssistantStore::open(&database).unwrap();
    assert_eq!(reopened.messages(&conversation.id).unwrap(), originals);
    let context = reopened.build_context(&conversation.id, 10_000).unwrap();
    assert!(!context.truncated);
    assert_eq!(context.messages, originals);
    assert_eq!(context.messages[1].provider.as_deref(), Some("codex"));
    assert_eq!(context.messages[3].provider.as_deref(), Some("claude"));
    let serialized = serde_json::to_value(&context.messages[3]).unwrap();
    assert_eq!(serialized["conversationId"], conversation.id);
    assert!(serialized.get("createdAt").is_some());
    assert_eq!(reopened.list_conversations().unwrap().len(), 1);
}

#[test]
fn bounded_context_keeps_originals_instructions_and_latest_request() {
    let store = AssistantStore::open(":memory:").unwrap();
    let conversation = store.create_conversation("Histórico extenso").unwrap();
    let instruction = store
        .append_message(
            &conversation.id,
            "system",
            "Objetivo: renda mensal. Nunca envie ordens de compra.",
            None,
            None,
        )
        .unwrap();
    for number in 0..20 {
        store
            .append_message(
                &conversation.id,
                "user",
                &format!("Pedido {number}: {}", "ação 🦀 longo prazo. ".repeat(10)),
                Some("codex"),
                None,
            )
            .unwrap();
    }
    let latest = store
        .append_message(
            &conversation.id,
            "user",
            "Compare o próximo aporte com meus objetivos anteriores.",
            Some("claude"),
            None,
        )
        .unwrap();
    let before = store.messages(&conversation.id).unwrap();
    let context = store.build_context(&conversation.id, 800).unwrap();
    assert!(context.truncated);
    assert!(context.omitted_message_count > 0);
    assert_eq!(context.messages.first(), Some(&instruction));
    assert_eq!(context.messages.last(), Some(&latest));
    assert!(context
        .summary
        .as_ref()
        .unwrap()
        .contains("não é resumo de IA"));
    let actual: usize = context
        .messages
        .iter()
        .map(|m| m.content.chars().count())
        .sum::<usize>()
        + context.summary.as_deref().map_or(0, |s| s.chars().count());
    assert_eq!(context.included_chars, actual);
    assert!(actual <= context.max_chars);
    assert_eq!(store.messages(&conversation.id).unwrap(), before);
}

#[test]
fn oversized_messages_are_explicitly_flagged_and_unicode_safe() {
    let store = AssistantStore::open(":memory:").unwrap();
    let conversation = store.create_conversation("Limites").unwrap();
    let original = store
        .append_message(&conversation.id, "user", &"🦀á".repeat(100), None, None)
        .unwrap();
    let context = store.build_context(&conversation.id, 19).unwrap();
    assert_eq!(context.messages[0].content.chars().count(), 19);
    assert_eq!(context.truncated_message_ids, vec![original.id]);
    assert_eq!(context.included_chars, 19);
    assert_eq!(
        store.messages(&conversation.id).unwrap()[0]
            .content
            .chars()
            .count(),
        200
    );
    let empty = store.build_context(&conversation.id, 0).unwrap();
    assert_eq!(empty.included_chars, 0);
    assert_eq!(empty.omitted_message_count, 1);
}

#[test]
fn insufficient_instruction_budget_fails_without_dropping_instructions() {
    let store = AssistantStore::open(":memory:").unwrap();
    let conversation = store.create_conversation("Instruções").unwrap();
    store
        .append_message(
            &conversation.id,
            "system",
            "Não apague esta instrução.",
            None,
            None,
        )
        .unwrap();
    assert!(matches!(
        store.build_context(&conversation.id, 5),
        Err(AssistantError::Validation(_))
    ));
}

#[test]
fn checkpoints_tool_results_and_citations_survive_restart() {
    let temporary = tempfile::tempdir().unwrap();
    let database = temporary.path().join("assistant.sqlite3");
    let (conversation, task, tool, source);
    {
        let store = AssistantStore::open(&database).unwrap();
        conversation = store.create_conversation("Análise interrompida").unwrap();
        task = store
            .create_task(&conversation.id, "portfolio-analysis")
            .unwrap();
        store
            .save_checkpoint(
                &task.id,
                "running",
                json!({"completedSteps": ["portfolio"]}),
            )
            .unwrap();
        let event = store
            .begin_tool_event(
                &conversation.id,
                Some(&task.id),
                "market.quote",
                json!({"ticker":"TEST11"}),
            )
            .unwrap();
        tool = store
            .finish_tool_event(
                &event.id,
                "completed",
                json!({"price":"100.00", "currency":"BRL"}),
            )
            .unwrap();
        source = store
            .add_source(
                &conversation.id,
                Some(&event.id),
                "Dado de teste",
                "https://example.com/quote",
                Some("Fixture"),
                Some("Citação sintética de teste."),
            )
            .unwrap();
        store
            .save_checkpoint(
                &task.id,
                "interrupted",
                json!({"completedSteps": ["portfolio", "quotes"], "nextStep": "compare"}),
            )
            .unwrap();
        assert!(store
            .finish_tool_event(&event.id, "completed", json!({"price":"1.00"}))
            .is_err());
    }
    let store = AssistantStore::open(database).unwrap();
    assert_eq!(store.tool_events(&conversation.id).unwrap(), vec![tool]);
    assert_eq!(store.sources(&conversation.id).unwrap(), vec![source]);
    let saved_task = &store.tasks(&conversation.id).unwrap()[0];
    assert_eq!(saved_task.id, task.id);
    assert_eq!(saved_task.status, "interrupted");
    assert_eq!(saved_task.checkpoint["nextStep"], "compare");
}

#[test]
fn records_cannot_cross_conversation_boundaries() {
    let store = AssistantStore::open(":memory:").unwrap();
    let first = store.create_conversation("Primeira").unwrap();
    let second = store.create_conversation("Segunda").unwrap();
    let task = store.create_task(&first.id, "analysis").unwrap();
    assert!(store
        .begin_tool_event(&second.id, Some(&task.id), "portfolio.read", json!({}))
        .is_err());
    let event = store
        .begin_tool_event(&first.id, Some(&task.id), "portfolio.read", json!({}))
        .unwrap();
    assert!(store
        .add_source(
            &second.id,
            Some(&event.id),
            "Fonte",
            "https://example.com",
            None,
            None
        )
        .is_err());
    assert!(store
        .append_message("missing", "user", "Olá", None, None)
        .is_err());
    assert!(store
        .append_message(&first.id, "invented-role", "Olá", None, None)
        .is_err());
    assert!(store.create_conversation(" ").is_err());
    assert_eq!(connector_specifications().len(), 3);
}
