//! Native authoritative localization source for the `localization.get_lang`
//! capability.
//!
//! This module mirrors `src/common/lang.ts` without Electron:
//! - Language selection comes from the host-built
//!   `<profile>/storage/lang.json` (`{"lang": "<code>"}`), defaulting to
//!   `"en-US"` when the file is missing or malformed. Two-letter codes are
//!   normalized to `xx-XX` exactly like Electron.
//! - Catalogs are the Electron `assets/lang/*.json` files embedded at compile
//!   time by `native/build.rs`. A selected locale with no catalog falls back
//!   to the whole `en-US` catalog; otherwise missing keys are merged from
//!   `en-US` — the same two rules as `loadLanguage()`.
//! - Lookup returns `entries[key]` or `""` for a missing key, exactly like
//!   Electron's `data[object] ?? ""`. A missing key is NOT an error.
//! - `setLang` is NOT implemented in this milestone; the selection file is
//!   re-read per request so an externally changed language takes effect
//!   without a restart.
//!
//! The renderer supplies only the lookup key (bounded by the protocol). It
//! cannot select files, paths, URLs, or locales.

use std::{collections::HashMap, path::Path, sync::OnceLock};

/// Maximum accepted lookup key length in UTF-8 bytes (mirrors the bridge
/// nonce/request-ID limits).
pub const MAX_LANG_KEY_BYTES: usize = 128;
/// Maximum accepted catalog value length in UTF-8 bytes. The largest real
/// catalog value is well under 1 KiB; this is a safety rail, not a claim.
pub const MAX_LANG_VALUE_BYTES: usize = 4 * 1024;
/// Maximum accepted language code length (generous BCP 47 bound).
pub const MAX_LANG_CODE_BYTES: usize = 32;

include!(concat!(env!("OUT_DIR"), "/remcord_lang/lang_catalogs.rs"));

fn parsed_catalogs() -> &'static HashMap<&'static str, HashMap<String, String>> {
    static CATALOGS: OnceLock<HashMap<&'static str, HashMap<String, String>>> = OnceLock::new();
    CATALOGS.get_or_init(|| {
        let mut map = HashMap::new();
        for (locale, raw) in LANG_CATALOGS {
            let entries: HashMap<String, String> = serde_json::from_str(raw)
                .unwrap_or_else(|error| panic!("embedded {locale} catalog must parse: {error}"));
            for value in entries.values() {
                assert!(
                    value.len() <= MAX_LANG_VALUE_BYTES,
                    "embedded {locale} catalog value exceeds the response bound"
                );
            }
            map.insert(*locale, entries);
        }
        map
    })
}

/// Available embedded locales (test and diagnostic use).
#[cfg(test)]
pub fn available_locales() -> Vec<&'static str> {
    let mut locales: Vec<&'static str> = parsed_catalogs().keys().copied().collect();
    locales.sort_unstable();
    locales
}

/// Resolve the selected language exactly like `resolveLangFromConfig()`:
/// `<profile>/storage/lang.json` → `lang`, else `"en-US"`; two-letter codes
/// become `xx-XX`. Malformed files and oversized codes fall back to `"en-US"`.
pub fn resolve_lang(profile_dir: &Path) -> String {
    let path = profile_dir.join("storage").join("lang.json");
    let raw = std::fs::read(&path).unwrap_or_default();
    let selected = serde_json::from_slice::<serde_json::Value>(&raw)
        .ok()
        .and_then(|value| value.get("lang")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| "en-US".to_owned());
    if selected.len() > MAX_LANG_CODE_BYTES {
        return "en-US".to_owned();
    }
    if selected.len() == 2 {
        return format!("{selected}-{}", selected.to_uppercase());
    }
    if selected.is_empty() {
        return "en-US".to_owned();
    }
    selected
}

pub struct LangLookup {
    /// Localized string, or `""` when the key is absent (Electron parity).
    pub value: String,
    /// The resolved language selection (e.g. `"en-GB"` even when its catalog
    /// is absent and `en-US` strings are served).
    pub lang: String,
    /// Whether the `en-US` catalog served this lookup.
    pub from_fallback: bool,
}

/// Look up one key in the resolved catalog with the Electron fallback rules.
pub fn lookup(profile_dir: &Path, key: &str) -> LangLookup {
    let lang = resolve_lang(profile_dir);
    let catalogs = parsed_catalogs();
    let fallback = catalogs
        .get("en-US")
        .expect("the en-US fallback catalog is embedded");
    let selected = if lang == "en-US" {
        None
    } else {
        catalogs.get(lang.as_str())
    };

    if let Some(entries) = selected {
        if let Some(value) = entries.get(key) {
            return LangLookup {
                value: value.clone(),
                lang,
                from_fallback: false,
            };
        }
    }
    LangLookup {
        value: fallback.get(key).cloned().unwrap_or_default(),
        lang,
        from_fallback: true,
    }
}

