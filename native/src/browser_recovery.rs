//! Bounded browser-process recovery (Phase 2G).
//!
//! Separate policy from the main-renderer crash recovery in `recovery.rs`.
//! This module handles abnormal WebView2 browser-process exits by reconstructing
//! a fresh main WebView inside the existing Tao window, using the Phase 2F
//! per-instance helpers. The two budgets are independent and do not share state.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Maximum automatic browser-triggered WebView recreations in a rolling window.
pub const MAX_BROWSER_RECREATIONS: u32 = 2;
/// Rolling window the browser-recreation budget is counted over.
pub const BROWSER_ATTEMPT_WINDOW: Duration = Duration::from_secs(3600);
/// Minimum spacing between two automatic browser recreations.
pub const BROWSER_MIN_ATTEMPT_INTERVAL: Duration = Duration::from_secs(5);

/// Exit kind reported by the environment observer (`ICoreWebView2Environment5::BrowserProcessExited`).
/// Mirrors `COREWEBVIEW2_BROWSER_PROCESS_EXIT_KIND_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserExitKind {
    /// Normal shutdown (user closed all windows, application exiting).
    Normal,
    /// Abnormal termination (crash, OOM, kill, etc.).
    Failed,
    /// Unrecognized numeric value from the runtime.
    Unrecognized(i32),
}

impl BrowserExitKind {
    pub fn from_raw(value: i32) -> Self {
        match value {
            0 => Self::Normal,
            1 => Self::Failed,
            other => Self::Unrecognized(other),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Failed => "Failed",
            Self::Unrecognized(_) => "Unrecognized",
        }
    }

    /// True if this exit kind should trigger recovery.
    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed)
    }
}

/// Sanitized browser-process exit record from the environment observer.
/// Carries no COM objects — only primitive diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrowserProcessExitedEvent {
    /// The browser process ID that exited.
    pub browser_pid: u32,
    /// Classification of the exit.
    pub kind: BrowserExitKind,
}

/// Outcome of feeding one browser-exit event through the recovery policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserRecoveryDecision {
    /// Not a failed browser exit: normal exit or unrecognized kind.
    Ineligible,
    /// A browser recreation is already in flight. Duplicate/stale events collapse here.
    AlreadyInFlight,
    /// A recreation already ran less than [`BROWSER_MIN_ATTEMPT_INTERVAL`] ago.
    Cooldown { retry_in_ms: u128 },
    /// [`MAX_BROWSER_RECREATIONS`] automatic recreations already ran inside the rolling window.
    BudgetExhausted { attempts: u32 },
    /// One browser-process recreation is approved. The attempt is recorded and the
    /// single-flight guard is set; `attempt` is the 1-based position inside the current window.
    RecreationRequested { attempt: u32 },
}

/// Per-process browser-recovery state. Lives on the event loop; scoped to the
/// main WebView slot (there is only one main WebView at a time).
#[derive(Debug)]
pub struct BrowserRecovery {
    in_flight: bool,
    attempts: VecDeque<Instant>,
}

impl BrowserRecovery {
    pub fn new() -> Self {
        Self {
            in_flight: false,
            attempts: VecDeque::new(),
        }
    }

    /// Whether the raw environment exit event is a failed browser exit eligible for recovery.
    /// Normal exits are explicitly ineligible; unrecognized kinds are ineligible (fail closed).
    pub fn is_eligible(event: &BrowserProcessExitedEvent) -> bool {
        event.kind.is_failed()
    }

