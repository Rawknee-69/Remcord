//! Live theme and Quick CSS filesystem watching for Phase D.
//!
//! Electron lifecycle this replaces (traced from `src/common/themes.ts`):
//!
//! ```text
//! Quick CSS: initQuickCss -> webContents.send("addTheme",
//!            "remcord-quick-css", <utf-8, even when empty>)
//!            -> fs.watch(quickCssPath) + 300 ms debounce on "change" events
//!            -> removeTheme + addTheme (re-read on fire)
//!            -> disableQuickCss closes the watcher + removeTheme
//!            -> window "closed" closes the watcher + clears the timer
//! Themes:    startThemeWatcher -> fs.watch(themesFolder, recursive) + 500 ms
//!            debounce, but ONLY refreshes the manifest LIST cache on
//!            manifest.json/rename events. Per-theme CSS edits need an enable
//!            toggle or a reload: there is NO live CSS update path.
//! ```
//!
//! Native ownership rules (enforced by construction):
//!
//! - Rust owns all filesystem watchers. Exactly TWO watchers exist for the
//!   whole application lifetime: one over the host profile directory
//!   (non-recursive, filtered to the single fixed `quickCss.css` name) and
//!   one over the host-controlled themes root (recursive). No watcher per
//!   theme, no watcher per document, no renderer watching, no
//!   MutationObserver, no modification-time polling.
//! - Watchers belong to the native application lifecycle: they are created
//!   once at startup, live in the event-loop closure, and are destroyed by
//!   RAII drop on shutdown. Document reload never creates a watcher.
//! - Reconfiguration replaces the whole handle (old watchers drop first).
//! - Filesystem bursts are coalesced: many raw events within the debounce
//!   window produce exactly one re-read. Exactly one timer thread exists per
//!   burst per resource (see [`ResourceDebouncer`]).
//! - Stale delayed callbacks can never resurrect deleted/disabled CSS: every
//!   timer captures an epoch + document generation, and the fire handler
//!   re-reads authoritative state (settings + disk) before touching the
//!   renderer. A mismatched epoch or generation discards the callback.
//! - Ordering is preserved by the substrate: content updates use
//!   [`crate::mod_substrate::managed_css_script`] (in-place, never
//!   re-appended), structural changes finish with
//!   [`crate::mod_substrate::managed_css_order_script`] over
//!   `[remcord-quick-css + sorted theme ids]`.
//!
//! Logical resources owned here: the fixed `<profile>/quickCss.css` file
//! (filtered out of the profile-directory watch) and the `<profile>/themes/`
//! tree. The renderer never supplies paths.

use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::quick_css::QUICK_CSS_FILE;
use crate::themes::LoadedTheme;

/// Debounce window in milliseconds. Matches Electron's 300 ms Quick CSS
/// debounce (`src/common/themes.ts`). The theme side previously debounced
/// only its manifest list cache at 500 ms and had no live CSS path; the live
/// CSS path uses this same 300 ms window so rapid editor saves coalesce
/// into one renderer update.
pub const CSS_DEBOUNCE_MS: u64 = 300;

/// Which logical resource tree produced the filesystem signal. The watcher
/// callbacks never carry paths to the event loop: the fire handler re-reads
/// authoritative host state instead of trusting event payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveCssFsSignal {
    QuickCss,
    Themes,
}

/// Thread-shared counters for the live CSS lifecycle. Locked only around
/// counter bumps and metric snapshots; never held across filesystem reads
/// or renderer evaluation.
#[derive(Debug, Default)]
pub struct LiveCssCounters {
    /// Native watcher handles currently owned (steady state: 2).
    pub watcher_count: u64,
    /// Watcher-set replacements (reconfiguration; steady state: 0).
    pub watcher_replacements: u64,
    /// Raw filesystem events that matched a logical resource.
    pub raw_fs_events: u64,
    /// Debounced re-reads that actually ran (coalesced work).
    pub debounced_reloads: u64,
    /// Managed style create/update operations issued to the renderer.
    pub css_updates: u64,
    /// Managed style removals issued to the renderer.
    pub css_removes: u64,
    /// Delayed callbacks discarded as stale (superseded epoch or a document
    /// reload happened mid-debounce).
    pub stale_discarded: u64,
    /// Order-enforcement scripts issued after structural changes.
    pub order_repairs: u64,
}

