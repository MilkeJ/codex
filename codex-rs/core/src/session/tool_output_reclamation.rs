use super::context_window::ContextWindowTokenStatus;
use crate::context_manager::ContextManager;
use crate::context_manager::ToolOutputReclamation;
use codex_protocol::config_types::AutoCompactTokenLimitScope;
use codex_protocol::models::BaseInstructions;
use tracing::info;

const RECLAMATION_THRESHOLD_PERCENT: i64 = 10;

pub(super) struct ReclamationEvaluation<'a> {
    pub(super) history: &'a ContextManager,
    pub(super) consumed_item_count: usize,
    pub(super) base_instructions: &'a BaseInstructions,
    pub(super) token_status: &'a ContextWindowTokenStatus,
    pub(super) auto_compact_limit_scope: AutoCompactTokenLimitScope,
    pub(super) turn_id: &'a str,
}

pub(super) enum ReclamationDecision {
    RetrySampling,
    ProceedToCompaction {
        reclamation: Option<ToolOutputReclamation>,
    },
}

#[derive(Default)]
pub(super) struct ToolOutputReclamationState {
    active: Option<ToolOutputReclamation>,
}

impl ToolOutputReclamationState {
    pub(super) fn active_plan(&self) -> Option<&ToolOutputReclamation> {
        self.active.as_ref()
    }

    pub(super) fn reset(&mut self) {
        self.active = None;
    }

    pub(super) fn evaluate_before_compaction(
        &mut self,
        evaluation: ReclamationEvaluation<'_>,
    ) -> ReclamationDecision {
        let current_reclamation = self.active.as_ref();
        let planned_reclamation = match current_reclamation {
            Some(current) => current.extend(
                evaluation.history,
                evaluation.consumed_item_count,
                evaluation.base_instructions,
            ),
            None => ToolOutputReclamation::plan(
                evaluation.history,
                evaluation.consumed_item_count,
                evaluation.base_instructions,
            ),
        };
        let Some(reclamation) = planned_reclamation else {
            return ReclamationDecision::ProceedToCompaction {
                reclamation: current_reclamation.cloned(),
            };
        };

        let previous_estimated_saved_tokens = current_reclamation
            .map(ToolOutputReclamation::estimated_saved_tokens)
            .unwrap_or_default();
        let incremental_estimated_saved_tokens = reclamation
            .estimated_saved_tokens()
            .saturating_sub(previous_estimated_saved_tokens);
        let projected_active_tokens = evaluation
            .token_status
            .active_context_tokens
            .saturating_sub(incremental_estimated_saved_tokens);
        let full_context_headroom = evaluation
            .token_status
            .full_context_window_limit
            .map(|limit| limit.saturating_sub(projected_active_tokens));
        let auto_compact_scope_headroom = match evaluation.auto_compact_limit_scope {
            AutoCompactTokenLimitScope::Total => evaluation
                .token_status
                .auto_compact_scope_limit
                .map(|limit| limit.saturating_sub(projected_active_tokens)),
            AutoCompactTokenLimitScope::BodyAfterPrefix => {
                // Reclamation forces a fresh root, so body-after-prefix accounting starts a new
                // prefill baseline on the reclaimed response.
                evaluation.token_status.auto_compact_scope_limit
            }
        };
        let projected_headroom = match (full_context_headroom, auto_compact_scope_headroom) {
            (Some(full), Some(scope)) => full.min(scope),
            (Some(full), None) => full,
            (None, Some(scope)) => scope,
            (None, None) => 0,
        }
        .max(0);
        // Scale the gate from the tightest active boundary. Either the auto-compaction scope or
        // the full model context can force this compaction, so a larger configured scope must not
        // make reclamation miss a smaller hard-cap escape hatch.
        let reclamation_threshold_tokens = [
            evaluation.token_status.auto_compact_scope_limit,
            evaluation.token_status.full_context_window_limit,
        ]
        .into_iter()
        .flatten()
        .min()
        .map(|limit| {
            i64::try_from(
                i128::from(limit.max(0)) * i128::from(RECLAMATION_THRESHOLD_PERCENT) / 100,
            )
            .unwrap_or(i64::MAX)
        });
        let should_reclaim = reclamation_threshold_tokens.is_some_and(|threshold| {
            incremental_estimated_saved_tokens >= threshold && projected_headroom >= threshold
        });

        info!(
            turn_id = %evaluation.turn_id,
            output_count = reclamation.output_count(),
            newly_reclaimed_output_count = reclamation
                .output_count()
                .saturating_sub(
                    current_reclamation
                        .map(ToolOutputReclamation::output_count)
                        .unwrap_or_default()
                ),
            total_estimated_saved_tokens = reclamation.estimated_saved_tokens(),
            incremental_estimated_saved_tokens,
            projected_active_tokens,
            projected_headroom,
            reclamation_threshold_tokens = ?reclamation_threshold_tokens,
            should_reclaim,
            "evaluated tool output reclamation before compaction"
        );

        if should_reclaim {
            self.active = Some(reclamation);
            ReclamationDecision::RetrySampling
        } else {
            // Once a reclaimed request has been sampled, compact the most complete plan so the
            // compaction request cannot resurrect oversized raw tool blobs. A first rejected plan
            // is not applied because no reclaimed request has made it part of the active view.
            let reclamation = current_reclamation.is_some().then_some(reclamation);
            ReclamationDecision::ProceedToCompaction { reclamation }
        }
    }
}
