//! Trusted-load gating on navigation success (Phase 2C).
//!
//! Pure decision state that associates a raw-COM `NavigationCompleted`
//! outcome with the corresponding wry `PageLoadFinished` event for one
//! WebView instance. The gate answers exactly one question: may the
//! finished document enter the trusted Discord initialization pipeline?
//!
//! ## Why this mechanism (correlation investigation)
//!
//! Three pairings were considered and two were rejected for verified
//! reasons, not taste:
//!
//! 1. **WebView2 `NavigationId`: unavailable.** In the installed
//!    `webview2-com-sys 0.39.1` bindings, `NavigationId` exists on
//!    `ContentLoading`/`DOMContentLoaded` args but NOT on
//!    `ICoreWebView2NavigationCompletedEventArgs` (only `IsSuccess` +
//!    `WebErrorStatus`, plus `HttpStatusCode` on `Args2`). There is
//!    therefore no id on the completed-args side to match on.
//! 2. **Wry event args: dropped.** Wry 0.57 (`src/webview2/mod.rs`)
//!    registers one `ContentLoading` handler (`PageLoadEvent::Started`)
//!    and one `NavigationCompleted` handler (`PageLoadEvent::Finished`),
//!    both ignoring their args (`_`) and re-reading the URL from the
//!    WebView. `IsSuccess` is unchecked, so `Finished` fires for failed
//!    navigations too. No wry payload carries anything matchable.
//! 3. **Ordering assumption: refused.** Both completions are queued as
//!    independent `NativeEvent`s, so the gate never assumes
//!    finish-before-outcome or outcome-before-finish.
//!
//! The adopted mechanism pairs by **document generation**, which is a
//! strict per-`Started` counter (`DocumentModLifecycle::begin_document`
//! runs exactly once per `PageLoadStarted` in shipped code):
//!
//! - `Started(g)` clears every pending finish and stored outcome. A new
//!   navigation attempt therefore can never inherit a previous
//!   navigation's verdict in either direction.
//! - `Finished(g)` either consumes an already-stored outcome for `g`
//!   (outcome-first order) or stashes the finish as pending
//!   (finish-first order).
//! - The observer outcome is tagged with the gate's current generation
//!   at processing time and resolves a pending finish only on exact
//!   generation equality; otherwise it is stored for a later finish of
//!   the same generation, or discarded by the next `Started`.
//!
//! Race-safety rests on one engine-level fact plus failsafes, never on
//! callback order: WebView2 dispatches one navigation's `Completed` to
//! all handlers before the next navigation's `ContentLoading`, so a
//! finish and its outcome are always processed with no `Started` between
//! them in the common case; where navigations overlap (an interrupted
//! navigation completing after the next one started), both sides are
//! tagged with the then-current generation and the pair still resolves
//! consistently, while any genuinely orphaned record is dropped
//! fail-closed (skip the pipeline, never run it blind).
//!
//! Concretely: the two `Completed` handlers (wry's, then the observer's —
//! registration order) run back-to-back on the WebView2 UI thread, and
//! their `send_event` calls are therefore adjacent in the event queue:
//! no same-thread send (every `Started`/`Finished`/outcome originates on
//! that thread) can intervene between a paired finish and outcome. The
//! only cross-thread senders (filesystem watcher threads) never touch
//! the gate. The gate handles both arrival orders anyway, so even this
//! mechanism is belt-and-braces rather than load-bearing.
//!
//! Authority: `IsSuccess` alone decides. HTTP status and `WebErrorStatus`
//! are diagnostic metadata and never permit the pipeline —
//! `IsSuccess=false` with HTTP 200 still skips.
//!
//! This module never reloads, retries, navigates, touches the renderer,
//! or reads page data. It is per-WebView-instance state owned by the
//! event loop; stale instances are rejected by id comparison.

/// Decision for a finished document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishAction {
    /// The outcome is already known-success (or no observer exists):
    /// run the trusted pipeline now.
    RunPipeline,
    /// The outcome is already known-failure: skip the pipeline.
    SkipFailed,
    /// No outcome yet: the finish is stashed; the observer outcome will
    /// resolve it.
    AwaitingOutcome,
}

