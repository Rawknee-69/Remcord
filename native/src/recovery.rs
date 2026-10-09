//! Bounded main-renderer crash recovery (Phase 2A).
//!
//! Pure policy over the existing `ProcessFailed` event pipeline. No new
//! observer, no second mechanism: the Phase 1 handler still forwards every
//! sanitized record to the event loop, and this module only decides what the
//! loop does with a main-WebView `RenderProcessExited` record.
//!
//! Scope (deliberately narrow):
//! - Eligible: role `main` + kind `RenderProcessExited` only.
//! - Never eligible: `RenderProcessUnresponsive`, `FrameRenderProcessExited`,
//!   utility / GPU / sandbox-helper / PPAPI / unknown kinds,
//!   `BrowserProcessExited`, navigation failures, bridge errors, adapter
//!   timeouts, and every child-WebView record (children keep Phase 1
//!   observe-only behavior; browser-process recreation is a separate
//!   milestone).
//!
//! Recovery primitive: the existing host-side WebView2 `Reload()` call
//! (`wry::WebView::reload`, i.e. `ICoreWebView2::Reload`). Renderer
//! JavaScript is never used — a dead renderer cannot run script.
//!
//! Budget: at most [`MAX_AUTO_ATTEMPTS`] automatic reloads in a rolling
//! [`ATTEMPT_WINDOW`], with at least [`MIN_ATTEMPT_INTERVAL`] between
//! attempts. All state is scoped to one main WebView instance (no global
//! process-failure counts shared with child WebViews).
//!
//! Single-flight guard: duplicate or overlapping failure records from the
//! same crash collapse into one reload. The guard releases on
//! `PageLoadStarted`, never on `PageLoadFinished`. Load-lifecycle mapping
//! (wry 0.57 `src/webview2/mod.rs`): `Started` is forwarded from WebView2
//! `ContentLoading` (a live renderer actually began loading new content —
//! proof the host reload took effect), while `Finished` is forwarded from
//! `NavigationCompleted` unconditionally (`IsSuccess` is unchecked, so a
//! failed navigation still produces `Finished`). `Finished` therefore cannot
//! prove recovery and must not re-arm or release anything: clearing there
//! would certify failed navigations and re-arm on intermediate completions.
//! If a requested reload never produces `Started` (e.g. the replacement
//! navigation fails first), the guard stays set — fail-closed, no reload
//! loop — until the next user-driven navigation starts loading.
//!
//! When the budget is exhausted the policy parks: no further automatic
//! reloads, one fixed-vocabulary parked log, and a host-owned diagnostic
//! pointing at the existing manual path (the built-in WebView2 browser
//! accelerator keys, F5 / Ctrl+R, which wry leaves enabled and this
//! milestone never touches). No new renderer capability or generic
//! JavaScript endpoint is added.
//!
//! Privacy: every log line carries only fixed vocabulary plus role,
//! instance, kind, bounded counts, and a length-capped host error. No URIs,
//! page source, cookies, tokens, or message bodies can appear here by
//! construction.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::process_failure::{ProcessFailedEvent, ProcessFailedKind, WebViewRole};

/// Maximum automatic reloads inside one rolling window.
pub const MAX_AUTO_ATTEMPTS: u32 = 3;
/// Rolling window the attempt budget is counted over.
pub const ATTEMPT_WINDOW: Duration = Duration::from_secs(3600);
/// Minimum spacing between two automatic attempts.
pub const MIN_ATTEMPT_INTERVAL: Duration = Duration::from_secs(5);
/// Instance id the main WebView always carries (see the observer).
pub const MAIN_INSTANCE_ID: u64 = 0;
/// Cap on host error text embedded in a log line.
pub const MAX_ERROR_CHARS: usize = 256;

