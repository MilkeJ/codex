use super::ContextManager;
use codex_protocol::models::BaseInstructions;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseItem;

const RECLAIMED_TOOL_OUTPUT_MESSAGE: &str =
    "Tool output retired after it was consumed by the model to reclaim context before compaction.";

/// A request-only rewrite of tool outputs that were consumed by an earlier sampling response.
///
/// The session history remains unchanged. Applying this plan to a cloned history produces a
/// smaller fresh-root request or compaction input without changing persisted history.
#[derive(Clone, Debug)]
pub(crate) struct ToolOutputReclamation {
    history_version: u64,
    output_indices: Vec<usize>,
    estimated_saved_tokens: i64,
}

impl ToolOutputReclamation {
    pub(crate) fn plan(
        history: &ContextManager,
        consumed_item_count: usize,
        base_instructions: &BaseInstructions,
    ) -> Option<Self> {
        Self::plan_from(
            history,
            consumed_item_count,
            base_instructions,
            /*existing*/ None,
        )
    }

    pub(crate) fn extend(
        &self,
        history: &ContextManager,
        consumed_item_count: usize,
        base_instructions: &BaseInstructions,
    ) -> Option<Self> {
        if self.history_version != history.history_version() {
            return None;
        }
        Self::plan_from(history, consumed_item_count, base_instructions, Some(self))
    }

    fn plan_from(
        history: &ContextManager,
        consumed_item_count: usize,
        base_instructions: &BaseInstructions,
        existing: Option<&Self>,
    ) -> Option<Self> {
        let mut projected_history = history.clone();
        if let Some(existing) = existing
            && !existing.apply(&mut projected_history)
        {
            return None;
        }
        let estimated_tokens_before =
            projected_history.estimate_token_count_with_base_instructions(base_instructions)?;
        let mut projected_items = projected_history.raw_items().to_vec();
        let new_output_indices = projected_items
            .iter_mut()
            .take(consumed_item_count)
            .enumerate()
            .filter_map(|(index, item)| {
                if existing.is_some_and(|existing| existing.output_indices.contains(&index)) {
                    return None;
                }
                let replacement = reclaimed_output(item)?;
                *item = replacement;
                Some(index)
            })
            .collect::<Vec<_>>();
        if new_output_indices.is_empty() {
            return None;
        }

        projected_history.replace(projected_items);
        let estimated_tokens_after =
            projected_history.estimate_token_count_with_base_instructions(base_instructions)?;
        let incremental_saved_tokens =
            estimated_tokens_before.saturating_sub(estimated_tokens_after);
        if incremental_saved_tokens == 0 {
            return None;
        }

        let mut output_indices = existing
            .map(|existing| existing.output_indices.clone())
            .unwrap_or_default();
        output_indices.extend(new_output_indices);
        output_indices.sort_unstable();
        let estimated_saved_tokens = existing
            .map(Self::estimated_saved_tokens)
            .unwrap_or_default()
            .saturating_add(incremental_saved_tokens);

        Some(Self {
            history_version: history.history_version(),
            output_indices,
            estimated_saved_tokens,
        })
    }

    pub(crate) fn apply(&self, history: &mut ContextManager) -> bool {
        if history.history_version() != self.history_version {
            return false;
        }

        let mut items = history.raw_items().to_vec();
        for index in &self.output_indices {
            let Some(item) = items.get_mut(*index) else {
                return false;
            };
            let Some(replacement) = reclaimed_output(item) else {
                return false;
            };
            *item = replacement;
        }
        history.replace(items);
        true
    }

    pub(crate) fn estimated_saved_tokens(&self) -> i64 {
        self.estimated_saved_tokens
    }

    pub(crate) fn output_count(&self) -> usize {
        self.output_indices.len()
    }
}

fn reclaimed_output(item: &ResponseItem) -> Option<ResponseItem> {
    Some(match item {
        ResponseItem::FunctionCallOutput {
            id,
            call_id,
            output,
            internal_chat_message_metadata_passthrough: metadata,
        } => ResponseItem::FunctionCallOutput {
            id: id.clone(),
            call_id: call_id.clone(),
            output: reclaimed_output_payload(output),
            internal_chat_message_metadata_passthrough: metadata.clone(),
        },
        ResponseItem::CustomToolCallOutput {
            id,
            call_id,
            name,
            output,
            internal_chat_message_metadata_passthrough: metadata,
        } => ResponseItem::CustomToolCallOutput {
            id: id.clone(),
            call_id: call_id.clone(),
            name: name.clone(),
            output: reclaimed_output_payload(output),
            internal_chat_message_metadata_passthrough: metadata.clone(),
        },
        ResponseItem::ToolSearchOutput {
            id,
            call_id,
            status,
            execution,
            internal_chat_message_metadata_passthrough: metadata,
            ..
        } => ResponseItem::ToolSearchOutput {
            id: id.clone(),
            call_id: call_id.clone(),
            status: status.clone(),
            execution: execution.clone(),
            tools: Vec::new(),
            internal_chat_message_metadata_passthrough: metadata.clone(),
        },
        _ => return None,
    })
}

fn reclaimed_output_payload(output: &FunctionCallOutputPayload) -> FunctionCallOutputPayload {
    FunctionCallOutputPayload {
        body: FunctionCallOutputBody::Text(RECLAIMED_TOOL_OUTPUT_MESSAGE.to_string()),
        success: output.success,
    }
}
