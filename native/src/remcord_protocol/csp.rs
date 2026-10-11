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
}
