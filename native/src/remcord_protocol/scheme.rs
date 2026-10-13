//! WebView2 environment creation with the `remcord` custom scheme registered.
//!
//! Registration (exact, via the installed `webview2-com 0.39` API):
//! - scheme `remcord`, the one canonical custom scheme
//! - `HasAuthorityComponent=true`, matching the `remcord://namespace/path`
//!   syntax the resolver enforces (the namespace is the URI authority)
//! - `TreatAsSecure=true`, the Electron `secure=true` equivalent
//! - `AllowedOrigins=["https://discord.com"]` only: the one origin proven
//!   to load remcord:// subresources. No additional origin is added without
//!   a traced consumer behind it.
//!
//! `supportFetchAPI`/`corsEnabled`/`stream` have no WebView2 registration
//! switch: fetch/XHR reach the same `WebResourceRequested` handler, and no
//! `Access-Control-Allow-Origin` is sent unless a consumer proves the need.
//! `bypassCSP` has NO WebView2 equivalent for custom schemes (registered
//! schemes adhere to CSP), so the Discord document CSP gains `remcord:` in
//! exactly the traced directives (`csp.rs`) instead of being disabled.
//!
//! Every other environment option mirrors wry 0.57's own
//! `create_environment` (additional browser args, system language, default
//! scroll style, caller-supplied user-data directory), so the only
//! behavioral delta from a wry-created environment is the scheme
//! registration. Registration happens before the environment exists and long
//! before navigation — post-creation registration is a no-op, which is why
//! this lives at environment creation rather than beside the request
//! handler. wry adopts the pre-created environment verbatim via
//! `WebViewBuilderExtWindows::with_environment`.

use std::error::Error;

use webview2_com::{
    CoreWebView2CustomSchemeRegistration, CoreWebView2EnvironmentOptions,
    CreateCoreWebView2EnvironmentCompletedHandler,
    Microsoft::Web::WebView2::Win32::{
        CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2Environment,
        ICoreWebView2EnvironmentOptions,
    },
};
use windows::{
    core::{HSTRING, PCWSTR},
    Win32::{
        Foundation::{E_POINTER, E_UNEXPECTED},
        Globalization::{
            GetUserDefaultUILanguage, LCIDToLocaleName, LOCALE_ALLOW_NEUTRAL_NAMES, MAX_LOCALE_NAME,
        },
        System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED},
    },
};

/// The one canonical runtime scheme name.
pub const REMCORD_SCHEME_NAME: &str = "remcord";

/// The only origin allowed to load `remcord://` resources. Extended only
/// with a traced consumer behind the addition.
pub const REMCORD_ALLOWED_ORIGINS: &[&str] = &["https://discord.com"];

/// The exact additional browser arguments the host passes at environment
/// creation (wry 0.57 parity: disables three Microsoft WebView2 UI features,
/// nothing GPU-related). Shared with the graphics diagnostics report so the
/// copied report states exactly what was requested — never a guess.
pub const ADDITIONAL_BROWSER_ARGUMENTS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";

/// Builds the `remcord` scheme registration with the fixed options above.
pub fn build_scheme_registration() -> CoreWebView2CustomSchemeRegistration {
    let registration = CoreWebView2CustomSchemeRegistration::new(REMCORD_SCHEME_NAME.to_owned());
    unsafe {
        registration.set_treat_as_secure(true);
        registration.set_has_authority_component(true);
        registration.set_allowed_origins(
            REMCORD_ALLOWED_ORIGINS
                .iter()
                .map(|origin| origin.to_string())
                .collect(),
        );
    }
    registration
}