    /// Feed one browser-exit event through the policy at time `now`.
    /// Prunes aged-out attempts first so the window rolls; records the attempt
    /// and sets the guard only on approval.
    pub fn decide(
        &mut self,
        event: &BrowserProcessExitedEvent,
        now: Instant,
    ) -> BrowserRecoveryDecision {
        if !Self::is_eligible(event) {
            return BrowserRecoveryDecision::Ineligible;
        }
        if self.in_flight {
            return BrowserRecoveryDecision::AlreadyInFlight;
        }
        self.prune(now);
        if self.attempts.len() as u32 >= MAX_BROWSER_RECREATIONS {
            return BrowserRecoveryDecision::BudgetExhausted {
                attempts: self.attempts.len() as u32,
            };
        }
        if let Some(last) = self.attempts.back() {
            let elapsed = now.saturating_duration_since(*last);
            if elapsed < BROWSER_MIN_ATTEMPT_INTERVAL {
                return BrowserRecoveryDecision::Cooldown {
                    retry_in_ms: BROWSER_MIN_ATTEMPT_INTERVAL
                        .saturating_sub(elapsed)
                        .as_millis(),
                };
            }
        }
        self.attempts.push_back(now);
        self.in_flight = true;
        BrowserRecoveryDecision::RecreationRequested {
            attempt: self.attempts.len() as u32,
        }
    }

    /// The approved recreation failed (teardown, construction, or initialization).
    /// The attempt stays counted but the guard releases so a later manual recovery
    /// or another failure-driven decision can proceed. Parked state is implicit:
    /// further automatic decisions will hit `BudgetExhausted` until attempts age out.
    pub fn on_recreation_error(&mut self) {
        self.in_flight = false;
    }

    /// A new main WebView successfully started loading content (`PageLoadStarted`).
    /// The recreation took effect on a live renderer, so the single-flight guard releases.
    /// Idempotent: intermediate loads clear nothing but the guard.
    pub fn on_page_load_started(&mut self) {
        self.in_flight = false;
    }

    /// Whether a browser recreation is currently outstanding.
    #[allow(dead_code)]
    pub fn is_in_flight(&self) -> bool {
        self.in_flight
    }

    /// Automatic attempts currently inside the rolling window at `now`.
    /// Read-only: does not prune, so logging never mutates policy state.
    #[allow(dead_code)]
    pub fn attempts_in_window(&self, now: Instant) -> u32 {
        self.attempts
            .iter()
            .filter(|attempt| now.saturating_duration_since(**attempt) <= BROWSER_ATTEMPT_WINDOW)
            .count() as u32
    }

    fn prune(&mut self, now: Instant) {
        while self
            .attempts
            .front()
            .is_some_and(|attempt| now.saturating_duration_since(*attempt) > BROWSER_ATTEMPT_WINDOW)
        {
            self.attempts.pop_front();
        }
    }
}

impl Default for BrowserRecovery {
    fn default() -> Self {
        Self::new()
    }
}

fn format_browser_exit_kind(kind: &BrowserExitKind) -> String {
    match kind {
        BrowserExitKind::Unrecognized(value) => format!("Unrecognized({value})"),
        known => known.as_str().to_owned(),
    }
}

