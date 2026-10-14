//! Shelter injector plugin surface for the native shell (F3).
//!
//! This module is the native counterpart of the injector half of
//! `src/discord/preload/mods/shelter.mts`: where Electron declares
//! `SHELTER_INJECTOR_PLUGINS` (a map of plugin id to `[remcord:// URL,
//! visibility]` remote entries) and wraps the Shelter bundle in an IIFE
//! carrying it, the native host declares the same global in local (pinned)
//! form and prepends it to the Shelter execution script.
//!
//! Traced Shelter behavior (the actual `uwu/shelter-builds` bundle, not
//! declarations): at the end of its async boot, Shelter iterates a global
//! `SHELTER_INJECTOR_PLUGINS` map (when present; absence skips the injector
//! entirely) and feeds each entry through its loader-plugin registrar:
//! - Array entries `[srcUrl, {isVisible, allowedActions}]` are REMOTE: the
//!   registrar fetches `plugin.json` + `plugin.js` from the source URL and
//!   enables update checks against it.
//! - Object entries `{js, manifest, update, injectorIntegration}` are LOCAL:
//!   the registrar validates the object shape (`js` string, `manifest`
//!   object, `update` boolean) and pins it (local, no update checks).
//! - Either form requires `injectorIntegration = {isVisible: bool,
//!   allowedActions: object}`; missing members throw. Registered entries
//!   start with the next boot pass (`on: true` unless toggling is allowed
//!   AND the plugin is visible).
//!
//! Native contract (deliberate, documented differences from Electron):
//! - LOCAL form only, for exactly two ids: `"remcord-arrpc"` and
//!   `"remcord-power"`. The artifacts are build-embedded (compile-time
//!   `ts-out` via `native/build.rs`, same pipeline as the settings/titlebar
//!   artifacts) and JSON-escaped into the declaration — never interpolated
//!   raw, never fetched, never a renderer-supplied URL. There is no network
//!   fetch in the injector path and no dependence on loose `ts-out` files
//!   at runtime (the installer ships only the executable).
//! - `"remcord-settings"` and `"remcord-titlebar"` are EXCLUDED: the host
//!   starts those ids itself from build-embedded artifacts after Shelter's
//!   marker (remove-first idempotent takeover). Claiming them here would
//!   double-register the ids and fetch loose files that do not ship.
//! - `"remcord-screenshare"` is EXCLUDED: its `onLoad` calls the Electron
//!   `window.remcord.screenshare.getSources` IPC surface unconditionally,
//!   which does not exist natively (deferred with F9 screenshare parity;
//!   Discord's Go Live modal plus the OS system picker own `getDisplayMedia`).
//!   Including it would guarantee a loud load failure on every launch.
//! - The touch-bar entry is darwin-only upstream; native is Windows-only.
//! - The declaration is a top-level `const` (Electron parity for the
//!   wrapper shape), evaluated once per document ahead of the Shelter
//!   bundle in the same script, so the global is in place before Shelter's
//!   boot reads it. Realm-per-document semantics (proven by the existing
//!   per-document managed styles and markers) make the redeclaration safe.
//!
//! The renderer supplies nothing here: ids, sources, manifests, and
//! visibility are all host constants.

include!(concat!(
    env!("OUT_DIR"),
    "/remcord_injector_plugins/remcord-arrpc.rs"
));
include!(concat!(
    env!("OUT_DIR"),
    "/remcord_injector_plugins/remcord-power.rs"
));

/// Maximum accepted injector artifact size in bytes (mirrors the `build.rs`
/// embed limit so the runtime invariant cannot drift from the build
/// invariant).
pub const MAX_INJECTOR_PLUGIN_BYTES: usize = 256 * 1024;

/// Structural members every injector artifact must surface. Must stay
/// identical to the `build.rs` marker set; both are asserted, so a pipeline
/// change dropping the Shelter lifecycle entry fails loudly.
pub const INJECTOR_PLUGIN_BOOT_MARKERS: &[&str] = &["onLoad"];

/// Exact injector-map keys served, in map order. Electron's full map also
/// carries `remcord-settings`, `remcord-titlebar`, and `remcord-screenshare`
/// (plus darwin-only `remcord-touchbar`); see the module docs for why those
/// ids are excluded natively.
pub const INJECTOR_PLUGIN_IDS: &[&str] = &["remcord-arrpc", "remcord-power"];

/// Expected manifest display names, in the same order as
/// [`INJECTOR_PLUGIN_IDS`]. Pinned at build time and re-checked here.
pub const INJECTOR_PLUGIN_EXPECTED_NAMES: &[&str] = &["Remcord RPC", "Remcord Power Management"];

/// The exact global Shelter's boot reads. Declared as a top-level `const`
/// ahead of the bundle (Electron's wrapper shape).
pub const INJECTOR_GLOBAL: &str = "SHELTER_INJECTOR_PLUGINS";