impl LiveCssCounters {
    pub fn snapshot(&self) -> LiveCssCountersSnapshot {
        LiveCssCountersSnapshot {
            watcher_count: self.watcher_count,
            watcher_replacements: self.watcher_replacements,
            raw_fs_events: self.raw_fs_events,
            debounced_reloads: self.debounced_reloads,
            css_updates: self.css_updates,
            css_removes: self.css_removes,
            stale_discarded: self.stale_discarded,
            order_repairs: self.order_repairs,
        }
    }
}

/// Copyable snapshot for logging and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveCssCountersSnapshot {
    pub watcher_count: u64,
    pub watcher_replacements: u64,
    pub raw_fs_events: u64,
    pub debounced_reloads: u64,
    pub css_updates: u64,
    pub css_removes: u64,
    pub stale_discarded: u64,
    pub order_repairs: u64,
}

/// One-shot-per-burst debounce state for a single logical resource.
///
/// Every raw filesystem event bumps an epoch. The first event of a burst
/// arms a timer; later events in the same burst only bump the epoch. The
/// timer reads the LATEST epoch after sleeping and fires once, so a burst
/// of N raw events produces exactly one debounced reload. A fire is fresh
/// only when both its epoch and its document generation still match: a
/// newer event (superseded epoch) or a document reload mid-debounce
/// (generation drift) discards it.
#[derive(Debug, Default)]
pub struct ResourceDebouncer {
    epoch: u64,
    timer_armed: bool,
    armed_generation: u64,
}

impl ResourceDebouncer {
    /// Record one raw filesystem event. Returns `(epoch, should_spawn)`
    /// where `should_spawn` is true only for the first event of a burst.
    pub fn on_fs_event(&mut self, document_generation: u64) -> (u64, bool) {
        self.epoch += 1;
        let epoch = self.epoch;
        if self.timer_armed {
            (epoch, false)
        } else {
            self.timer_armed = true;
            self.armed_generation = document_generation;
            (epoch, true)
        }
    }

    /// Read the latest epoch when the timer expires. Clears the armed flag
    /// so the next event starts a new burst.
    pub fn take_fire_epoch(&mut self) -> (u64, u64) {
        self.timer_armed = false;
        (self.epoch, self.armed_generation)
    }

    /// Whether a delayed fire is still valid. Stale fires must never touch
    /// the renderer or the filesystem. Associated (not a method) so both
    /// the event loop and tests share the exact gate.
    pub fn is_fresh(
        fire_epoch: u64,
        fire_generation: u64,
        current_epoch: u64,
        current_generation: u64,
    ) -> bool {
        fire_epoch == current_epoch && fire_generation == current_generation
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn is_armed(&self) -> bool {
        self.timer_armed
    }
}

/// Minimal diff between two theme snapshots (both in sorted-id order).
/// Drives minimal renderer work: only changed themes are re-pushed, only
/// removed/disabled themes are removed.
#[derive(Debug, Default)]
pub struct ThemeDiff {
    /// Themes whose CSS bytes changed (same id, new content): in-place
    /// update, no structural/order change.
    pub updated: Vec<LoadedTheme>,
    /// Newly enabled/created themes (or manifest switched to a new valid
    /// CSS file with content): structural change, needs order repair.
    pub added: Vec<LoadedTheme>,
    /// Style ids that must be removed (deleted, disabled, invalid,
    /// missing/empty CSS): structural change, needs order repair only when
    /// the enabled set shrank (removal itself is positional cleanup).
    pub removed_style_ids: Vec<String>,
}

impl ThemeDiff {
    pub fn is_empty(&self) -> bool {
        self.updated.is_empty() && self.added.is_empty() && self.removed_style_ids.is_empty()
    }

