use crate::{ContextBundle, Message};

fn prefix(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn tail(value: &str, limit: usize) -> String {
    let length = value.chars().count();
    value.chars().skip(length.saturating_sub(limit)).collect()
}

/// A deliberately labelled deterministic excerpt, not a claimed semantic summary.
fn extract(omitted: &[Message], limit: usize) -> Option<String> {
    if omitted.is_empty() || limit < 80 {
        return None;
    }
    let mut result = String::from("Extrato anterior (não é resumo de IA; pode omitir decisões):\n");
    let budget = limit.saturating_sub(result.chars().count());
    // Select fewer excerpts when the budget is small, so the excerpt actually
    // contains message text rather than only identifiers and role labels.
    let excerpt_count = (budget / 80).clamp(1, 8).min(omitted.len());
    let per_message = budget / excerpt_count;
    // Preserve the first request and the latest omitted context, rather than
    // pretending every message fits a model's finite context window.
    let mut selected: Vec<&Message> = omitted.iter().take(1).collect();
    if excerpt_count > 1 {
        selected.extend(omitted.iter().skip(omitted.len() - (excerpt_count - 1)));
    }
    for message in selected {
        let line = format!(
            "[{} #{}] {}\n",
            message.role,
            prefix(&message.id, 8),
            message.content
        );
        result.push_str(&prefix(&line, per_message));
        result.push('\n');
    }
    Some(prefix(&result, limit))
}

pub(crate) fn build(
    conversation_id: &str,
    messages: Vec<Message>,
    max_chars: usize,
) -> ContextBundle {
    let total: usize = messages.iter().map(|m| m.content.chars().count()).sum();
    if total <= max_chars {
        return ContextBundle {
            conversation_id: conversation_id.into(),
            max_chars,
            included_chars: total,
            truncated: false,
            summary: None,
            messages,
            omitted_message_count: 0,
            truncated_message_ids: vec![],
        };
    }

    let instruction_chars: usize = messages
        .iter()
        .filter(|m| m.role == "system")
        .map(|m| m.content.chars().count())
        .sum();
    let conversation_budget = max_chars.saturating_sub(instruction_chars);
    let summary_budget = if conversation_budget >= 512 {
        (conversation_budget / 4).min(2_000)
    } else {
        0
    };
    let mut remaining = conversation_budget.saturating_sub(summary_budget);
    let mut selected = std::collections::BTreeMap::new();
    for (index, message) in messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == "system")
    {
        selected.insert(index, message.clone());
    }
    let mut truncated_message_ids = Vec::new();
    for (included_regular, (index, message)) in messages
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, m)| m.role != "system")
        .enumerate()
    {
        let length = message.content.chars().count();
        if length > remaining {
            // Keep the end of the newest oversized message; flag that the model
            // is receiving only a fragment, while storage retains the original.
            if included_regular == 0 && remaining > 0 {
                let mut fragment = message.clone();
                fragment.content = tail(&fragment.content, remaining);
                truncated_message_ids.push(fragment.id.clone());
                selected.insert(index, fragment);
            }
            break;
        }
        remaining -= length;
        selected.insert(index, message.clone());
    }
    let omitted: Vec<Message> = messages
        .iter()
        .enumerate()
        .filter(|(index, _)| !selected.contains_key(index))
        .map(|(_, message)| message.clone())
        .collect();
    let summary = extract(&omitted, summary_budget);
    let included: Vec<Message> = selected.into_values().collect();
    let included_chars = included
        .iter()
        .map(|m| m.content.chars().count())
        .sum::<usize>()
        + summary.as_deref().map_or(0, |s| s.chars().count());
    ContextBundle {
        conversation_id: conversation_id.into(),
        max_chars,
        included_chars,
        truncated: true,
        summary,
        messages: included,
        omitted_message_count: omitted.len(),
        truncated_message_ids,
    }
}