/// Host-controlled handle to one embedded injector artifact. No renderer
/// input can influence which artifact this resolves to.
pub struct InjectorPluginArtifact {
    /// Injector-map key (Electron's exact id).
    pub id: &'static str,
    /// Exact `plugin.js` text.
    pub js: &'static str,
    /// Lowercase hex SHA-256 over the exact embedded bytes.
    pub sha256: &'static str,
    /// Byte length of the embedded text.
    pub bytes: usize,
    /// Plugin name from `plugin.json`.
    pub name: &'static str,
    /// Raw `plugin.json` text (parsed when building the map).
    pub manifest_json: &'static str,
}

/// Returns the embedded injector artifacts in map order.
pub fn artifacts() -> [InjectorPluginArtifact; 2] {
    [
        InjectorPluginArtifact {
            id: "remcord-arrpc",
            js: REMCORD_ARRPC_PLUGIN_JS,
            sha256: REMCORD_ARRPC_PLUGIN_SHA256,
            bytes: REMCORD_ARRPC_PLUGIN_BYTES,
            name: REMCORD_ARRPC_PLUGIN_NAME,
            manifest_json: REMCORD_ARRPC_PLUGIN_MANIFEST_JSON,
        },
        InjectorPluginArtifact {
            id: "remcord-power",
            js: REMCORD_POWER_PLUGIN_JS,
            sha256: REMCORD_POWER_PLUGIN_SHA256,
            bytes: REMCORD_POWER_PLUGIN_BYTES,
            name: REMCORD_POWER_PLUGIN_NAME,
            manifest_json: REMCORD_POWER_PLUGIN_MANIFEST_JSON,
        },
    ]
}

/// Validates one candidate artifact with the same rules `build.rs`
/// enforces, so the runtime invariant is testable without re-running the
/// build.
pub fn validate_artifact(js: &str, name: &str, expected_name: &str) -> Result<(), String> {
    if js.is_empty() {
        return Err("injector plugin artifact must not be empty".to_owned());
    }
    if js.len() > MAX_INJECTOR_PLUGIN_BYTES {
        return Err("injector plugin artifact exceeds the size limit".to_owned());
    }
    for marker in INJECTOR_PLUGIN_BOOT_MARKERS {
        if !js.contains(marker) {
            return Err(format!(
                "injector plugin artifact is missing the structural member `{marker}`"
            ));
        }
    }
    if name != expected_name {
        return Err(format!(
            "unexpected injector plugin name {name:?}; refusing a foreign artifact"
        ));
    }
    Ok(())
}

/// Validates that `sha256` is a well-formed lowercase hex digest.
pub fn validate_sha256(sha256: &str) -> Result<(), String> {
    if sha256.len() != 64
        || !sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("injector plugin identity must be 64 lowercase hex characters".to_owned());
    }
    Ok(())
}

/// Builds the injector map value (local form): each id maps to
/// `{js, manifest, update: false, injectorIntegration}` with Electron's
/// exact visibility values. Serialization (never raw interpolation) keeps
/// artifact bytes — backticks, `${`, quotes, newlines — inert.
pub fn injector_map_json() -> Result<String, String> {
    let artifacts = artifacts();
    if INJECTOR_PLUGIN_IDS.len() != artifacts.len()
        || INJECTOR_PLUGIN_EXPECTED_NAMES.len() != artifacts.len()
    {
        return Err("injector plugin registry is inconsistent".to_owned());
    }
    let mut map = serde_json::Map::with_capacity(artifacts.len());
    for (artifact, expected_name) in artifacts.iter().zip(INJECTOR_PLUGIN_EXPECTED_NAMES.iter()) {
        validate_artifact(artifact.js, artifact.name, expected_name)?;
        validate_sha256(artifact.sha256)?;
        if artifact.bytes != artifact.js.len() {
            return Err(format!(
                "injector plugin '{}' byte length does not match its text",
                artifact.id
            ));
        }
        let manifest: serde_json::Value =
            serde_json::from_str(artifact.manifest_json).map_err(|error| {
                format!(
                    "injector plugin '{}' manifest is not valid JSON: {error}",
                    artifact.id
                )
            })?;
        if !manifest.is_object() {
            return Err(format!(
                "injector plugin '{}' manifest must be a JSON object",
                artifact.id
            ));
        }
        map.insert(
            artifact.id.to_owned(),
            serde_json::json!({
                "js": artifact.js,
                "manifest": manifest,
                "update": false,
                "injectorIntegration": {
                    "isVisible": false,
                    "allowedActions": {}
                }
            }),
        );
    }
    serde_json::to_string(&map)
        .map_err(|error| format!("injector map cannot be serialized: {error}"))
}

/// Builds the full declaration prepended to the Shelter execution script:
/// `const SHELTER_INJECTOR_PLUGINS=<map>;`.
pub fn injector_declaration() -> Result<String, String> {
    let map = injector_map_json()?;
    Ok(format!("const {INJECTOR_GLOBAL}={map};"))
}

#[cfg(test)]
mod tests {
    use super::{
        artifacts, injector_declaration, injector_map_json, validate_artifact, validate_sha256,
        INJECTOR_GLOBAL, INJECTOR_PLUGIN_EXPECTED_NAMES, INJECTOR_PLUGIN_IDS,
    };