/// Outcome of feeding one failure record through the recovery policy.
/// `decide` mutates only on approval (`ReloadRequested` records the attempt
/// and sets the single-flight guard); every other variant leaves state
/// untouched.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RecoveryDecision {
    /// Not a main-renderer exit: wrong role, wrong kind, or both. No
    /// recovery rides on this record; Phase 1 counting/logging is
    /// unaffected.
    Ineligible,
    /// Eligible shape but from an obsolete WebView instance. Stale records
    /// must never spend budget or touch the guard.
    Stale { expected_instance: u64 },
    /// An automatic reload for this WebView is already outstanding.
    /// Duplicate/overlapping records collapse here.
    AlreadyInFlight,
    /// A reload already ran less than [`MIN_ATTEMPT_INTERVAL`] ago.
    Cooldown { retry_in_ms: u128 },
    /// [`MAX_AUTO_ATTEMPTS`] automatic attempts already ran inside the
    /// rolling [`ATTEMPT_WINDOW`]. Parked: no reload until old attempts age
    /// out; the user path stays available.
    BudgetExhausted { attempts: u32 },
    /// One host-side reload is approved. The attempt is recorded and the
    /// single-flight guard is set; `attempt` is the 1-based position inside
    /// the current window.
    ReloadRequested { attempt: u32 },
}

/// Per-main-WebView recovery state. Counters and timestamps live here, on
/// the event loop, scoped to the tracked instance — never in globals shared
/// with child WebViews.
#[derive(Debug)]
pub struct MainRendererRecovery {
    instance_id: u64,
    in_flight: bool,
    attempts: VecDeque<Instant>,
}

impl MainRendererRecovery {
    pub fn new(instance_id: u64) -> Self {
        Self {
            instance_id,
            in_flight: false,
            attempts: VecDeque::new(),
        }
    }

    /// The only failure shape this milestone recovers: a dead main
    /// renderer. Everything else (unresponsive, frame exits, utility / GPU /
    /// sandbox / PPAPI / unknown processes, browser-process exits, child
    /// records) is ineligible by construction.
    pub fn is_eligible(event: &ProcessFailedEvent) -> bool {
        event.role == WebViewRole::Main && event.kind == ProcessFailedKind::RenderProcessExited
    }

    /// Feed one failure record through the policy at time `now`.
    /// Prunes aged-out attempts first so the window rolls; records the
    /// attempt and sets the guard only on approval.
    pub fn decide(&mut self, event: &ProcessFailedEvent, now: Instant) -> RecoveryDecision {
        if !Self::is_eligible(event) {
            return RecoveryDecision::Ineligible;
        }
        if event.instance_id != self.instance_id {
            return RecoveryDecision::Stale {
                expected_instance: self.instance_id,
            };
        }
        if self.in_flight {
            return RecoveryDecision::AlreadyInFlight;
        }
        self.prune(now);
        if self.attempts.len() as u32 >= MAX_AUTO_ATTEMPTS {
            return RecoveryDecision::BudgetExhausted {
                attempts: self.attempts.len() as u32,
            };
        }
        if let Some(last) = self.attempts.back() {
            let elapsed = now.saturating_duration_since(*last);
            if elapsed < MIN_ATTEMPT_INTERVAL {
                return RecoveryDecision::Cooldown {
                    retry_in_ms: MIN_ATTEMPT_INTERVAL.saturating_sub(elapsed).as_millis(),
                };
            }
        }
        self.attempts.push_back(now);
        self.in_flight = true;
        RecoveryDecision::ReloadRequested {
            attempt: self.attempts.len() as u32,
        }
    }

    /// The approved host `Reload()` call itself failed. The attempt stays
    /// counted (an automatic attempt ran), but the guard releases so a
    /// later user-initiated or failure-driven retry can decide again.
    pub fn on_reload_error(&mut self) {
        self.in_flight = false;
    }

    /// A new document began loading content (`PageLoadStarted`, i.e.
    /// WebView2 `ContentLoading`): the requested reload took effect on a
    /// live renderer, so the single-flight guard releases. The attempt
    /// history is untouched — budget and cooldown still bound the next
    /// decision. Deliberately idempotent: intermediate loads clear nothing
    /// but the guard.
    pub fn on_page_load_started(&mut self) {
        self.in_flight = false;
    }

    /// Whether a reload is currently outstanding for this WebView.
    pub fn is_in_flight(&self) -> bool {
        self.in_flight
    }