/// Fixed-vocabulary `[WebView][BrowserRecovery]` log line for one policy decision.
/// Carries PID, kind, and bounded counts only — no page data.
pub fn format_browser_recovery_log(
    decision: &BrowserRecoveryDecision,
    event: &BrowserProcessExitedEvent,
) -> String {
    let pid = event.browser_pid;
    let kind = format_browser_exit_kind(&event.kind);
    match decision {
        BrowserRecoveryDecision::Ineligible => {
            format!("[WebView][BrowserRecovery] skip ineligible pid={pid} kind={kind}")
        }
        BrowserRecoveryDecision::AlreadyInFlight => {
            format!("[WebView][BrowserRecovery] skip already_in_flight pid={pid} kind={kind}")
        }
        BrowserRecoveryDecision::Cooldown { retry_in_ms } => {
            format!(
                "[WebView][BrowserRecovery] skip cooldown pid={pid} kind={kind} retry_in_ms={retry_in_ms}"
            )
        }
        BrowserRecoveryDecision::BudgetExhausted { attempts } => {
            format!(
                "[WebView][BrowserRecovery] parked budget_exhausted pid={pid} kind={kind} attempts={attempts}/{} window_secs={} manual_recovery_required",
                MAX_BROWSER_RECREATIONS,
                BROWSER_ATTEMPT_WINDOW.as_secs(),
            )
        }
        BrowserRecoveryDecision::RecreationRequested { attempt } => {
            format!(
                "[WebView][BrowserRecovery] attempt recreation pid={pid} kind={kind} attempt={attempt}/{}",
                MAX_BROWSER_RECREATIONS,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed_event(pid: u32) -> BrowserProcessExitedEvent {
        BrowserProcessExitedEvent {
            browser_pid: pid,
            kind: BrowserExitKind::Failed,
        }
    }

    fn normal_event(pid: u32) -> BrowserProcessExitedEvent {
        BrowserProcessExitedEvent {
            browser_pid: pid,
            kind: BrowserExitKind::Normal,
        }
    }

    #[test]
    fn only_failed_exits_are_eligible() {
        assert!(BrowserRecovery::is_eligible(&failed_event(1234)));
        assert!(!BrowserRecovery::is_eligible(&normal_event(1234)));
        assert!(!BrowserRecovery::is_eligible(&BrowserProcessExitedEvent {
            browser_pid: 1234,
            kind: BrowserExitKind::Unrecognized(99),
        }));
    }

    #[test]
    fn first_failed_exit_requests_one_recreation() {
        let mut recovery = BrowserRecovery::new();
        let now = Instant::now();
        let decision = recovery.decide(&failed_event(1234), now);
        assert_eq!(
            decision,
            BrowserRecoveryDecision::RecreationRequested { attempt: 1 }
        );
        assert!(recovery.is_in_flight());
        let line = format_browser_recovery_log(&decision, &failed_event(1234));
        assert_eq!(
            line,
            "[WebView][BrowserRecovery] attempt recreation pid=1234 kind=Failed attempt=1/2"
        );
    }

    #[test]
    fn ineligible_events_touch_no_state() {
        let mut recovery = BrowserRecovery::new();
        let now = Instant::now();
        let decision = recovery.decide(&normal_event(1234), now);
        assert_eq!(decision, BrowserRecoveryDecision::Ineligible);
        assert!(!recovery.is_in_flight());
        assert_eq!(recovery.attempts_in_window(now), 0);
    }

    #[test]
    fn overlapping_events_collapse_into_single_flight() {
        let mut recovery = BrowserRecovery::new();
        let now = Instant::now();
        assert_eq!(
            recovery.decide(&failed_event(1234), now),
            BrowserRecoveryDecision::RecreationRequested { attempt: 1 }
        );
        let event = failed_event(1234);
        assert_eq!(
            recovery.decide(&event, now),
            BrowserRecoveryDecision::AlreadyInFlight
        );
        assert_eq!(
            recovery.decide(&event, now + Duration::from_secs(30)),
            BrowserRecoveryDecision::AlreadyInFlight
        );
        assert_eq!(recovery.attempts_in_window(now), 1);
    }

    #[test]
    fn five_second_cooldown_bounds_rapid_recreations() {
        let mut recovery = BrowserRecovery::new();
        let start = Instant::now();
        assert_eq!(
            recovery.decide(&failed_event(1234), start),
            BrowserRecoveryDecision::RecreationRequested { attempt: 1 }
        );
        recovery.on_page_load_started();
        // 4.999 s after the last attempt: still cooling down.
        let early = start + Duration::from_millis(4999);
        let decision = recovery.decide(&failed_event(1234), early);
        assert_eq!(
            decision,
            BrowserRecoveryDecision::Cooldown { retry_in_ms: 1 }
        );
        assert!(!recovery.is_in_flight());
        assert_eq!(recovery.attempts_in_window(start), 1);
        // Exactly 5 s after the last attempt: the minimum interval is met.
        let on_time = start + Duration::from_secs(5);
        assert_eq!(
            recovery.decide(&failed_event(1234), on_time),
            BrowserRecoveryDecision::RecreationRequested { attempt: 2 }
        );
    }

    #[test]
    fn two_recreations_in_one_hour_park_further_recovery() {
        let mut recovery = BrowserRecovery::new();
        let start = Instant::now();
        for (index, offset_secs) in [0, 6].iter().enumerate() {
            let now = start + Duration::from_secs(*offset_secs);
            recovery.on_page_load_started();
            assert_eq!(
                recovery.decide(&failed_event(1234), now),
                BrowserRecoveryDecision::RecreationRequested {
                    attempt: (index + 1) as u32
                }
            );
        }
        recovery.on_page_load_started();
        // Third crash inside the same hour: parked, no recreation.
        let event = failed_event(1234);
        for offset_secs in [12, 60, 1800] {
            let now = start + Duration::from_secs(offset_secs);
            let decision = recovery.decide(&event, now);
            assert_eq!(
                decision,
                BrowserRecoveryDecision::BudgetExhausted { attempts: 2 }
            );
            assert!(!recovery.is_in_flight());
        }
        assert_eq!(recovery.attempts_in_window(start), 2);
        let line = format_browser_recovery_log(
            &BrowserRecoveryDecision::BudgetExhausted { attempts: 2 },
            &event,
        );
        assert_eq!(
            line,
            "[WebView][BrowserRecovery] parked budget_exhausted pid=1234 kind=Failed attempts=2/2 window_secs=3600 manual_recovery_required"
        );
    }

    #[test]
    fn window_rolls_so_old_attempts_age_out() {
        let mut recovery = BrowserRecovery::new();
        let start = Instant::now();
        for offset_secs in [0, 6] {
            recovery.on_page_load_started();
            let now = start + Duration::from_secs(offset_secs);
            assert!(matches!(
                recovery.decide(&failed_event(1234), now),
                BrowserRecoveryDecision::RecreationRequested { .. }
            ));
        }
        recovery.on_page_load_started();
        // One hour and one second after the first attempt it ages out: the
        // window holds attempt 2, so a new crash is attempt 2 again.
        let aged = start + Duration::from_secs(3601);
        assert_eq!(
            recovery.decide(&failed_event(1234), aged),
            BrowserRecoveryDecision::RecreationRequested { attempt: 2 }
        );
    }

    #[test]
    fn recreation_failure_releases_guard_but_keeps_attempt() {
        let mut recovery = BrowserRecovery::new();
        let start = Instant::now();
        assert_eq!(
            recovery.decide(&failed_event(1234), start),
            BrowserRecoveryDecision::RecreationRequested { attempt: 1 }
        );
        recovery.on_recreation_error();
        assert!(!recovery.is_in_flight());
        assert_eq!(recovery.attempts_in_window(start), 1);
        // Cooldown still applies to the next failure-driven decision.
        let retry = start + Duration::from_secs(6);
        assert_eq!(
            recovery.decide(&failed_event(1234), retry),
            BrowserRecoveryDecision::RecreationRequested { attempt: 2 }
        );
    }

    #[test]
    fn log_lines_are_fixed_vocabulary() {
        let event = failed_event(5678);
        assert_eq!(
            format_browser_recovery_log(&BrowserRecoveryDecision::Ineligible, &event),
            "[WebView][BrowserRecovery] skip ineligible pid=5678 kind=Failed"
        );
        assert_eq!(
            format_browser_recovery_log(&BrowserRecoveryDecision::AlreadyInFlight, &event),
            "[WebView][BrowserRecovery] skip already_in_flight pid=5678 kind=Failed"
        );
        assert_eq!(
            format_browser_recovery_log(
                &BrowserRecoveryDecision::Cooldown { retry_in_ms: 123 },
                &event
            ),
            "[WebView][BrowserRecovery] skip cooldown pid=5678 kind=Failed retry_in_ms=123"
        );
        assert_eq!(
            format_browser_recovery_log(&BrowserRecoveryDecision::BudgetExhausted { attempts: 2 }, &event),
            "[WebView][BrowserRecovery] parked budget_exhausted pid=5678 kind=Failed attempts=2/2 window_secs=3600 manual_recovery_required"
        );
        assert_eq!(
            format_browser_recovery_log(
                &BrowserRecoveryDecision::RecreationRequested { attempt: 1 },
                &event
            ),
            "[WebView][BrowserRecovery] attempt recreation pid=5678 kind=Failed attempt=1/2"
        );
    }

    #[test]
    fn shipped_module_never_reloads_or_navigates() {
        let source = include_str!("browser_recovery.rs");
        let shipped = source[..source.find("#[cfg(test)]").unwrap_or(source.len())]
            .split('\n')
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
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