/// Builds environment options mirroring wry 0.57's defaults, plus the
/// `remcord` scheme registration (the only delta).
pub fn build_environment_options() -> CoreWebView2EnvironmentOptions {
    let options = CoreWebView2EnvironmentOptions::default();
    unsafe {
        // wry parity: the exact default additional-args string wry uses.
        options.set_additional_browser_arguments(ADDITIONAL_BROWSER_ARGUMENTS.to_owned());
        // wry parity: the user's system language.
        let lcid = GetUserDefaultUILanguage();
        let mut lang = [0; MAX_LOCALE_NAME as usize];
        LCIDToLocaleName(lcid as u32, Some(&mut lang), LOCALE_ALLOW_NEUTRAL_NAMES);
        options.set_language(String::from_utf16_lossy(&lang));
        let registration = build_scheme_registration();
        options.set_scheme_registrations(vec![Some(registration.into())]);
    }
    options
}

/// Creates the WebView2 environment with the `remcord` scheme registered.
/// The caller hands the result to wry via `with_environment`.
pub fn create_environment(
    profile_dir: &std::path::Path,
) -> Result<ICoreWebView2Environment, Box<dyn Error>> {
    // wry initializes COM again at WebView creation; a second init on the
    // same thread is a harmless S_FALSE.
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
    }
    let options = build_environment_options();
    let (tx, rx) = std::sync::mpsc::channel();
    unsafe {
        CreateCoreWebView2EnvironmentWithOptions(
            PCWSTR::null(),
            &HSTRING::from(profile_dir.to_string_lossy().as_ref()),
            &ICoreWebView2EnvironmentOptions::from(options),
            &CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
                move |error_code, environment| {
                    let result: Result<ICoreWebView2Environment, webview2_com::Error> = (|| {
                        error_code?;
                        environment.ok_or_else(|| windows::core::Error::from(E_POINTER).into())
                    })(
                    );
                    tx.send(result)
                        .map_err(|_| windows::core::Error::from(E_UNEXPECTED))
                },
            )),
        )?;
    }
    Ok(webview2_com::wait_with_pump(rx)??)
}

#[cfg(test)]
mod tests {
    use super::*;
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2CustomSchemeRegistration;

    #[test]
    fn scheme_name_and_allowed_origin_are_fixed() {
        assert_eq!(REMCORD_SCHEME_NAME, "remcord");
        assert_eq!(REMCORD_ALLOWED_ORIGINS, &["https://discord.com"]);
    }

    #[test]
    fn registration_carries_the_exact_fixed_options_through_com() {
        // Exercises the real COM interface of the registration object
        // (SchemeName/TreatAsSecure/HasAuthorityComponent/GetAllowedOrigins),
        // the same methods the WebView2 loader calls at environment
        // creation — a marshaling regression fails here, not at runtime.
        let registration = build_scheme_registration();
        let iface: ICoreWebView2CustomSchemeRegistration = registration.into();
        unsafe {
            let mut name = windows::core::PWSTR::null();
            iface.SchemeName(&mut name).unwrap();
            assert_eq!(name.to_string().unwrap(), "remcord");
            let mut secure = windows::core::BOOL::default();
            iface.TreatAsSecure(&mut secure).unwrap();
            assert!(secure.as_bool());
            let mut authority = windows::core::BOOL::default();
            iface.HasAuthorityComponent(&mut authority).unwrap();
            assert!(authority.as_bool());
            let mut count = 0u32;
            let mut origins: *mut windows::core::PWSTR = std::ptr::null_mut();
            iface.GetAllowedOrigins(&mut count, &mut origins).unwrap();
            assert_eq!(count, 1);
            assert_eq!((*origins).to_string().unwrap(), "https://discord.com");
        }
    }

    #[test]
    fn options_carry_exactly_one_scheme_registration() {
        let options = build_environment_options();
        let registrations = unsafe { options.scheme_registrations() };
        assert_eq!(registrations.len(), 1);
        let iface = registrations[0]
            .as_ref()
            .expect("registration must be present");
        unsafe {
            let mut name = windows::core::PWSTR::null();
            iface.SchemeName(&mut name).unwrap();
            assert_eq!(name.to_string().unwrap(), "remcord");
            let mut secure = windows::core::BOOL::default();
            iface.TreatAsSecure(&mut secure).unwrap();
            assert!(secure.as_bool());
        }
    }
}
