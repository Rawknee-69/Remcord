//! Narrow CSP extension for remcord:// subresources.
//!
//! WebView2 custom schemes participate in CSP (unlike the `bypassCSP`
//! behavior of the previous generation), so the Discord document CSP must
//! explicitly allow the `remcord:` scheme in the directives actual consumers
//! need. Nothing is removed; each listed directive gains the `remcord:`
//! source expression.
//!
//! Traced consumers:
//! - style-src: addStyle("remcord://assets/css/*") — inviteBack, titlebars,
//!   discord.css; editor.html <link> to remcord://assets/css/editor.css.
//! - script-src: injectJS("remcord://assets/js/*") — patchVencordQuickCSS.js
//!   (disableAutogain.js is a known-broken caller: file absent from repo).
//!   remcord://plugins/* Shelter injector entries load as scripts.
//! - img-src: setup.tsx <img>/screenshots/icons, ThemesCard/DetectableCard
//!   icons (Trash/Edit/Folder/UpgradeArrow), setup.css url(mockup.jpg),
//!   splash Update.webp.
//! - media-src: splash Loading.webm <source>.
//! - connect-src: kept narrow for any plugin fetch of local-scheme resources;
//!   no blanket Access-Control-Allow-Origin is added (same-origin
//!   CSS/JS/image/video loading needs no CORS headers).
//! - default-src: extended only when present, as the fallback for the above.
//!
//! font-src, object-src and all other directives are intentionally untouched:
//! no current consumer loads local-scheme fonts or objects.

/// Directives extended with the local-scheme sources, in application order.
pub const REMCORD_CSP_DIRECTIVES: &[&str] = &[
    "default-src",
    "script-src",
    "style-src",
    "img-src",
    "media-src",
    "connect-src",
];

/// Electron's strict replacement policy (`src/discord/extensions/csp.ts`),
/// byte-identical so the native `"strict"` mode enforces exactly what
/// Electron enforces. The host still appends `remcord:` (via
/// [`extend_csp_for_remcord`]) and the badges `connect-src` patch on top,
/// because the native shell serves local-scheme subresources and donor
/// badges exactly like the traced Electron consumers need.
pub const REMCORD_STRICT_CSP: &str = "default-src 'self'; script-src 'self' 'unsafe-inline' 'unsafe-eval' blob: https://*.discord.com https://discord.com https://*.githubusercontent.com https://*.github.com; style-src 'self' 'unsafe-inline' https://*.discord.com https://discord.com https://fonts.googleapis.com; img-src 'self' blob: data: https://*.discord.com https://discord.com https://*.discordapp.com https://cdn.discordapp.com https://*.githubusercontent.com https://*.github.com https://raw.githubusercontent.com; font-src 'self' data: https://fonts.gstatic.com; connect-src 'self' blob: https://*.discord.com https://discord.com wss://*.discord.com wss://gateway.discord.gg https://*.githubusercontent.com https://*.github.com https://api.github.com; media-src 'self' blob: https://*.discord.com https://discord.com; worker-src 'self' blob:; frame-src 'self' https://*.discord.com https://discord.com https://*.youtube.com https://youtube.com https://*.twitch.tv https://open.spotify.com";

/// Outcome of applying one CSP mode to a document response: either a
/// replacement policy or no policy at all (stripped).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CspFilterOutcome {
    /// Emit this policy as the document `content-security-policy`.
    Policy(&'static str),
    /// Emit Discord's own policy with the remcord/badges extensions.
    PreserveDiscord,
    /// Emit no `content-security-policy` at all (Electron `"none"` parity).
    Strip,
}

/// Normalizes a persisted `csp` setting value to the mode the document
/// filter implements. Missing values take the compiled default (Electron
/// parity: `"none"`); present-but-unknown values fail closed to
/// [`CspFilterOutcome::PreserveDiscord`] so a corrupt setting can never
/// silently strip the document policy.
pub fn normalize_csp_mode(persisted: Option<&str>) -> CspFilterOutcome {
    match persisted {
        None => CspFilterOutcome::Strip,
        Some("none") => CspFilterOutcome::Strip,
        Some("strict") => CspFilterOutcome::Policy(REMCORD_STRICT_CSP),
        Some("vanilla") => CspFilterOutcome::PreserveDiscord,
        Some(_) => CspFilterOutcome::PreserveDiscord,
    }
}