    #[test]
    fn embedded_artifacts_pass_validation() {
        for (artifact, expected_name) in artifacts()
            .iter()
            .zip(INJECTOR_PLUGIN_EXPECTED_NAMES.iter())
        {
            validate_artifact(artifact.js, artifact.name, expected_name)
                .unwrap_or_else(|error| panic!("{}: {error}", artifact.id));
            validate_sha256(artifact.sha256)
                .unwrap_or_else(|error| panic!("{}: {error}", artifact.id));
            assert_eq!(
                artifact.bytes,
                artifact.js.len(),
                "{} byte length must match its text",
                artifact.id
            );
            assert!(
                INJECTOR_PLUGIN_IDS.contains(&artifact.id),
                "{} must be a registered injector id",
                artifact.id
            );
        }
        assert_eq!(artifacts().len(), INJECTOR_PLUGIN_IDS.len());
    }

    #[test]
    fn injector_ids_are_exactly_the_two_servable_builtins() {
        assert_eq!(INJECTOR_PLUGIN_IDS, &["remcord-arrpc", "remcord-power"]);
        // The host-owned ids must never be claimed here (the host starts
        // those itself after the marker); the Electron-only screenshare
        // surface would fail every launch.
        for excluded in [
            "remcord-settings",
            "remcord-titlebar",
            "remcord-screenshare",
            "remcord-touchbar",
        ] {
            assert!(
                !INJECTOR_PLUGIN_IDS.contains(&excluded),
                "{excluded} must stay out of the native injector map"
            );
        }
    }

    #[test]
    fn injector_map_uses_local_form_with_electron_visibility() {
        let map: serde_json::Value =
            serde_json::from_str(&injector_map_json().expect("the injector map must build"))
                .expect("the injector map must be valid JSON");
        let object = map.as_object().expect("the injector map must be an object");
        assert_eq!(object.len(), 2);
        for id in INJECTOR_PLUGIN_IDS {
            let entry = object
                .get(*id)
                .unwrap_or_else(|| panic!("{id} must be mapped"));
            // Local form: inline source plus manifest, pinned (no update
            // checks, no source URL to fetch). A remote `[url, config]`
            // array here would reintroduce the fetch path.
            assert!(
                entry.is_object(),
                "{id} must use local object form, not a remote array"
            );
            let entry = entry.as_object().unwrap();
            assert!(
                entry["js"].as_str().is_some_and(|js| !js.is_empty()),
                "{id} js"
            );
            assert!(entry["manifest"].is_object(), "{id} manifest");
            assert_eq!(entry["update"], false);
            assert_eq!(entry["injectorIntegration"]["isVisible"], false);
            assert!(entry["injectorIntegration"]["allowedActions"].is_object());
            assert!(entry.get("src").is_none(), "{id} must carry no source URL");
        }
        // No remcord URL, path, or foreign id anywhere in the map: entries
        // are pinned locals, never fetched. (Plugin sources may compare
        // against "https://" strings in code — that is data, not a fetch
        // the host performs — so only the scheme/address forms that could
        // select a load are refused here.)
        let raw = injector_map_json().expect("the injector map must build");
        for forbidden in [
            "remcord://",
            "remcord-settings",
            "remcord-titlebar",
            "remcord-screenshare",
        ] {
            assert!(
                !raw.contains(forbidden),
                "the injector map must not contain {forbidden:?}"
            );
        }
    }

    #[test]
    fn declaration_names_the_exact_shelter_global() {
        let declaration = injector_declaration().expect("the declaration must build");
        assert_eq!(INJECTOR_GLOBAL, "SHELTER_INJECTOR_PLUGINS");
        assert!(
            declaration.starts_with("const SHELTER_INJECTOR_PLUGINS="),
            "the declaration must open with the exact global"
        );
        assert!(declaration.ends_with(';'));
        // The map round-trips: strip the declaration wrapper and parse.
        let map_json = declaration
            .strip_prefix("const SHELTER_INJECTOR_PLUGINS=")
            .and_then(|rest| rest.strip_suffix(';'))
            .expect("the declaration must wrap the map exactly once");
        let map: serde_json::Value =
            serde_json::from_str(map_json).expect("the wrapped map must parse");
        assert_eq!(
            map.as_object().expect("map").len(),
            2,
            "exactly the two servable builtins"
        );
    }

    #[test]
    fn bad_artifacts_are_rejected() {
        assert!(validate_artifact("", "Remcord RPC", "Remcord RPC").is_err());
        assert!(validate_artifact("x", "Foreign", "Remcord RPC").is_err());
        let no_marker = "var x = 1;";
        assert!(validate_artifact(no_marker, "Remcord RPC", "Remcord RPC").is_err());
        assert!(validate_sha256("").is_err());
        assert!(validate_sha256(&"0".repeat(64)).is_ok());
        assert!(validate_sha256(&"A".repeat(64)).is_err());
    }
}