    /// Automatic attempts currently inside the rolling window at `now`.
    /// Read-only: does not prune, so logging never mutates policy state.
    pub fn attempts_in_window(&self, now: Instant) -> u32 {
        self.attempts
            .iter()
            .filter(|attempt| now.saturating_duration_since(**attempt) <= ATTEMPT_WINDOW)
            .count() as u32
    }

    fn prune(&mut self, now: Instant) {
        while self
            .attempts
            .front()
            .is_some_and(|attempt| now.saturating_duration_since(*attempt) > ATTEMPT_WINDOW)
        {
            self.attempts.pop_front();
        }
    }
}

fn format_kind(kind: &ProcessFailedKind) -> String {
    match kind {
        ProcessFailedKind::Unrecognized(value) => format!("Unrecognized({value})"),
        known => known.as_str().to_owned(),
    }
}

/// Fixed-vocabulary `[WebView][Recovery]` log line for one policy decision.
/// Carries role/instance/kind plus bounded counts only — no page data.
pub fn format_log(decision: &RecoveryDecision, event: &ProcessFailedEvent) -> String {
    let role = event.role.as_str();
    let instance = event.instance_id;
    let kind = format_kind(&event.kind);
    match decision {
        RecoveryDecision::Ineligible => {
            format!(
                "[WebView][Recovery] skip ineligible role={role} instance={instance} kind={kind}"
            )
        }
        RecoveryDecision::Stale { expected_instance } => {
            format!(
                "[WebView][Recovery] skip stale role={role} instance={instance} expected_instance={expected_instance} kind={kind}"
            )
        }
        RecoveryDecision::AlreadyInFlight => {
            format!(
                "[WebView][Recovery] skip already_in_flight role={role} instance={instance} kind={kind}"
            )
        }
        RecoveryDecision::Cooldown { retry_in_ms } => {
            format!(
                "[WebView][Recovery] skip cooldown role={role} instance={instance} kind={kind} retry_in_ms={retry_in_ms}"
            )
        }
        RecoveryDecision::BudgetExhausted { attempts } => {
            format!(
                "[WebView][Recovery] parked budget_exhausted role={role} instance={instance} kind={kind} attempts={attempts}/{} window_secs={} manual_reload_required",
                MAX_AUTO_ATTEMPTS,
                ATTEMPT_WINDOW.as_secs(),
            )
        }
        RecoveryDecision::ReloadRequested { attempt } => {
            format!(
                "[WebView][Recovery] attempt reload role={role} instance={instance} kind={kind} attempt={attempt}/{}",
                MAX_AUTO_ATTEMPTS,
            )
        }
    }
}