/// Extends one document policy with the remcord-scheme allowances plus the
/// traced badges `connect-src` patch. Shared by the vanilla-preservation and
/// strict-replacement paths so both carry identical local-scheme and badges
/// behavior; nothing is ever removed here.
pub fn extend_document_csp(csp: &str) -> String {
    let mut out = extend_csp_for_remcord(csp);
    if !out.contains("badges.vencord.dev") {
        out = if let Some(pos) = out.to_lowercase().find("connect-src") {
            let after = &out[pos..];
            let end = after.find(';').unwrap_or(after.len());
            let directive = after[..end].trim().to_string();
            if directive.contains("badges.vencord.dev") {
                out
            } else {
                out.replacen(
                    &directive,
                    &format!("{directive} https://badges.vencord.dev"),
                    1,
                )
            }
        } else {
            format!("{out}; connect-src https://badges.vencord.dev")
        };
    }
    out
}

/// Appends `remcord:` to each present listed directive that lacks it.
/// Directives absent from the policy are left absent (fallback semantics
/// unchanged); existing sources are preserved byte-for-byte.
pub fn extend_csp_for_remcord(csp: &str) -> String {
    let mut out = csp.to_string();
    for directive in REMCORD_CSP_DIRECTIVES {
        // Find "directive-name" as a whole token (start or after ';').
        let Some(pos) = find_directive(&out, directive) else {
            continue;
        };
        let after = &out[pos..];
        let end = after.find(';').map(|i| pos + i).unwrap_or(out.len());
        let current = out[pos..end].trim_end().to_string();
        if current.contains("remcord:") {
            continue;
        }
        out.replace_range(pos..end, &format!("{current} remcord:"));
    }
    out
}

/// Locates a directive name at a directive boundary (string start or right
/// after ';'), avoiding substring hits inside other directives or sources.
fn find_directive(csp: &str, directive: &str) -> Option<usize> {
    let lower = csp.to_ascii_lowercase();
    let needle = directive.to_ascii_lowercase();
    let mut search_from = 0;
    while let Some(rel) = lower[search_from..].find(&needle) {
        let pos = search_from + rel;
        let at_boundary = pos == 0
            || lower[..pos]
                .rfind(';')
                .is_some_and(|semi| lower[semi + 1..pos].trim().is_empty());
        let followed_by_space_or_end =
            out_char_at(&lower, pos + needle.len()).is_none_or(|c| c == ' ' || c == '\t');
        if at_boundary && followed_by_space_or_end {
            return Some(pos);
        }
        search_from = pos + 1;
        if search_from >= lower.len() {
            break;
        }
    }
    None
}

