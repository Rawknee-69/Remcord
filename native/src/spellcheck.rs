//! Native spellcheck wiring (F18).
//!
//! Electron parity reference:
//! - `src/discord/window.ts`: `webPreferences.spellcheck` comes from
//!   `getConfig("spellcheck")` (default `true` in `src/common/config.ts`)
//!   and the session languages from
//!   `session.setSpellCheckerLanguages(getConfig("spellcheckLanguage"))`
//!   (default `["en-US"]`).
//! - `src/main.ts` disables the `WinRetrieveSuggestionsOnlyOnDemand`
//!   Chromium feature so suggestions resolve on Windows.
//!
//! WebView2 (wry 0.57) exposes no per-webview spellcheck API, so the
//! host-ownable slice is:
//! - **Validation + normalization** of the authoritative settings pair
//!   (`settings.spellcheck`, `settings.spellcheckLanguage`) into a closed
//!   [`SpellcheckState`] served by the read-only `spellcheck.get_state`
//!   capability. The renderer supplies nothing (empty request by protocol).
//! - **Document-start enforcement** ([`spellcheck_script`]): the validated
//!   boolean + primary language are spliced as JSON literals into
//!   `native/web/spellcheck/apply.js`, which sets `documentElement.lang`
//!   and the `spellcheck` attribute on editable roots (with a
//!   `MutationObserver` for Discord's client-rendered inputs), origin-gated
//!   to the three Discord entry origins and idempotent per document.
//!
//! Safety properties (all pinned by tests):
//! - The only host-supplied values are the closed boolean and one BCP-47
//!   language tag (spliced as JSON, so always a `true`/`false` literal and
//!   a quoted string). No renderer input reaches the script; there is no
//!   bridge/fetch/storage/URL/path/eval surface.
//! - Language tags are validated (`is_valid_language_tag`), deduplicated,
//!   and capped at [`MAX_LANGUAGES`]; invalid entries are dropped and an
//!   empty result falls back to `"en-US"` (Electron's default), so the
//!   script can never carry an empty or hostile `lang`.
//! - The script publishes a frozen `window.__remcordSpellcheck` status
//!   global (`{ installed, enabled, lang }`), so a disabled state is
//!   visible instead of misleading.

/// Template placeholders the host replaces with JSON literals.
pub const ENABLED_PLACEHOLDER: &str = "__REMCORD_SPELLCHECK_ENABLED__";
/// Template placeholders the host replaces with JSON literals.
pub const LANG_PLACEHOLDER: &str = "__REMCORD_SPELLCHECK_LANG__";

/// Maximum languages carried in the state (mirrors
/// `settings_import::MAX_SPELLCHECK_LANGUAGES`; Electron is unbounded).
pub const MAX_LANGUAGES: usize = 16;

/// Maximum BCP-47 tag length in bytes (`"zh-Hant-HK"`-shaped tags fit;
/// anything longer is dropped, never truncated mid-tag).
pub const MAX_LANG_TAG_BYTES: usize = 35;

/// Fallback primary language (Electron's `config.ts` default).
pub const FALLBACK_LANGUAGE: &str = "en-US";

/// Document-start template. The ONLY host-supplied values are the boolean
/// and the primary language tag, spliced as JSON literals.
const SPELLCHECK_APPLY_TEMPLATE: &str = include_str!("../web/spellcheck/apply.js");

/// Validated spellcheck state served by `spellcheck.get_state`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpellcheckState {
    /// Mirrors `settings.spellcheck` (Electron's `webPreferences.spellcheck`).
    pub enabled: bool,
    /// Normalized `settings.spellcheckLanguage` (deduped, capped, valid
    /// tags only; may be empty when the user cleared the list).
    pub languages: Vec<String>,
    /// First normalized language, or [`FALLBACK_LANGUAGE`] when the list is
    /// empty (Electron falls back to the session default the same way).
    pub primary_language: String,
}

/// Minimal BCP-47 shape check: `ll`/`lll` language subtag plus optional
/// `-XXXX` subtags (2–8 alphanumerics each, e.g. `en-US`, `zh-Hant-HK`,
/// `es-419`). Rejects paths, URLs, whitespace, and underscores at parse
/// time; registry membership is Chromium's job, not ours.
pub fn is_valid_language_tag(tag: &str) -> bool {
    if tag.is_empty() || tag.len() > MAX_LANG_TAG_BYTES {
        return false;
    }
    let mut parts = tag.split('-');
    let first = parts.next().unwrap_or_default();
    if !(first.len() == 2 || first.len() == 3) || !first.bytes().all(|b| b.is_ascii_alphabetic()) {
        return false;
    }
    for part in parts {
        if part.len() < 2 || part.len() > 8 || !part.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return false;
        }
    }
    true
}