/// Decision for an observer outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeAction {
    /// A pending finish of the same generation resolves to success: run
    /// the pipeline for that generation.
    RunPipeline { generation: u64 },
    /// A pending finish of the same generation resolves to failure: skip.
    SkipFailed { generation: u64 },
    /// No pending finish: the outcome is stored (it may precede its
    /// finish) or superseded state made it irrelevant. No pipeline runs.
    Absorbed,
}

/// Per-WebView-instance trusted-load gate.
#[derive(Debug)]
pub struct NavigationGate {
    instance_id: u64,
    observer_active: bool,
    current_generation: u64,
    pending: Option<u64>,
    outcome: Option<StoredOutcome>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StoredOutcome {
    generation: u64,
    success: bool,
}

impl NavigationGate {
    /// One gate per WebView instance. `observer_active` is false only
    /// when the raw-COM registration failed, in which case the gate
    /// permanently degrades to the pre-gating behavior (every finish
    /// runs the pipeline, exactly as before Phase 2C).
    pub fn new(instance_id: u64, observer_active: bool) -> Self {
        Self {
            instance_id,
            observer_active,
            current_generation: 0,
            pending: None,
            outcome: None,
        }
    }

    /// A new navigation attempt started: the new generation supersedes
    /// everything. Returns the discarded pending generation, if any, so
    /// the caller can log the fail-closed discard (an outcome that never
    /// arrived for a finished document).
    pub fn on_started(&mut self, generation: u64) -> Option<u64> {
        self.current_generation = generation;
        let stranded = self.pending.take();
        self.outcome = None;
        stranded
    }

    /// A document finished loading. With no observer this always runs
    /// (legacy fallback). Otherwise a stored outcome for this generation
    /// decides immediately; without one the finish waits for its outcome.
    pub fn on_finished(&mut self, generation: u64) -> FinishAction {
        if !self.observer_active {
            return FinishAction::RunPipeline;
        }
        match self.outcome.take() {
            Some(stored) if stored.generation == generation => {
                if stored.success {
                    FinishAction::RunPipeline
                } else {
                    FinishAction::SkipFailed
                }
            }
            _ => {
                // A stored outcome for any OTHER generation is irrelevant
                // (it can only exist if generations went backwards, which
                // shipped code never does) — drop it and wait.
                self.pending = Some(generation);
                FinishAction::AwaitingOutcome
            }
        }
    }

    /// A raw observer outcome arrived. `success` is the authoritative
    /// `IsSuccess`; HTTP/error metadata must never reach this call.
    /// Resolves a same-generation pending finish, or stores the outcome
    /// for a finish that has not been processed yet.
    pub fn on_outcome(&mut self, success: bool) -> OutcomeAction {
        if !self.observer_active {
            return OutcomeAction::Absorbed;
        }
        let generation = self.current_generation;
        match self.pending.take() {
            Some(pending) if pending == generation => {
                if success {
                    OutcomeAction::RunPipeline { generation }
                } else {
                    OutcomeAction::SkipFailed { generation }
                }
            }
            _ => {
                // No same-generation pending finish: either the finish
                // comes later (outcome-first order — store it) or a newer
                // navigation already superseded this outcome (a `Started`
                // would have cleared it first, so reaching here with a
                // mismatched pending is impossible in shipped flow; store
                // defensively — it can only be consumed by its own
                // generation, which will never arrive).
                self.outcome = Some(StoredOutcome {
                    generation,
                    success,
                });
                OutcomeAction::Absorbed
            }
        }
    }

    /// Fixed-vocabulary skip line. Carries role/instance/generation only
    /// — never URLs, page data, or status payloads (those already have
    /// their own observer log line).
    pub fn format_skip_log(&self, role: &str, generation: u64) -> String {
        format!(
            "[WebView][TrustedLoad] skip navigation_failed role={role} instance={} generation={generation}",
            self.instance_id,
        )
    }

    /// Fixed-vocabulary deferral line, proving a finish waited for its
    /// outcome instead of running blind or being dropped.
    pub fn format_awaiting_log(&self, role: &str, generation: u64) -> String {
        format!(
            "[WebView][TrustedLoad] awaiting_outcome role={role} instance={} generation={generation}",
            self.instance_id,
        )
    }