/// Validate a renderer-supplied lookup key. Empty, oversized, and non-UTF-8
/// (already excluded by JSON parsing) keys are rejected before any lookup.
pub fn validate_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("localization key must not be empty".to_owned());
    }
    if key.len() > MAX_LANG_KEY_BYTES {
        return Err("localization key exceeds the size limit".to_owned());
    }
    Ok(())
}

/// Maximum number of entries in a merged boot catalog. The real `en-US`
/// catalog holds a few hundred keys; this is a safety rail, not a claim.
pub const MAX_BOOT_I18N_KEYS: usize = 8192;

/// Maximum serialized size of a merged boot catalog in bytes.
pub const MAX_BOOT_I18N_BYTES: usize = 512 * 1024;

/// The full merged translation catalog, mirroring Electron `loadLanguage()`.
pub struct RawLang {
    /// The resolved language selection (e.g. `"en-GB"` even when its catalog
    /// is absent and `en-US` strings are served).
    pub lang: String,
    /// Selected catalog with missing keys filled from `en-US`.
    pub entries: HashMap<String, String>,
}

/// Load the whole merged catalog exactly like Electron `loadLanguage()`
/// (`src/common/lang.ts`): the selected locale's embedded catalog, with keys
/// absent from it filled from the embedded `en-US` catalog. A selected locale
/// with no catalog serves the whole `en-US` catalog (same two rules as
/// [`lookup`]). The selection file is re-read on every call; there is no
/// in-memory cache to invalidate.
pub fn load_raw_lang(profile_dir: &Path) -> Result<RawLang, String> {
    let lang = resolve_lang(profile_dir);
    let catalogs = parsed_catalogs();
    let fallback = catalogs
        .get("en-US")
        .expect("the en-US fallback catalog is embedded");
    let mut entries = fallback.clone();
    if lang != "en-US" {
        if let Some(selected) = catalogs.get(lang.as_str()) {
            for (key, value) in selected {
                entries.insert(key.clone(), value.clone());
            }
        }
    }
    if entries.len() > MAX_BOOT_I18N_KEYS {
        return Err("merged translation catalog exceeds the entry limit".to_owned());
    }
    let serialized_len = serde_json::to_string(&entries)
        .map_err(|error| format!("translation catalog cannot be serialized: {error}"))?
        .len();
    if serialized_len > MAX_BOOT_I18N_BYTES {
        return Err("merged translation catalog exceeds the size limit".to_owned());
    }
    Ok(RawLang { lang, entries })
}

/// Validate a renderer-supplied locale for `setLang`. Non-empty, bounded, and
/// restricted to BCP 47 characters; the value is stored verbatim and
/// normalized on read by [`resolve_lang`], exactly like Electron stores the
/// raw Discord locale string and normalizes two-letter codes on read.
pub fn validate_locale(locale: &str) -> Result<(), String> {
    if locale.is_empty() {
        return Err("locale must not be empty".to_owned());
    }
    if locale.len() > MAX_LANG_CODE_BYTES {
        return Err("locale exceeds the size limit".to_owned());
    }
    if !locale
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("locale contains characters outside [A-Za-z0-9_-]".to_owned());
    }
    Ok(())
}

/// Host-constructed language selection path. The renderer cannot influence
/// this: the profile directory is fixed and the remaining segments are
/// constants.
pub fn lang_file_path(profile_dir: &Path) -> std::path::PathBuf {
    profile_dir.join("storage").join("lang.json")
}