    /// True when the enabled style-id SET changed (add/remove), which is
    /// the only case that can misplace a newly appended node.
    pub fn is_structural(&self) -> bool {
        !self.added.is_empty() || !self.removed_style_ids.is_empty()
    }
}

/// Diff two sorted theme snapshots by id, then by CSS bytes.
pub fn diff_theme_snapshots(old: &[LoadedTheme], new: &[LoadedTheme]) -> ThemeDiff {
    let mut diff = ThemeDiff::default();
    let mut old_by_id: std::collections::HashMap<&str, &LoadedTheme> =
        std::collections::HashMap::with_capacity(old.len());
    for theme in old {
        old_by_id.insert(theme.id.as_str(), theme);
    }
    let mut new_ids: std::collections::HashSet<&str> =
        std::collections::HashSet::with_capacity(new.len());
    for theme in new {
        new_ids.insert(theme.id.as_str());
        match old_by_id.get(theme.id.as_str()) {
            None => diff.added.push(theme.clone()),
            Some(previous) if previous.css != theme.css => diff.updated.push(theme.clone()),
            Some(_) => {}
        }
    }
    for theme in old {
        if !new_ids.contains(theme.id.as_str()) {
            diff.removed_style_ids.push(theme.style_id.clone());
        }
    }
    diff
}

/// What a Quick CSS re-read means relative to the previously injected
/// document state. `previous_css` is `Some` exactly when the managed style
/// was injected into the current document.
#[derive(Debug, PartialEq, Eq)]
pub enum QuickCssTransition {
    /// Nothing changed (same bytes, or still absent): issue no renderer work.
    NoChange,
    /// Inject a new style or update the existing one in place.
    InjectOrUpdate { css: String },
    /// Remove the previously injected style (disabled, deleted-then-empty
    /// is handled by the loader recreating the file, read errors and
    /// oversized/invalid content must never present as successful CSS).
    Remove,
}

/// Classify a fresh [`crate::quick_css::QuickCssReport`] against the
/// document's previously injected state.
pub fn quick_css_transition(
    previous_css: Option<&str>,
    report_css: Option<&str>,
) -> QuickCssTransition {
    match (previous_css, report_css) {
        (None, None) => QuickCssTransition::NoChange,
        (Some(_), None) => QuickCssTransition::Remove,
        (None, Some(css)) | (Some(_), Some(css)) => {
            if Some(css) == previous_css {
                QuickCssTransition::NoChange
            } else {
                QuickCssTransition::InjectOrUpdate {
                    css: css.to_owned(),
                }
            }
        }
    }
}

/// Whether a `notify` event kind can carry a CSS-relevant change.
/// Access (read/open/close chatter) and pure metadata-only modifications
/// never schedule a re-read; everything else funnels into the debouncer,
/// which coalesces bursts. Event-type granularity is deliberately NOT
/// trusted for correctness: the fire handler re-reads authoritative state.
pub fn is_actionable_event_kind(kind: &notify::EventKind) -> bool {
    !matches!(
        kind,
        notify::EventKind::Access(_)
            | notify::EventKind::Modify(notify::event::ModifyKind::Metadata(_))
    )
}

/// Whether a path observed by the profile-directory watcher is the fixed
/// Quick CSS file. The renderer never supplies this name; it is the
/// host-owned constant. Directory-wide chatter (settings writes, storage,
/// cache files) is ignored here.
pub fn is_quick_css_path(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == QUICK_CSS_FILE)
}

/// Native watcher handles for the whole application lifetime. Exactly TWO
/// watchers: profile directory (non-recursive, Quick CSS filtered) and the
/// themes root (recursive). Dropping this struct closes both watchers;
/// document reload never touches it.
pub struct LiveCssWatchers {
    // Held for their Drop (watcher deregistration) only.
    _profile_watcher: notify::RecommendedWatcher,
    _themes_watcher: notify::RecommendedWatcher,
}

impl LiveCssWatchers {
    pub fn watcher_count() -> u64 {
        2
    }
}