fn out_char_at(s: &str, byte_idx: usize) -> Option<char> {
    s.get(byte_idx..)?.chars().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extends_only_present_listed_directives() {
        let csp = "default-src 'self'; script-src 'self' https://discord.com; style-src 'self'; img-src 'self' data:; media-src 'self'; connect-src 'self'";
        let extended = extend_csp_for_remcord(csp);
        for d in [
            "default-src",
            "script-src",
            "style-src",
            "img-src",
            "media-src",
            "connect-src",
        ] {
            let pos = extended.find(d).unwrap();
            let end = extended[pos..]
                .find(';')
                .map(|i| pos + i)
                .unwrap_or(extended.len());
            assert!(
                extended[pos..end].contains("remcord:"),
                "{d} must gain remcord:"
            );
        }
    }

    #[test]
    fn absent_directives_stay_absent() {
        let csp = "script-src 'self'; style-src 'self'";
        let extended = extend_csp_for_remcord(csp);
        assert!(!extended.contains("img-src"));
        assert!(!extended.contains("media-src"));
        assert!(extended.contains("remcord:"));
    }

    #[test]
    fn preserves_existing_sources_and_badges_patch() {
        let csp = "connect-src 'self' https://badges.vencord.dev; script-src 'self'";
        let extended = extend_csp_for_remcord(csp);
        assert!(extended.contains("https://badges.vencord.dev"));
        assert!(extended.contains("connect-src 'self' https://badges.vencord.dev remcord:"));
    }

    #[test]
    fn does_not_duplicate_scheme_source() {
        let csp = "script-src 'self' remcord:; style-src 'self'";
        let extended = extend_csp_for_remcord(csp);
        assert_eq!(extended.matches("remcord:").count(), 2);
    }

    #[test]
    fn leaves_unlisted_directives_untouched() {
        let csp = "font-src 'self'; object-src 'none'; script-src 'self'";
        let extended = extend_csp_for_remcord(csp);
        let font_pos = extended.find("font-src").unwrap();
        let font_end = extended[font_pos..]
            .find(';')
            .map(|i| font_pos + i)
            .unwrap_or(extended.len());
        assert!(!extended[font_pos..font_end].contains("remcord:"));
        assert!(!extended.contains("object-src 'none' remcord:"));
    }

    #[test]
    fn no_substring_false_positives() {
        // "script-src-elem" must not be treated as "script-src".
        let csp = "script-src-elem 'self'; script-src 'self'";
        let extended = extend_csp_for_remcord(csp);
        assert!(!extended.contains("script-src-elem 'self' remcord:"));
        assert!(extended.contains("script-src 'self' remcord:"));
    }

    #[test]
    fn strict_policy_matches_the_electron_replacement() {
        // Byte-parity with `src/discord/extensions/csp.ts` REMCORD_CSP: the
        // native "strict" mode must enforce exactly what Electron enforces.
        for directive in [
            "default-src 'self'",
            "script-src 'self' 'unsafe-inline' 'unsafe-eval' blob:",
            "style-src 'self' 'unsafe-inline' https://*.discord.com",
            "img-src 'self' blob: data:",
            "font-src 'self' data: https://fonts.gstatic.com",
            "connect-src 'self' blob:",
            "media-src 'self' blob:",
            "worker-src 'self' blob:",
            "frame-src 'self' https://*.discord.com",
        ] {
            assert!(
                REMCORD_STRICT_CSP.contains(directive),
                "strict policy must carry {directive}"
            );
        }
    }

    #[test]
    fn normalize_pins_each_persisted_mode() {
        assert_eq!(normalize_csp_mode(None), CspFilterOutcome::Strip);
        assert_eq!(normalize_csp_mode(Some("none")), CspFilterOutcome::Strip);
        assert_eq!(
            normalize_csp_mode(Some("strict")),
            CspFilterOutcome::Policy(REMCORD_STRICT_CSP)
        );
        assert_eq!(
            normalize_csp_mode(Some("vanilla")),
            CspFilterOutcome::PreserveDiscord
        );
        // A corrupt/foreign value must fail closed to preserving Discord's
        // policy — never to stripping it.
        for hostile in ["", "NONE", "Strict", "none ", "strict;", "eval", "none\0"] {
            assert_eq!(
                normalize_csp_mode(Some(hostile)),
                CspFilterOutcome::PreserveDiscord,
                "{hostile:?} must preserve"
            );
        }
    }

    #[test]
    fn document_extension_carries_badges_and_scheme_allowances() {
        let extended = extend_document_csp("default-src 'self'; connect-src 'self'");
        assert!(extended.contains("https://badges.vencord.dev"));
        assert!(extended.contains("remcord:"));
        // Idempotent across repeated application.
        let again = extend_document_csp(&extended);
        assert_eq!(again.matches("https://badges.vencord.dev").count(), 1);
        // A policy without connect-src gains the badges directive.
        let bare = extend_document_csp("script-src 'self'");
        assert!(bare.contains("connect-src https://badges.vencord.dev"));
        // The strict replacement extends the same way.
        let strict = extend_document_csp(REMCORD_STRICT_CSP);
        assert!(strict.contains("https://badges.vencord.dev"));
        assert!(strict.contains("remcord:"));
    }
}