/// Normalize a raw language list: trim, drop empty/invalid/overlong tags,
/// dedupe case-insensitively preserving first-seen order, cap at
/// [`MAX_LANGUAGES`]. Never fails: a hostile list yields fewer (possibly
/// zero) entries, never an error.
pub fn normalize_languages(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for entry in raw {
        let tag = entry.trim();
        if tag.is_empty() || !is_valid_language_tag(tag) {
            continue;
        }
        let dominated = out
            .iter()
            .any(|seen: &String| seen.eq_ignore_ascii_case(tag));
        if dominated {
            continue;
        }
        out.push(tag.to_owned());
        if out.len() >= MAX_LANGUAGES {
            break;
        }
    }
    out
}

/// Resolve the closed state from the authoritative settings pair.
pub fn resolve_state(enabled: bool, raw_languages: &[String]) -> SpellcheckState {
    let languages = normalize_languages(raw_languages);
    let primary_language = languages
        .first()
        .cloned()
        .unwrap_or_else(|| FALLBACK_LANGUAGE.to_owned());
    SpellcheckState {
        enabled,
        languages,
        primary_language,
    }
}

/// Build the document-start script for one run. The validated boolean and
/// primary language are spliced as JSON literals (`true`/`false` + quoted
/// string), so no host string can break out of its literal. Fails when the
/// template placeholders drift (fail fast instead of shipping a dead layer).
pub fn spellcheck_script(enabled: bool, primary_language: &str) -> Result<String, String> {
    if !SPELLCHECK_APPLY_TEMPLATE.contains(ENABLED_PLACEHOLDER)
        || !SPELLCHECK_APPLY_TEMPLATE.contains(LANG_PLACEHOLDER)
    {
        return Err("[spellcheck] apply template placeholders drifted".to_owned());
    }
    let lang = if is_valid_language_tag(primary_language.trim()) {
        primary_language.trim().to_owned()
    } else {
        FALLBACK_LANGUAGE.to_owned()
    };
    let enabled_json = serde_json::to_string(&enabled).map_err(|e| e.to_string())?;
    let lang_json = serde_json::to_string(&lang).map_err(|e| e.to_string())?;
    let script = SPELLCHECK_APPLY_TEMPLATE
        .replace(ENABLED_PLACEHOLDER, &enabled_json)
        .replace(LANG_PLACEHOLDER, &lang_json);
    if script.contains(ENABLED_PLACEHOLDER) || script.contains(LANG_PLACEHOLDER) {
        return Err("[spellcheck] apply template splice incomplete".to_owned());
    }
    Ok(script)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn f18_accepts_bcp47_tags_and_rejects_hostile_shapes() {
        for valid in ["en-US", "en", "fr-FR", "zh-Hant-HK", "es-419", "pt-BR"] {
            assert!(is_valid_language_tag(valid), "{valid} must validate");
        }
        for invalid in [
            "",
            "e",
            "en_US",
            "en US",
            "../escape",
            "https://x",
            "javascript:1",
            "en-",
            "-US",
            "1234",
            "toolongtag-123456789",
        ] {
            assert!(
                !is_valid_language_tag(invalid),
                "{invalid} must be rejected"
            );
        }
        assert!(!is_valid_language_tag(&"a".repeat(64)));
    }

    #[test]
    fn f18_normalize_dedupes_caps_and_drops_invalid() {
        let raw = strings(&[
            "en-US",
            " EN-us ",
            "fr-FR",
            "../escape",
            "",
            "en_US",
            "de-DE",
        ]);
        assert_eq!(
            normalize_languages(&raw),
            strings(&["en-US", "fr-FR", "de-DE"])
        );
    }

    #[test]
    fn f18_normalize_caps_at_sixteen_preserving_order() {
        let raw: Vec<String> = (0..24).map(|i| format!("en-{i:02}")).collect();
        let normalized = normalize_languages(&raw);
        assert_eq!(normalized.len(), MAX_LANGUAGES);
        assert_eq!(normalized[0], "en-00");
    }

    #[test]
    fn f18_resolve_falls_back_to_en_us_on_empty_list() {
        let state = resolve_state(true, &[]);
        assert!(state.enabled);
        assert!(state.languages.is_empty());
        assert_eq!(state.primary_language, FALLBACK_LANGUAGE);
        let disabled = resolve_state(false, &strings(&["fr-FR"]));
        assert!(!disabled.enabled);
        assert_eq!(disabled.primary_language, "fr-FR");
    }

    #[test]
    fn f18_script_splices_the_closed_pair_as_json_literals() {
        let on = spellcheck_script(true, "fr-FR").expect("must build");
        assert!(on.contains("const spellcheckEnabled = true"));
        assert!(on.contains("const spellcheckLang = \"fr-FR\""));
        assert!(!on.contains(ENABLED_PLACEHOLDER));
        assert!(!on.contains(LANG_PLACEHOLDER));
        assert!(on.contains("__remcordSpellcheck"));
        let off = spellcheck_script(false, "en-US").expect("must build");
        assert!(off.contains("const spellcheckEnabled = false"));
    }

    #[test]
    fn f18_script_falls_back_on_hostile_language() {
        let script = spellcheck_script(true, "../escape").expect("must build");
        assert!(script.contains("const spellcheckLang = \"en-US\""));
        assert!(!script.contains("../escape"));
    }
}
