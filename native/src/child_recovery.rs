//! Safe child-WebView renderer-exit handling (Phase 2H).
//!
//! Pure policy over the existing `ProcessFailed` event pipeline. No new
//! observer, no second mechanism: the Phase 1 handler still forwards every
//! sanitized record to the event loop, and this module only decides what the
//! loop does with a child-WebView `RenderProcessExited` record.
//!
//! Scope (deliberately narrow):
//! - Eligible: role `child` + kind `RenderProcessExited` only, with an
//!   instance id matching the currently live managed child, while shutdown
//!   has not started.
//! - Never eligible: `RenderProcessUnresponsive`, `FrameRenderProcessExited`,
//!   utility / GPU / sandbox-helper / PPAPI / unknown kinds,
//!   `BrowserProcessExited` (the environment-level observer owns global
//!   browser recovery; a per-WebView browser-process record must never
//!   drive it), navigation failures, bridge errors, every main-WebView
//!   record (main keeps its Phase 2A three-attempt budget), stale records
//!   from an older child instance, and anything arriving after shutdown
//!   begins.
//!
//! Close primitive: the event loop drops the managed single-slot child
//! (unregister both raw-COM subscriptions best-effort, drop the child
//! WebView/controller, close its native window). No reload of the main
//! WebView, no replacement child, no spend of either recovery budget, no
//! retry of an interrupted hidden-ICS download: dropping the child drops
//! its download handlers with it, so an in-flight download can never log
//! completion afterwards.
//!
//! Single-slot guard: the close clears the slot only when it still refers
//! to that exact instance id. A duplicate record from the same exit then
//! finds no live child and collapses into a no-op; a stale record from an
//! older child can never close a newer replacement, including across an
//! environment-level browser recovery that dropped the previous child.
//!
//! Privacy: every log line carries only fixed vocabulary plus role,
//! instance, kind, and reason — no URIs, page source, cookies, tokens, or
//! message bodies can appear here by construction.

use crate::process_failure::{ProcessFailedEvent, ProcessFailedKind, WebViewRole};

/// Outcome of feeding one failure record through the child-exit policy.
/// `decide` is pure: it never mutates budgets, guards, or slots. The event
/// loop performs the close only on [`ChildExitDecision::Close`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildExitDecision {
    /// The record names the live child and its renderer exited: close and
    /// drop exactly that instance. `instance_id` echoes the record so the
    /// caller can re-check the slot before clearing it.
    Close { instance_id: u64 },
    /// Any other shape: diagnostic-only, slot untouched.
    Ignore { reason: ChildExitIgnoreReason },
}

/// Why a record was ignored. Every variant leaves all state untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildExitIgnoreReason {
    /// Shutdown already started: late records must not tear anything down.
    Shutdown,
    /// Not a child record: the main recovery path owns main records.
    NotChild,
    /// A child record whose kind is never a close trigger (unresponsive,
    /// subframe exit, browser-process exit, utility / GPU / sandbox /
    /// PPAPI / unknown processes).
    UnexpectedKind,
    /// No managed child is live: nothing to close (duplicate of an already
    /// closed exit, or a record for a child dropped by browser recovery).
    NoLiveChild,
    /// A record from an older child instance: must never close the live one.
    Stale { expected_instance: u64 },
}

/// Feed one failure record through the child-exit policy.
///
/// `live_child_instance` is the instance id of the currently slotted child,
/// or `None` when no child is live. `shutting_down` is true once the main
/// shutdown path has started. Pure: no budget, guard, or slot is touched.
pub fn decide(
    event: &ProcessFailedEvent,
    live_child_instance: Option<u64>,
    shutting_down: bool,
) -> ChildExitDecision {
    if shutting_down {
        return ChildExitDecision::Ignore {
            reason: ChildExitIgnoreReason::Shutdown,
        };
    }
    if event.role != WebViewRole::Child {
        return ChildExitDecision::Ignore {
            reason: ChildExitIgnoreReason::NotChild,
        };
    }
    if event.kind != ProcessFailedKind::RenderProcessExited {
        return ChildExitDecision::Ignore {
            reason: ChildExitIgnoreReason::UnexpectedKind,
        };
    }
    match live_child_instance {
        None => ChildExitDecision::Ignore {
            reason: ChildExitIgnoreReason::NoLiveChild,
        },
        Some(live) if live == event.instance_id => ChildExitDecision::Close {
            instance_id: event.instance_id,
        },
        Some(expected_instance) => ChildExitDecision::Ignore {
            reason: ChildExitIgnoreReason::Stale { expected_instance },
        },
    }
}