/// Persist a language selection exactly like Electron `setLang()`
/// (`src/common/lang.ts`): `<profile>/storage/lang.json` becomes
/// `{"lang": locale}` (4-space JSON, the same formatting Electron writes),
/// creating `storage/` when absent, via a same-directory temp file plus
/// rename so readers never observe a truncated file. Electron additionally
/// clears its in-memory `currentLanguage`/`languageCache`; the native owner
/// holds no cache (the selection is re-read per call), so there is nothing to
/// invalidate.
pub fn apply_lang(profile_dir: &Path, locale: &str) -> Result<(), String> {
    validate_locale(locale)?;
    let path = lang_file_path(profile_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("language selection cannot be written: {error}"))?;
    }
    // Byte-identical layout to Electron's `JSON.stringify({ lang }, null, 4)`
    // for this single-key document. Exactness is safe: `validate_locale`
    // above restricts the value to characters that need no JSON escaping.
    let serialized = format!("{{\n    \"lang\": \"{locale}\"\n}}");
    let temp_path = path.with_extension("json.tmp");
    if let Err(error) = std::fs::write(&temp_path, serialized.as_bytes()) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!("language selection cannot be written: {error}"));
    }
    if let Err(error) = std::fs::rename(&temp_path, &path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!("language selection cannot be written: {error}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        apply_lang, available_locales, load_raw_lang, lookup, resolve_lang, validate_key,
        validate_locale,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_profile_dir(tag: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time must be available")
            .as_nanos();
        std::env::temp_dir().join(format!("legcord-lang-{tag}-{nanos}-{}", std::process::id()))
    }

    fn seed_lang(profile: &std::path::Path, body: impl AsRef<[u8]>) {
        std::fs::create_dir_all(profile.join("storage")).unwrap();
        std::fs::write(profile.join("storage").join("lang.json"), body).unwrap();
    }

    #[test]
    fn embeds_the_electron_catalogs_with_an_en_us_fallback() {
        let locales = available_locales();
        assert!(locales.contains(&"en-US"));
        assert!(locales.contains(&"de"));
        assert!(locales.len() > 10);
    }

    #[test]
    fn resolves_the_invite_caller_key_like_electron() {
        let profile = unique_profile_dir("invite");
        // No lang.json: Electron default en-US applies.
        let found = lookup(&profile, "invite-goBackToApp");
        assert_eq!(found.value, "Go back to the Discord app");
        assert_eq!(found.lang, "en-US");
    }

    #[test]
    fn missing_keys_return_empty_string_like_electron() {
        let profile = unique_profile_dir("missing");
        let found = lookup(&profile, "remcord-native-no-such-key");
        assert_eq!(found.value, "");
        assert_eq!(found.lang, "en-US");
    }

    #[test]
    fn unknown_locale_falls_back_to_en_us_catalog() {
        // Mirrors production: "en-GB" has no catalog file, so Electron serves
        // en-US strings while reporting the en-GB selection.
        let profile = unique_profile_dir("engb");
        seed_lang(&profile, r#"{"lang":"en-GB"}"#);
        assert_eq!(resolve_lang(&profile), "en-GB");
        let found = lookup(&profile, "invite-goBackToApp");
        assert_eq!(found.value, "Go back to the Discord app");
        assert!(found.from_fallback);
    }

    #[test]
    fn malformed_selection_falls_back_to_en_us() {
        let profile = unique_profile_dir("malformed");
        seed_lang(&profile, b"{not json");
        assert_eq!(resolve_lang(&profile), "en-US");
        seed_lang(&profile, r#"{"lang":""}"#);
        assert_eq!(resolve_lang(&profile), "en-US");
    }

    #[test]
    fn rejects_empty_and_oversized_keys() {
        assert!(validate_key("").is_err());
        assert!(validate_key("a").is_ok());
        assert!(validate_key(&"k".repeat(128)).is_ok());
        assert!(validate_key(&"k".repeat(129)).is_err());
    }

    #[test]
    fn raw_catalog_merges_selected_over_en_us_fallback() {
        let profile = unique_profile_dir("raw-default");
        let raw = load_raw_lang(&profile).expect("the default catalog must load");
        assert_eq!(raw.lang, "en-US");
        assert_eq!(
            raw.entries.get("invite-goBackToApp").map(String::as_str),
            Some("Go back to the Discord app")
        );
        assert!(!raw.entries.is_empty());
    }

    #[test]
    fn raw_catalog_reports_selection_with_fallback_strings() {
        let profile = unique_profile_dir("raw-engb");
        seed_lang(&profile, r#"{"lang":"en-GB"}"#);
        let raw = load_raw_lang(&profile).expect("the fallback catalog must load");
        assert_eq!(raw.lang, "en-GB");
        assert_eq!(
            raw.entries.get("invite-goBackToApp").map(String::as_str),
            Some("Go back to the Discord app")
        );
    }

    #[test]
    fn rejects_empty_oversized_and_evil_locales() {
        assert!(validate_locale("en-US").is_ok());
        assert!(validate_locale("zh-Hans").is_ok());
        assert!(validate_locale("").is_err());
        assert!(validate_locale(&"a".repeat(32)).is_ok());
        assert!(validate_locale(&"a".repeat(33)).is_err());
        assert!(validate_locale("../escape").is_err());
        assert!(validate_locale("en;US").is_err());
        assert!(validate_locale("en US").is_err());
    }

    #[test]
    fn apply_lang_round_trips_through_the_selection_file() {
        let profile = unique_profile_dir("apply");
        apply_lang(&profile, "de").expect("a valid locale must persist");
        // Stored verbatim (Electron parity), normalized on read (`xx-XX`).
        let body = std::fs::read_to_string(profile.join("storage").join("lang.json")).unwrap();
        assert_eq!(body, "{\n    \"lang\": \"de\"\n}");
        assert_eq!(resolve_lang(&profile), "de-DE");
        assert!(
            !profile.join("storage").join("lang.json.tmp").exists(),
            "no temp file may be left behind"
        );
        assert!(apply_lang(&profile, "").is_err());
        assert!(apply_lang(&profile, "../escape").is_err());
    }
}