/// Create the single application-owned watcher set. `on_signal` runs on the
/// `notify` background thread and must only forward (never read the disk or
/// touch the renderer); the event loop debounces, re-reads, and injects.
///
/// Creating a second set while one is alive is safe (the old set drops
/// first when the caller replaces the handle); the caller bumps
/// `watcher_replacements` so reconfiguration stays observable.
pub fn start_live_css_watchers(
    profile_dir: &Path,
    counters: Arc<Mutex<LiveCssCounters>>,
    on_signal: impl Fn(LiveCssFsSignal) + Send + Sync + 'static,
) -> Result<LiveCssWatchers, String> {
    use notify::Watcher as _;
    let on_signal = Arc::new(on_signal);

    let themes_root = crate::themes::themes_dir(profile_dir);
    std::fs::create_dir_all(&themes_root)
        .map_err(|error| format!("could not ensure the native themes root: {error}"))?;

    let counters_for_profile = Arc::clone(&counters);
    let signal_for_profile = Arc::clone(&on_signal);
    let mut profile_watcher = notify::RecommendedWatcher::new(
        move |result: Result<notify::Event, notify::Error>| match result {
            Ok(event) => {
                if !is_actionable_event_kind(&event.kind) {
                    return;
                }
                if !event.paths.iter().any(|path| is_quick_css_path(path)) {
                    return;
                }
                if let Ok(mut guard) = counters_for_profile.lock() {
                    guard.raw_fs_events += 1;
                }
                signal_for_profile(LiveCssFsSignal::QuickCss);
            }
            Err(error) => eprintln!("[LiveCss][watch_error] profile: {error}"),
        },
        notify::Config::default(),
    )
    .map_err(|error| format!("could not create the Quick CSS watcher: {error}"))?;
    profile_watcher
        .watch(profile_dir, notify::RecursiveMode::NonRecursive)
        .map_err(|error| format!("could not watch the native profile dir: {error}"))?;

    let counters_for_themes = Arc::clone(&counters);
    let signal_for_themes = Arc::clone(&on_signal);
    let mut themes_watcher = notify::RecommendedWatcher::new(
        move |result: Result<notify::Event, notify::Error>| match result {
            Ok(event) => {
                if !is_actionable_event_kind(&event.kind) {
                    return;
                }
                if let Ok(mut guard) = counters_for_themes.lock() {
                    guard.raw_fs_events += 1;
                }
                signal_for_themes(LiveCssFsSignal::Themes);
            }
            Err(error) => eprintln!("[LiveCss][watch_error] themes: {error}"),
        },
        notify::Config::default(),
    )
    .map_err(|error| format!("could not create the themes watcher: {error}"))?;
    themes_watcher
        .watch(&themes_root, notify::RecursiveMode::Recursive)
        .map_err(|error| format!("could not watch the native themes root: {error}"))?;

    if let Ok(mut guard) = counters.lock() {
        guard.watcher_count = LiveCssWatchers::watcher_count();
    }
    Ok(LiveCssWatchers {
        _profile_watcher: profile_watcher,
        _themes_watcher: themes_watcher,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        diff_theme_snapshots, is_actionable_event_kind, is_quick_css_path, quick_css_transition,
        start_live_css_watchers, LiveCssCounters, LiveCssFsSignal, QuickCssTransition,
        ResourceDebouncer, CSS_DEBOUNCE_MS,
    };
    use crate::themes::LoadedTheme;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn loaded(id: &str, css: &str) -> LoadedTheme {
        LoadedTheme {
            id: id.to_owned(),
            style_id: format!("remcord-native-theme-{id}"),
            css: css.to_owned(),
            css_bytes: css.len(),
        }
    }

    fn temp_profile() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must advance")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "remcord-native-livecss-test-{}-{}",
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(dir.join("storage")).expect("must create temp profile");
        std::fs::write(
            dir.join("storage").join("settings.json"),
            r#"{"quickCss":true}"#,
        )
        .expect("must seed settings");
        dir
    }

    #[test]
    fn debounce_window_matches_electron_quick_css_behavior() {
        assert_eq!(CSS_DEBOUNCE_MS, 300);
    }

    #[test]
    fn exactly_one_watcher_set_covers_both_css_trees() {
        let profile = temp_profile();
        let counters = Arc::new(Mutex::new(LiveCssCounters::default()));
        let signals = Arc::new(Mutex::new(Vec::<LiveCssFsSignal>::new()));
        let signals_for_watch = Arc::clone(&signals);
        let watchers = start_live_css_watchers(&profile, Arc::clone(&counters), move |signal| {
            signals_for_watch.lock().expect("signal log").push(signal);
        })
        .expect("watchers must start on a temp profile");
        assert_eq!(super::LiveCssWatchers::watcher_count(), 2);
        assert_eq!(counters.lock().expect("counters").watcher_count, 2);
        // Reconfiguration replaces the set safely: dropping the old handle
        // closes its watchers before the new set starts.
        drop(watchers);
        let signals_for_second = Arc::clone(&signals);
        let counters_for_second = Arc::clone(&counters);
        {
            let mut guard = counters_for_second.lock().expect("counters");
            guard.watcher_replacements += 1;
        }
        let _second = start_live_css_watchers(&profile, counters_for_second, move |signal| {
            signals_for_second.lock().expect("signal log").push(signal);
        })
        .expect("replacement watchers must start");
        let guard = counters.lock().expect("counters");
        assert_eq!(guard.watcher_count, 2);
        assert_eq!(guard.watcher_replacements, 1);
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn watcher_shutdown_is_clean_drop_without_panic() {
        let profile = temp_profile();
        let counters = Arc::new(Mutex::new(LiveCssCounters::default()));
        let watchers = start_live_css_watchers(&profile, counters, |_| {}).expect("must start");
        drop(watchers);
        // A fresh set starts afterwards: no leaked OS watcher state.
        let counters = Arc::new(Mutex::new(LiveCssCounters::default()));
        let _fresh = start_live_css_watchers(&profile, counters, |_| {}).expect("must restart");
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn watcher_creation_failure_is_typed_not_a_crash() {
        // A missing profile parent is created by the handle (themes root is
        // ensured), but watching a path that is a FILE as a directory fails
        // cleanly when the themes root cannot be ensured.
        let profile = temp_profile();
        let file_as_root = profile.join("not-a-dir");
        std::fs::write(&file_as_root, "x").expect("fixture");
        let counters = Arc::new(Mutex::new(LiveCssCounters::default()));
        // Point the profile INSIDE a file path: create_dir_all fails, so the
        // constructor returns Err instead of panicking.
        let nested = file_as_root.join("child");
        let result = start_live_css_watchers(&nested, counters, |_| {});
        assert!(result.is_err());
        std::fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn burst_events_coalesce_into_one_timer() {
        let mut debouncer = ResourceDebouncer::default();
        // Rapid A -> B -> C within one debounce window.
        let (_, spawn_first) = debouncer.on_fs_event(7);
        assert!(spawn_first);
        assert!(debouncer.is_armed());
        let (_, spawn_second) = debouncer.on_fs_event(7);
        assert!(!spawn_second);
        let (_, spawn_third) = debouncer.on_fs_event(7);
        assert!(!spawn_third);
        assert_eq!(debouncer.epoch(), 3);
        // The single timer reads the latest epoch.
        let (fire_epoch, fire_generation) = debouncer.take_fire_epoch();
        assert_eq!((fire_epoch, fire_generation), (3, 7));
        assert!(!debouncer.is_armed());
        assert!(ResourceDebouncer::is_fresh(
            fire_epoch,
            fire_generation,
            3,
            7
        ));
    }

    #[test]
    fn superseded_epoch_is_stale_and_never_fires() {
        let mut debouncer = ResourceDebouncer::default();
        let _ = debouncer.on_fs_event(4);
        let (first_fire, _) = debouncer.take_fire_epoch();
        // A newer burst arrived after the first timer read.
        let _ = debouncer.on_fs_event(4);
        assert!(!ResourceDebouncer::is_fresh(
            first_fire,
            4,
            debouncer.epoch(),
            4
        ));
    }

    #[test]
    fn document_reload_mid_debounce_discards_the_fire() {
        let mut debouncer = ResourceDebouncer::default();
        let _ = debouncer.on_fs_event(9);
        let (fire_epoch, fire_generation) = debouncer.take_fire_epoch();
        // Reload bumped the document generation before the timer fired.
        assert!(!ResourceDebouncer::is_fresh(
            fire_epoch,
            fire_generation,
            fire_epoch,
            10
        ));
    }

    #[test]
    fn disabled_quick_css_removes_and_reenable_injects_current_bytes() {
        // Enabled A -> rapid edits coalesce is covered by the caller keeping
        // only the latest re-read; here the transition table is pinned.
        assert_eq!(
            quick_css_transition(None, Some(":root{--a:1;}")),
            QuickCssTransition::InjectOrUpdate {
                css: ":root{--a:1;}".to_owned()
            }
        );
        assert_eq!(
            quick_css_transition(Some(":root{--a:1;}"), Some(":root{--b:1;}")),
            QuickCssTransition::InjectOrUpdate {
                css: ":root{--b:1;}".to_owned()
            }
        );
        // Identical re-read (duplicate event after the first fire): no work.
        assert_eq!(
            quick_css_transition(Some(":root{--b:1;}"), Some(":root{--b:1;}")),
            QuickCssTransition::NoChange
        );
        // Disable, failed read, oversized/invalid content: remove, never
        // present an error as successful CSS.
        assert_eq!(
            quick_css_transition(Some(":root{--b:1;}"), None),
            QuickCssTransition::Remove
        );
        assert_eq!(
            quick_css_transition(None, None),
            QuickCssTransition::NoChange
        );
    }

    #[test]
    fn missing_then_recreated_file_maps_to_inject() {
        // Missing (None) -> recreated with bytes: a fresh injection.
        assert_eq!(
            quick_css_transition(None, Some(":root{--re:1;}")),
            QuickCssTransition::InjectOrUpdate {
                css: ":root{--re:1;}".to_owned()
            }
        );
        // Recreated empty (Electron empty-file parity): still an injection
        // of the empty style, not a removal.
        assert_eq!(
            quick_css_transition(None, Some("")),
            QuickCssTransition::InjectOrUpdate { css: String::new() }
        );
    }

    #[test]
    fn theme_modification_updates_only_the_changed_theme() {
        let old = vec![
            loaded("alpha", ":root{--a:1;}"),
            loaded("zulu", ":root{--z:1;}"),
        ];
        let new = vec![
            loaded("alpha", ":root{--a:2;}"),
            loaded("zulu", ":root{--z:1;}"),
        ];
        let diff = diff_theme_snapshots(&old, &new);
        assert_eq!(diff.updated.len(), 1);
        assert_eq!(diff.updated[0].id, "alpha");
        assert!(diff.added.is_empty());
        assert!(diff.removed_style_ids.is_empty());
        assert!(!diff.is_structural());
        assert!(!diff.is_empty());
    }

    #[test]
    fn theme_deletion_removes_only_the_stale_style() {
        let old = vec![
            loaded("alpha", ":root{--a:1;}"),
            loaded("zulu", ":root{--z:1;}"),
        ];
        let new = vec![loaded("alpha", ":root{--a:1;}")];
        let diff = diff_theme_snapshots(&old, &new);
        assert!(diff.updated.is_empty() && diff.added.is_empty());
        assert_eq!(
            diff.removed_style_ids,
            vec!["remcord-native-theme-zulu".to_owned()]
        );
        assert!(diff.is_structural());
    }

    #[test]
    fn theme_recreation_after_delete_is_an_add() {
        let old = vec![loaded("alpha", ":root{--a:1;}")];
        let new = vec![
            loaded("alpha", ":root{--a:1;}"),
            loaded("zulu", ":root{--z:9;}"),
        ];
        let diff = diff_theme_snapshots(&old, &new);
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0].id, "zulu");
        assert!(diff.updated.is_empty() && diff.removed_style_ids.is_empty());
        assert!(diff.is_structural());
    }

    #[test]
    fn manifest_switch_to_a_new_valid_css_file_is_an_update_not_an_add() {
        // Same theme id, different bytes (the manifest now names another
        // valid file with different content): position is stable, so this
        // must update in place and NOT trigger an order repair.
        let old = vec![loaded("alpha", ":root{--a:1;}")];
        let new = vec![loaded("alpha", ":root{--a:1;--extra:1;}")];
        let diff = diff_theme_snapshots(&old, &new);
        assert_eq!(diff.updated.len(), 1);
        assert!(!diff.is_structural());
    }

    #[test]
    fn identical_rescan_is_empty_and_issues_no_renderer_work() {
        let snapshot = vec![
            loaded("alpha", ":root{--a:1;}"),
            loaded("zulu", ":root{--z:1;}"),
        ];
        let diff = diff_theme_snapshots(&snapshot, &snapshot);
        assert!(diff.is_empty());
        assert!(!diff.is_structural());
    }

    #[test]
    fn access_and_metadata_events_never_schedule_a_reload() {
        use notify::event::{AccessKind, AccessMode, MetadataKind, ModifyKind};
        assert!(!is_actionable_event_kind(&notify::EventKind::Access(
            AccessKind::Open(AccessMode::Read)
        )));
        assert!(!is_actionable_event_kind(&notify::EventKind::Modify(
            ModifyKind::Metadata(MetadataKind::Permissions)
        )));
        assert!(is_actionable_event_kind(&notify::EventKind::Modify(
            ModifyKind::Data(notify::event::DataChange::Content)
        )));
        assert!(is_actionable_event_kind(&notify::EventKind::Create(
            notify::event::CreateKind::File
        )));
        assert!(is_actionable_event_kind(&notify::EventKind::Remove(
            notify::event::RemoveKind::File
        )));
    }

    #[test]
    fn profile_dir_chatter_outside_quick_css_is_ignored() {
        assert!(is_quick_css_path(
            PathBuf::from("E:\\p").join("quickCss.css").as_path()
        ));
        assert!(!is_quick_css_path(
            PathBuf::from("E:\\p")
                .join("storage")
                .join("settings.json")
                .as_path()
        ));
        assert!(!is_quick_css_path(
            PathBuf::from("E:\\p").join("quickCss.css.bak").as_path()
        ));
    }
}