fn format_kind(kind: &ProcessFailedKind) -> String {
    match kind {
        ProcessFailedKind::Unrecognized(value) => format!("Unrecognized({value})"),
        known => known.as_str().to_owned(),
    }
}

fn format_reason(reason: &ChildExitIgnoreReason) -> String {
    match reason {
        ChildExitIgnoreReason::Shutdown => "shutdown".to_owned(),
        ChildExitIgnoreReason::NotChild => "not_child".to_owned(),
        ChildExitIgnoreReason::UnexpectedKind => "unexpected_kind".to_owned(),
        ChildExitIgnoreReason::NoLiveChild => "no_live_child".to_owned(),
        ChildExitIgnoreReason::Stale { expected_instance } => {
            format!("stale expected_instance={expected_instance}")
        }
    }
}

/// Fixed-vocabulary `[Popup][ChildExit]` log line for one policy decision.
/// Carries role/instance/kind plus the close/ignore reason only — no page
/// data.
pub fn format_log(decision: &ChildExitDecision, event: &ProcessFailedEvent) -> String {
    let role = event.role.as_str();
    let instance = event.instance_id;
    let kind = format_kind(&event.kind);
    match decision {
        ChildExitDecision::Close { .. } => {
            format!(
                "[Popup][ChildExit] close role={role} instance={instance} kind={kind} renderer_exited"
            )
        }
        ChildExitDecision::Ignore { reason } => {
            let reason = format_reason(reason);
            format!("[Popup][ChildExit] ignore role={role} instance={instance} kind={kind} reason={reason}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser_recovery::{BrowserExitKind, BrowserProcessExitedEvent, BrowserRecovery};
    use crate::process_failure::{ProcessFailedReason, WebViewRole};
    use crate::recovery::{MainRendererRecovery, MAIN_INSTANCE_ID};
    use std::time::Instant;

    fn child_render_exit(instance_id: u64) -> ProcessFailedEvent {
        ProcessFailedEvent {
            role: WebViewRole::Child,
            instance_id,
            kind: ProcessFailedKind::RenderProcessExited,
            reason: Some(ProcessFailedReason::Crashed),
            exit_code: Some(-1),
        }
    }

    fn child_event_with_kind(instance_id: u64, kind: ProcessFailedKind) -> ProcessFailedEvent {
        ProcessFailedEvent {
            role: WebViewRole::Child,
            instance_id,
            kind,
            reason: None,
            exit_code: None,
        }
    }

    /// Test-only single-slot child model: the live instance plus a pending
    /// hidden-child download flag. `claimed_complete` pins that teardown
    /// never reports a download as finished.
    struct ChildSlot {
        live: Option<u64>,
        pending_download: bool,
        claimed_complete: bool,
        closes: u64,
    }

    impl ChildSlot {
        fn new(live: Option<u64>, pending_download: bool) -> Self {
            Self {
                live,
                pending_download,
                claimed_complete: false,
                closes: 0,
            }
        }

        /// Mirror of the event-loop close arm: close only on `Close` whose
        /// instance still matches the slot; a pending download is dropped,
        /// never claimed complete.
        fn apply(&mut self, decision: &ChildExitDecision) {
            if let ChildExitDecision::Close { instance_id } = decision {
                if self.live == Some(*instance_id) {
                    self.live = None;
                    self.pending_download = false;
                    self.closes += 1;
                }
            }
        }
    }

    #[test]
    fn matching_child_renderer_exit_closes_exactly_once() {
        let live = Some(4);
        let event = child_render_exit(4);
        let decision = decide(&event, live, false);
        assert_eq!(decision, ChildExitDecision::Close { instance_id: 4 });
        assert_eq!(
            format_log(&decision, &event),
            "[Popup][ChildExit] close role=child instance=4 kind=RenderProcessExited renderer_exited"
        );
        // The event-loop close clears the slot first, so the duplicate
        // finds no live child and collapses into a harmless no-op.
        let mut slot = ChildSlot::new(live, false);
        slot.apply(&decision);
        assert_eq!(slot.live, None);
        assert_eq!(slot.closes, 1);
        let duplicate = decide(&event, slot.live, false);
        assert_eq!(
            duplicate,
            ChildExitDecision::Ignore {
                reason: ChildExitIgnoreReason::NoLiveChild,
            }
        );
        slot.apply(&duplicate);
        assert_eq!(slot.live, None);
        assert_eq!(slot.closes, 1);
    }

    #[test]
    fn obsolete_child_instance_cannot_close_current_child() {
        // The live slot moved on (instance 5); a late record from the
        // destroyed instance 3 must not touch it.
        let event = child_render_exit(3);
        let decision = decide(&event, Some(5), false);
        assert_eq!(
            decision,
            ChildExitDecision::Ignore {
                reason: ChildExitIgnoreReason::Stale {
                    expected_instance: 5
                },
            }
        );
        assert_eq!(
            format_log(&decision, &event),
            "[Popup][ChildExit] ignore role=child instance=3 kind=RenderProcessExited reason=stale expected_instance=5"
        );
        let mut slot = ChildSlot::new(Some(5), false);
        slot.apply(&decision);
        assert_eq!(slot.live, Some(5));
        assert_eq!(slot.closes, 0);
    }

    #[test]
    fn main_renderer_exit_still_uses_main_recovery_path() {
        // A main renderer exit is never a child close: the child policy
        // abstains and the main-renderer policy stays eligible.
        let event = ProcessFailedEvent {
            role: WebViewRole::Main,
            ..child_render_exit(MAIN_INSTANCE_ID)
        };
        assert_eq!(
            decide(&event, Some(4), false),
            ChildExitDecision::Ignore {
                reason: ChildExitIgnoreReason::NotChild,
            }
        );
        assert!(MainRendererRecovery::is_eligible(&event));
        let mut recovery = MainRendererRecovery::new(MAIN_INSTANCE_ID);
        let now = Instant::now();
        assert_eq!(
            recovery.decide(&event, now),
            crate::recovery::RecoveryDecision::ReloadRequested { attempt: 1 }
        );
    }

    #[test]
    fn unresponsive_or_subframe_exit_does_not_close_child() {
        for kind in [
            ProcessFailedKind::RenderProcessUnresponsive,
            ProcessFailedKind::FrameRenderProcessExited,
            ProcessFailedKind::UtilityProcessExited,
            ProcessFailedKind::GpuProcessExited,
            ProcessFailedKind::BrowserProcessExited,
            ProcessFailedKind::UnknownProcessExited,
            ProcessFailedKind::Unrecognized(42),
        ] {
            let event = child_event_with_kind(4, kind);
            let decision = decide(&event, Some(4), false);
            assert_eq!(
                decision,
                ChildExitDecision::Ignore {
                    reason: ChildExitIgnoreReason::UnexpectedKind,
                },
                "{kind:?} must never close the child"
            );
            let mut slot = ChildSlot::new(Some(4), false);
            slot.apply(&decision);
            assert_eq!(slot.live, Some(4));
            assert_eq!(slot.closes, 0);
        }
    }

    #[test]
    fn child_browser_process_event_does_not_race_global_recovery() {
        // A per-WebView child browser-process record stays diagnostic-only:
        // the authoritative environment-level path owns global recovery.
        let child_record = child_event_with_kind(4, ProcessFailedKind::BrowserProcessExited);
        assert_eq!(
            decide(&child_record, Some(4), false),
            ChildExitDecision::Ignore {
                reason: ChildExitIgnoreReason::UnexpectedKind,
            }
        );
        // The environment path still decides independently on its own event.
        let mut browser_recovery = BrowserRecovery::new();
        let now = Instant::now();
        let env_event = BrowserProcessExitedEvent {
            browser_pid: 1234,
            kind: BrowserExitKind::Failed,
        };
        assert!(BrowserRecovery::is_eligible(&env_event));
        assert_eq!(
            browser_recovery.decide(&env_event, now),
            crate::browser_recovery::BrowserRecoveryDecision::RecreationRequested { attempt: 1 }
        );
        // Ordering: browser recovery dropped the child (slot empty); a late
        // record from the dead instance cannot close a replacement.
        let mut slot = ChildSlot::new(None, false);
        let late = decide(&child_render_exit(4), slot.live, false);
        assert_eq!(
            late,
            ChildExitDecision::Ignore {
                reason: ChildExitIgnoreReason::NoLiveChild,
            }
        );
        slot.apply(&late);
        slot.live = Some(5);
        let stale = decide(&child_render_exit(4), slot.live, false);
        assert_eq!(
            stale,
            ChildExitDecision::Ignore {
                reason: ChildExitIgnoreReason::Stale {
                    expected_instance: 5
                },
            }
        );
        slot.apply(&stale);
        assert_eq!(slot.live, Some(5));
        assert_eq!(slot.closes, 0);
    }

    #[test]
    fn child_exit_consumes_or_resets_neither_recovery_budget() {
        let mut main_recovery = MainRendererRecovery::new(MAIN_INSTANCE_ID);
        let browser_recovery = BrowserRecovery::new();
        let now = Instant::now();
        // The child exit itself is ineligible for main-renderer recovery.
        let event = child_render_exit(4);
        assert_eq!(
            main_recovery.decide(&event, now),
            crate::recovery::RecoveryDecision::Ineligible
        );
        assert!(!main_recovery.is_in_flight());
        assert_eq!(main_recovery.attempts_in_window(now), 0);
        // The close performs no browser-recovery decision at all.
        assert_eq!(browser_recovery.attempts_in_window(now), 0);
        // A later main renderer exit still gets its full first attempt.
        let main_exit = ProcessFailedEvent {
            role: WebViewRole::Main,
            ..child_render_exit(MAIN_INSTANCE_ID)
        };
        assert_eq!(
            main_recovery.decide(&main_exit, now),
            crate::recovery::RecoveryDecision::ReloadRequested { attempt: 1 }
        );
    }

    #[test]
    fn shutdown_prevents_late_event_handling() {
        let event = child_render_exit(4);
        let decision = decide(&event, Some(4), true);
        assert_eq!(
            decision,
            ChildExitDecision::Ignore {
                reason: ChildExitIgnoreReason::Shutdown,
            }
        );
        assert_eq!(
            format_log(&decision, &event),
            "[Popup][ChildExit] ignore role=child instance=4 kind=RenderProcessExited reason=shutdown"
        );
        let mut slot = ChildSlot::new(Some(4), false);
        slot.apply(&decision);
        assert_eq!(slot.live, Some(4));
        assert_eq!(slot.closes, 0);
    }

    #[test]
    fn pending_hidden_child_download_is_dropped_without_claiming_completion() {
        // A hidden ICS child with an in-flight download: the renderer exit
        // closes the child, the pending download is dropped, and completion
        // is never claimed (no reload, no replacement, no retry).
        let mut slot = ChildSlot::new(Some(2), true);
        let decision = decide(&child_render_exit(2), slot.live, false);
        assert_eq!(decision, ChildExitDecision::Close { instance_id: 2 });
        slot.apply(&decision);
        assert_eq!(slot.live, None);
        assert!(!slot.pending_download);
        assert!(
            !slot.claimed_complete,
            "child teardown must never claim download completion"
        );
        assert_eq!(slot.closes, 1);
        // A navigation failure is not a renderer exit either: it never
        // closes the child and never completes the download.
        let mut nav_slot = ChildSlot::new(Some(2), true);
        let nav_failure = child_event_with_kind(2, ProcessFailedKind::UtilityProcessExited);
        let nav_decision = decide(&nav_failure, nav_slot.live, false);
        assert!(matches!(nav_decision, ChildExitDecision::Ignore { .. }));
        nav_slot.apply(&nav_decision);
        assert_eq!(nav_slot.live, Some(2));
        assert!(nav_slot.pending_download);
        assert!(!nav_slot.claimed_complete);
    }

    #[test]
    fn ignore_reasons_log_fixed_vocabulary() {
        let event = child_render_exit(4);
        for (decision, reason) in [
            (
                ChildExitDecision::Ignore {
                    reason: ChildExitIgnoreReason::NotChild,
                },
                "reason=not_child",
            ),
            (
                ChildExitDecision::Ignore {
                    reason: ChildExitIgnoreReason::UnexpectedKind,
                },
                "reason=unexpected_kind",
            ),
            (
                ChildExitDecision::Ignore {
                    reason: ChildExitIgnoreReason::NoLiveChild,
                },
                "reason=no_live_child",
            ),
        ] {
            let line = format_log(&decision, &event);
            assert!(
                line.ends_with(reason),
                "log line must end with {reason}: {line}"
            );
            assert!(line.starts_with("[Popup][ChildExit] ignore role=child instance=4 "));
        }
    }

    #[test]
    fn shipped_module_stays_policy_only() {
        let source = include_str!("child_recovery.rs");
        let shipped = source[..source.find("#[cfg(test)]").unwrap_or(source.len())]
            .split('\n')
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        // No recovery surface: no navigation or teardown calls, no reload,
        // no recreation, no script execution, no messaging, no budget
        // mutation, no download completion claim, no page-data reads.
        for forbidden in [
            "Navigate(",
            "Reload(",
            "Close(",
            "recreate",
            "RecreationRequested",
            "attempts.push",
            "in_flight = true",
            "ExecuteScript",
            "evaluate_script",
            "postMessage",
            "PostWebMessage",
            "completed file=",
            "ProcessDescription",
            "FrameInfos",
            "Uri(",
            "Source(",
            "Cookie",
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