/// Fixed-vocabulary log line for the host `Reload()` outcome. A failure
/// embeds only a length-capped, single-line host error — never renderer
/// or page data.
pub fn format_reload_result(ok: bool, error: Option<&str>) -> String {
    if ok {
        return "[WebView][Recovery] reload ok".to_owned();
    }
    let capped: String = error
        .unwrap_or("unknown")
        .chars()
        .take(MAX_ERROR_CHARS)
        .collect();
    let single_line = capped.replace(char::is_control, " ");
    format!("[WebView][Recovery] reload failed error={single_line}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process_failure::{ProcessFailedReason, WebViewRole};

    fn main_render_exit(instance_id: u64) -> ProcessFailedEvent {
        ProcessFailedEvent {
            role: WebViewRole::Main,
            instance_id,
            kind: ProcessFailedKind::RenderProcessExited,
            reason: Some(ProcessFailedReason::Crashed),
            exit_code: Some(-1),
        }
    }

    fn main_event_with_kind(kind: ProcessFailedKind) -> ProcessFailedEvent {
        ProcessFailedEvent {
            role: WebViewRole::Main,
            instance_id: MAIN_INSTANCE_ID,
            kind,
            reason: None,
            exit_code: None,
        }
    }

    #[test]
    fn only_main_render_process_exits_are_eligible() {
        assert!(MainRendererRecovery::is_eligible(&main_render_exit(
            MAIN_INSTANCE_ID
        )));
        // Every other kind is ineligible on the main WebView, including the
        // near-miss shapes a broader policy might have grabbed.
        for kind in [
            ProcessFailedKind::BrowserProcessExited,
            ProcessFailedKind::RenderProcessUnresponsive,
            ProcessFailedKind::FrameRenderProcessExited,
            ProcessFailedKind::UtilityProcessExited,
            ProcessFailedKind::SandboxHelperProcessExited,
            ProcessFailedKind::GpuProcessExited,
            ProcessFailedKind::PpapiPluginProcessExited,
            ProcessFailedKind::PpapiBrokerProcessExited,
            ProcessFailedKind::UnknownProcessExited,
            ProcessFailedKind::Unrecognized(42),
        ] {
            assert!(
                !MainRendererRecovery::is_eligible(&main_event_with_kind(kind)),
                "{kind:?} must be ineligible"
            );
        }
        // Child renderer exits keep observe-only behavior: ineligible.
        let child_exit = ProcessFailedEvent {
            role: WebViewRole::Child,
            ..main_render_exit(1)
        };
        assert!(!MainRendererRecovery::is_eligible(&child_exit));
    }

    #[test]
    fn first_eligible_event_requests_one_reload() {
        let mut recovery = MainRendererRecovery::new(MAIN_INSTANCE_ID);
        let now = Instant::now();
        let decision = recovery.decide(&main_render_exit(MAIN_INSTANCE_ID), now);
        assert_eq!(decision, RecoveryDecision::ReloadRequested { attempt: 1 });
        assert!(recovery.is_in_flight());
        let line = format_log(&decision, &main_render_exit(MAIN_INSTANCE_ID));
        assert_eq!(
            line,
            "[WebView][Recovery] attempt reload role=main instance=0 kind=RenderProcessExited attempt=1/3"
        );
    }

    #[test]
    fn ineligible_events_log_a_skip_and_touch_no_state() {
        let mut recovery = MainRendererRecovery::new(MAIN_INSTANCE_ID);
        let now = Instant::now();
        // Unresponsive main renderer: the deferred unresponsive-process
        // policy must not ride this milestone.
        let unresponsive = main_event_with_kind(ProcessFailedKind::RenderProcessUnresponsive);
        let decision = recovery.decide(&unresponsive, now);
        assert_eq!(decision, RecoveryDecision::Ineligible);
        assert!(!recovery.is_in_flight());
        assert_eq!(recovery.attempts_in_window(now), 0);
        assert_eq!(
            format_log(&decision, &unresponsive),
            "[WebView][Recovery] skip ineligible role=main instance=0 kind=RenderProcessUnresponsive"
        );
        // Child renderer exit: same skip, same untouched state.
        let child_exit = ProcessFailedEvent {
            role: WebViewRole::Child,
            ..main_render_exit(4)
        };
        let decision = recovery.decide(&child_exit, now);
        assert_eq!(decision, RecoveryDecision::Ineligible);
        assert!(!recovery.is_in_flight());
        assert_eq!(recovery.attempts_in_window(now), 0);
    }

    #[test]
    fn overlapping_failures_collapse_into_single_flight() {
        let mut recovery = MainRendererRecovery::new(MAIN_INSTANCE_ID);
        let now = Instant::now();
        assert_eq!(
            recovery.decide(&main_render_exit(MAIN_INSTANCE_ID), now),
            RecoveryDecision::ReloadRequested { attempt: 1 }
        );
        // Duplicate/overlapping records from the same crash cannot trigger
        // repeated reloads while the first is outstanding.
        let event = main_render_exit(MAIN_INSTANCE_ID);
        assert_eq!(
            recovery.decide(&event, now),
            RecoveryDecision::AlreadyInFlight
        );
        assert_eq!(
            recovery.decide(&event, now + Duration::from_secs(30)),
            RecoveryDecision::AlreadyInFlight
        );
        assert_eq!(recovery.attempts_in_window(now), 1);
        assert_eq!(
            format_log(&RecoveryDecision::AlreadyInFlight, &event),
            "[WebView][Recovery] skip already_in_flight role=main instance=0 kind=RenderProcessExited"
        );
    }

    #[test]
    fn load_started_releases_the_guard_without_spending_budget() {
        let mut recovery = MainRendererRecovery::new(MAIN_INSTANCE_ID);
        let start = Instant::now();
        assert_eq!(
            recovery.decide(&main_render_exit(MAIN_INSTANCE_ID), start),
            RecoveryDecision::ReloadRequested { attempt: 1 }
        );
        // New document content loading proves the host reload took effect.
        recovery.on_page_load_started();
        assert!(!recovery.is_in_flight());
        // Repeated/intermediate Started events are idempotent: no extra
        // attempts, no guard churn.
        recovery.on_page_load_started();
        assert_eq!(recovery.attempts_in_window(start), 1);
        // A later crash is a new decision, still inside budget.
        let later = start + Duration::from_secs(6);
        assert_eq!(
            recovery.decide(&main_render_exit(MAIN_INSTANCE_ID), later),
            RecoveryDecision::ReloadRequested { attempt: 2 }
        );
    }

    #[test]
    fn finished_event_must_not_release_or_rearm() {
        // Wry forwards NavigationCompleted as Finished even for failed
        // navigations, so Finished cannot prove recovery. The policy
        // exposes no Finished hook by design; this test pins that the
        // guard survives everything except Started and reload-error
        // cleanup — a bare Finished changes nothing.
        let mut recovery = MainRendererRecovery::new(MAIN_INSTANCE_ID);
        let now = Instant::now();
        assert_eq!(
            recovery.decide(&main_render_exit(MAIN_INSTANCE_ID), now),
            RecoveryDecision::ReloadRequested { attempt: 1 }
        );
        assert!(
            recovery.is_in_flight(),
            "only on_page_load_started or on_reload_error may release the guard"
        );
    }

    #[test]
    fn five_second_cooldown_bounds_rapid_repeated_crashes() {
        let mut recovery = MainRendererRecovery::new(MAIN_INSTANCE_ID);
        let start = Instant::now();
        assert_eq!(
            recovery.decide(&main_render_exit(MAIN_INSTANCE_ID), start),
            RecoveryDecision::ReloadRequested { attempt: 1 }
        );
        recovery.on_page_load_started();
        // 4.999 s after the last attempt: still cooling down.
        let early = start + Duration::from_millis(4999);
        let decision = recovery.decide(&main_render_exit(MAIN_INSTANCE_ID), early);
        assert_eq!(decision, RecoveryDecision::Cooldown { retry_in_ms: 1 });
        assert!(!recovery.is_in_flight());
        assert_eq!(recovery.attempts_in_window(start), 1);
        let event = main_render_exit(MAIN_INSTANCE_ID);
        let line = format_log(&decision, &event);
        assert!(
            line.starts_with(
                "[WebView][Recovery] skip cooldown role=main instance=0 kind=RenderProcessExited retry_in_ms="
            ),
            "{line}"
        );
        // Exactly 5 s after the last attempt: the minimum interval is met.
        let on_time = start + Duration::from_secs(5);
        assert_eq!(
            recovery.decide(&main_render_exit(MAIN_INSTANCE_ID), on_time),
            RecoveryDecision::ReloadRequested { attempt: 2 }
        );
    }

    #[test]
    fn three_attempts_in_one_hour_park_further_recovery() {
        let mut recovery = MainRendererRecovery::new(MAIN_INSTANCE_ID);
        let start = Instant::now();
        for (index, offset_secs) in [0, 6, 12].iter().enumerate() {
            let now = start + Duration::from_secs(*offset_secs);
            recovery.on_page_load_started();
            assert_eq!(
                recovery.decide(&main_render_exit(MAIN_INSTANCE_ID), now),
                RecoveryDecision::ReloadRequested {
                    attempt: (index + 1) as u32
                }
            );
        }
        recovery.on_page_load_started();
        // Fourth crash inside the same hour: parked, no reload, no state
        // growth no matter how many further records arrive.
        let event = main_render_exit(MAIN_INSTANCE_ID);
        for offset_secs in [18, 60, 1800] {
            let now = start + Duration::from_secs(offset_secs);
            let decision = recovery.decide(&event, now);
            assert_eq!(decision, RecoveryDecision::BudgetExhausted { attempts: 3 });
            assert!(!recovery.is_in_flight());
        }
        assert_eq!(recovery.attempts_in_window(start), 3);
        let line = format_log(&RecoveryDecision::BudgetExhausted { attempts: 3 }, &event);
        assert_eq!(
            line,
            "[WebView][Recovery] parked budget_exhausted role=main instance=0 kind=RenderProcessExited attempts=3/3 window_secs=3600 manual_reload_required"
        );
    }

    #[test]
    fn window_rolls_so_old_attempts_age_out() {
        let mut recovery = MainRendererRecovery::new(MAIN_INSTANCE_ID);
        let start = Instant::now();
        for offset_secs in [0, 6, 12] {
            recovery.on_page_load_started();
            let now = start + Duration::from_secs(offset_secs);
            assert!(matches!(
                recovery.decide(&main_render_exit(MAIN_INSTANCE_ID), now),
                RecoveryDecision::ReloadRequested { .. }
            ));
        }
        recovery.on_page_load_started();
        // One hour and one second after the first attempt it ages out: the
        // window holds attempts 2-3, so a new crash is attempt 3 again.
        let aged = start + Duration::from_secs(3601);
        assert_eq!(
            recovery.decide(&main_render_exit(MAIN_INSTANCE_ID), aged),
            RecoveryDecision::ReloadRequested { attempt: 3 }
        );
    }

    #[test]
    fn reload_failure_releases_the_guard_but_keeps_the_attempt() {
        let mut recovery = MainRendererRecovery::new(MAIN_INSTANCE_ID);
        let start = Instant::now();
        assert_eq!(
            recovery.decide(&main_render_exit(MAIN_INSTANCE_ID), start),
            RecoveryDecision::ReloadRequested { attempt: 1 }
        );
        // The host Reload() call failed: safe for a user-initiated retry
        // (guard released) without refunding the automatic attempt.
        recovery.on_reload_error();
        assert!(!recovery.is_in_flight());
        assert_eq!(recovery.attempts_in_window(start), 1);
        // Cooldown still applies to the next failure-driven decision.
        let retry = start + Duration::from_secs(6);
        assert_eq!(
            recovery.decide(&main_render_exit(MAIN_INSTANCE_ID), retry),
            RecoveryDecision::ReloadRequested { attempt: 2 }
        );
    }

    #[test]
    fn stale_instance_records_spend_nothing() {
        let mut recovery = MainRendererRecovery::new(MAIN_INSTANCE_ID);
        let now = Instant::now();
        // An obsolete main-WebView record (a generation this tracker does
        // not own) must not spend budget or set the guard.
        let stale = main_render_exit(7);
        let decision = recovery.decide(&stale, now);
        assert_eq!(
            decision,
            RecoveryDecision::Stale {
                expected_instance: MAIN_INSTANCE_ID
            }
        );
        assert!(!recovery.is_in_flight());
        assert_eq!(recovery.attempts_in_window(now), 0);
        assert_eq!(
            format_log(&decision, &stale),
            "[WebView][Recovery] skip stale role=main instance=7 expected_instance=0 kind=RenderProcessExited"
        );
        // The live instance still decides normally afterwards.
        assert_eq!(
            recovery.decide(&main_render_exit(MAIN_INSTANCE_ID), now),
            RecoveryDecision::ReloadRequested { attempt: 1 }
        );
    }

    #[test]
    fn reload_result_lines_are_fixed_vocabulary_and_capped() {
        assert_eq!(
            format_reload_result(true, None),
            "[WebView][Recovery] reload ok"
        );
        assert_eq!(
            format_reload_result(false, Some("COM error 0x80004005")),
            "[WebView][Recovery] reload failed error=COM error 0x80004005"
        );
        assert_eq!(
            format_reload_result(false, None),
            "[WebView][Recovery] reload failed error=unknown"
        );
        // Host errors are length-capped and flattened: multi-line or
        // oversized diagnostics cannot smuggle unbounded content into logs.
        let long = format!("line1\nline2 {}", "x".repeat(400));
        let line = format_reload_result(false, Some(&long));
        let error = line
            .strip_prefix("[WebView][Recovery] reload failed error=")
            .expect("fixed prefix");
        assert!(!error.contains('\n'));
        assert!(error.chars().count() <= MAX_ERROR_CHARS);
    }
}