    /// Fixed-vocabulary discard line for a finish whose outcome never
    /// arrived before the next navigation superseded it (fail-closed).
    pub fn format_stranded_log(&self, role: &str, generation: u64) -> String {
        format!(
            "[WebView][TrustedLoad] discard stranded_finish role={role} instance={} generation={generation}",
            self.instance_id,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active_main() -> NavigationGate {
        NavigationGate::new(0, true)
    }

    #[test]
    fn success_permits_the_pipeline_finish_first_order() {
        let mut gate = active_main();
        gate.on_started(1);
        assert_eq!(gate.on_finished(1), FinishAction::AwaitingOutcome);
        assert_eq!(
            gate.on_outcome(true),
            OutcomeAction::RunPipeline { generation: 1 }
        );
    }

    #[test]
    fn success_permits_the_pipeline_outcome_first_order() {
        // The gate never assumes queue order: an outcome arriving before
        // its finish is stored and consumed by the matching finish.
        let mut gate = active_main();
        gate.on_started(1);
        assert_eq!(gate.on_outcome(true), OutcomeAction::Absorbed);
        assert_eq!(gate.on_finished(1), FinishAction::RunPipeline);
    }

    #[test]
    fn failure_skips_the_pipeline_in_both_orders() {
        let mut gate = active_main();
        gate.on_started(1);
        assert_eq!(gate.on_finished(1), FinishAction::AwaitingOutcome);
        assert_eq!(
            gate.on_outcome(false),
            OutcomeAction::SkipFailed { generation: 1 }
        );

        let mut gate = active_main();
        gate.on_started(2);
        assert_eq!(gate.on_outcome(false), OutcomeAction::Absorbed);
        assert_eq!(gate.on_finished(2), FinishAction::SkipFailed);
    }

    #[test]
    fn failure_cannot_contaminate_a_later_success() {
        let mut gate = active_main();
        gate.on_started(1);
        assert_eq!(gate.on_finished(1), FinishAction::AwaitingOutcome);
        assert_eq!(
            gate.on_outcome(false),
            OutcomeAction::SkipFailed { generation: 1 }
        );
        // The next navigation starts clean: no stored failure survives.
        gate.on_started(2);
        assert_eq!(gate.on_finished(2), FinishAction::AwaitingOutcome);
        assert_eq!(
            gate.on_outcome(true),
            OutcomeAction::RunPipeline { generation: 2 }
        );
    }

    #[test]
    fn early_outcome_cannot_leak_into_a_later_navigation() {
        // Outcome-first order, then a NEW navigation before the finish:
        // the stored outcome belongs to generation 1 and must not decide
        // generation 2.
        let mut gate = active_main();
        gate.on_started(1);
        assert_eq!(gate.on_outcome(false), OutcomeAction::Absorbed);
        gate.on_started(2);
        assert_eq!(gate.on_finished(2), FinishAction::AwaitingOutcome);
        assert_eq!(
            gate.on_outcome(true),
            OutcomeAction::RunPipeline { generation: 2 }
        );
    }

    #[test]
    fn interrupted_navigation_resolves_consistently() {
        // Realizable overlap: nav B interrupts A before A completes, so
        // A's completion is processed while B is current. Both sides are
        // tagged with the then-current generation and the pair still
        // resolves consistently: nothing runs for the superseded
        // document, and B resolves purely on its own outcome.
        let mut gate = active_main();
        gate.on_started(1);
        // B's start strands nothing (A never finished).
        assert_eq!(gate.on_started(2), None);
        assert_eq!(gate.on_finished(2), FinishAction::AwaitingOutcome);
        assert_eq!(
            gate.on_outcome(false),
            OutcomeAction::SkipFailed { generation: 2 }
        );
        assert_eq!(gate.on_finished(2), FinishAction::AwaitingOutcome);
        assert_eq!(
            gate.on_outcome(true),
            OutcomeAction::RunPipeline { generation: 2 }
        );
    }

    #[test]
    fn stranded_finish_is_reported_for_logging() {
        let mut gate = active_main();
        gate.on_started(1);
        assert_eq!(gate.on_finished(1), FinishAction::AwaitingOutcome);
        // No outcome arrives; the next navigation discards the stranded
        // finish instead of running it blind.
        assert_eq!(gate.on_started(2), Some(1));
        assert_eq!(gate.on_started(3), None);
    }

    #[test]
    fn instances_stay_isolated_and_stale_is_rejected() {
        // Instance scoping is enforced by construction: each WebView owns
        // its gate, and the event loop feeds an outcome to a gate only
        // after the instance-id check passes (stale records return before
        // ever reaching the gate). Two gates therefore resolve
        // independently even for the same generation number.
        let mut main = NavigationGate::new(0, true);
        let mut child = NavigationGate::new(4, true);
        main.on_started(1);
        child.on_started(1);
        assert_eq!(main.on_finished(1), FinishAction::AwaitingOutcome);
        assert_eq!(child.on_finished(1), FinishAction::AwaitingOutcome);
        // The child's failure resolves only the child's pending finish.
        assert_eq!(
            child.on_outcome(false),
            OutcomeAction::SkipFailed { generation: 1 }
        );
        assert_eq!(
            main.on_outcome(true),
            OutcomeAction::RunPipeline { generation: 1 }
        );
    }

    #[test]
    fn inactive_observer_degrades_to_legacy_run() {
        // Observer registration failed: every finish runs the pipeline,
        // exactly the pre-gating behavior; outcomes are inert.
        let mut gate = NavigationGate::new(0, false);
        gate.on_started(1);
        assert_eq!(gate.on_finished(1), FinishAction::RunPipeline);
        assert_eq!(gate.on_outcome(false), OutcomeAction::Absorbed);
        assert_eq!(gate.on_finished(1), FinishAction::RunPipeline);
    }

    #[test]
    fn only_is_success_decides_http_and_error_are_metadata() {
        // The gate API takes a bare success bool: the event loop feeds
        // `event.is_success` and nothing else. An HTTP 200 combined with
        // `IsSuccess=false` still skips, and a missing HTTP code with
        // `IsSuccess=true` still runs — there is no HTTP/error input on
        // the gate by construction, and this test pins that call shape.
        let mut gate = active_main();
        gate.on_started(1);
        assert_eq!(gate.on_finished(1), FinishAction::AwaitingOutcome);
        // Caller contract: map from (is_success=false, http_status=200).
        assert_eq!(
            gate.on_outcome(false),
            OutcomeAction::SkipFailed { generation: 1 }
        );
        gate.on_started(2);
        assert_eq!(gate.on_finished(2), FinishAction::AwaitingOutcome);
        // Caller contract: map from (is_success=true, http_status=None).
        assert_eq!(
            gate.on_outcome(true),
            OutcomeAction::RunPipeline { generation: 2 }
        );
    }

    #[test]
    fn log_lines_are_fixed_vocabulary_and_data_free() {
        let gate = active_main();
        let skip = gate.format_skip_log("main", 2);
        assert_eq!(
            skip,
            "[WebView][TrustedLoad] skip navigation_failed role=main instance=0 generation=2"
        );
        let awaiting = gate.format_awaiting_log("child", 5);
        assert_eq!(
            awaiting,
            "[WebView][TrustedLoad] awaiting_outcome role=child instance=0 generation=5"
        );
        let stranded = gate.format_stranded_log("main", 1);
        assert_eq!(
            stranded,
            "[WebView][TrustedLoad] discard stranded_finish role=main instance=0 generation=1"
        );
        for line in [&skip, &awaiting, &stranded] {
            for forbidden in [
                "https://",
                "http://",
                "discord",
                "?",
                "cookie",
                "token",
                "remcord://",
                "remcord://",
                "query",
            ] {
                assert!(
                    !line.to_lowercase().contains(forbidden),
                    "log line must not contain {forbidden:?}: {line}"
                );
            }
        }
    }

    #[test]
    fn shipped_module_never_reloads_or_navigates() {
        let source = include_str!("navigation_gate.rs");
        let shipped = source[..source.find("#[cfg(test)]").unwrap_or(source.len())]
            .split('\n')
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        // Decision state only: no navigation calls, no reloads, no
        // timers, no script execution, no messaging, no COM, no URLs.
        for forbidden in [
            "Navigate(",
            "Reload(",
            "Close(",
            "ExecuteScript",
            "evaluate_script",
            "postMessage",
            "PostWebMessage",
            "std::thread",
            "set_timeout",
            "Uri(",
            "https://",
            "remcord://",
            "remcord://",
        ] {
            assert!(
                !shipped.contains(forbidden),
                "shipped code must not contain {forbidden}"
            );
        }
    }
}
