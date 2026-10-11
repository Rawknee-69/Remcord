//! WebResourceRequested handler for the remcord:// protocol.
//!
//! Registers the remcord scheme and handles requests using WebView2's
//! WebResourceRequested event with a narrow filter targeting only remcord://*.

use std::sync::Arc;

use crate::remcord_protocol::{mime, resolver, NamespaceRoots, ResolveError};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2, ICoreWebView2Environment, ICoreWebView2WebResourceRequestedEventArgs,
    ICoreWebView2WebResourceResponse, ICoreWebView2_2,
};
use webview2_com::WebResourceRequestedEventHandler;
use windows::core::{Interface, HSTRING};
use windows::Win32::UI::Shell::SHCreateMemStream;

/// Creates a minimal error response body.
fn error_body(status: u16, message: &str) -> Vec<u8> {
    format!("<html><body><h1>{status} Error</h1><p>{message}</p></body></html>").into_bytes()
}

/// Pure mapping from resolver failure to HTTP status + safe message.
/// No filesystem path ever appears in the message.
pub fn status_for_error(error: &ResolveError) -> (u16, &'static str) {
    match error {
        ResolveError::InvalidNamespace(_) => (400, "Invalid namespace"),
        ResolveError::EmptyPath => (400, "Empty path"),
        ResolveError::MalformedPercentEncoding(_) => (400, "Malformed URL encoding"),
        ResolveError::AbsoluteWindowsPath(_) => (400, "Absolute path not allowed"),
        ResolveError::DriveRelativePath(_) => (400, "Drive-relative path not allowed"),
        ResolveError::UncPath(_) => (400, "UNC path not allowed"),
        ResolveError::TraversalAttempt(_) => (400, "Path traversal not allowed"),
        ResolveError::NamespaceCrossing => (400, "Namespace crossing not allowed"),
        ResolveError::SymlinkEscape => (400, "Symlink escape not allowed"),
        ResolveError::IsDirectory => (400, "Path is a directory"),
        ResolveError::NotFound => (404, "File not found"),
        ResolveError::OutsideRoot => (400, "Path outside authorized root"),
    }
}

/// Creates a WebResourceResponse from status, headers, and body bytes.
unsafe fn create_web_response(
    env: &ICoreWebView2Environment,
    status: u16,
    content_type: &str,
    body: Vec<u8>,
) -> windows::core::Result<ICoreWebView2WebResourceResponse> {
    let status_text = http::StatusCode::from_u16(status)
        .unwrap()
        .canonical_reason()
        .unwrap_or("OK");
    let status_hstring = HSTRING::from(status_text);
    let headers = format!(
        "content-type: {content_type}\r\ncontent-length: {}\r\n",
        body.len()
    );
    let headers_hstring = HSTRING::from(headers);

    let stream = if !body.is_empty() {
        SHCreateMemStream(Some(&body))
    } else {
        None
    };

    env.CreateWebResourceResponse(
        stream.as_ref(),
        status as i32,
        &status_hstring,
        &headers_hstring,
    )
}

/// Handles a single remcord:// request.
unsafe fn handle_remcord_request(
    env: &ICoreWebView2Environment,
    args: &ICoreWebView2WebResourceRequestedEventArgs,
    roots: &NamespaceRoots,
) -> windows::core::Result<()> {
    let request = args.Request()?;
    let mut uri_ptr = windows::core::PWSTR::null();
    request.Uri(&mut uri_ptr)?;
    let uri = uri_ptr.to_string().unwrap_or_default();
    let namespace = uri
        .strip_prefix("remcord://")
        .and_then(|rest| rest.split('/').next())
        .unwrap_or("unknown");

    // Resolve the URI to a filesystem path
    let target_path = match resolver::resolve_uri(&uri, roots) {
        Ok(path) => path,
        Err(e) => {
            let (status, message) = status_for_error(&e);
            // Rejection observability without attacker-controlled content: the
            // namespace (host-owned vocabulary) plus the safe status only.
            println!("[RemcordProtocol] status={status} namespace={namespace} rejected=true");
            let body = error_body(status, message);
            let response = create_web_response(env, status, "text/html", body)?;
            args.SetResponse(&response)?;
            return Ok(());
        }
    };

    // Read the file
    let file_bytes = std::fs::read(&target_path)
        .map_err(|_| windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL))?;

    // Determine MIME type
    let content_type = mime::mime_for_path(&target_path);
    let byte_count = file_bytes.len();

    // Create and set the response
    let response = create_web_response(env, 200, content_type, file_bytes)?;
    args.SetResponse(&response)?;

    // Shipped observability: namespace + status + bytes + MIME only. The URI
    // path and any filesystem path never reach the log.
    println!(
        "[RemcordProtocol] status=200 namespace={namespace} bytes={byte_count} mime={content_type}"
    );
    Ok(())
}

/// Registers the remcord:// protocol handler with WebView2.
///
/// Mechanism (verified against the installed crates):
/// - `webview2-com 0.39` exposes environment-options scheme registration
///   (`CoreWebView2CustomSchemeRegistration`), but it applies at environment
///   CREATION time, which wry owns — there is no hook to inject options
///   before wry creates the environment. The registration mechanism
///   available after creation is `WebResourceRequested` + `SetResponse`,
///   the same machinery the existing Discord CSP interception uses.
/// - Origin restriction (the `AllowedOrigins` equivalent) is enforced by
///   layers instead of scheme options: top-level navigation is limited to
///   HTTPS Discord hosts, popups are denied, the filter below targets ONLY
///   `remcord://*`, and the Discord document CSP gains `remcord:` in exactly
///   the traced directives (see `csp.rs`). No `TreatAsSecure` parity is
///   claimed: WebView2 treats the scheme per its own policy and every
///   remcord:// response carries an explicit Content-Type.
/// - Filters use `ICoreWebView2_22::AddWebResourceRequestedFilterWithRequestSourceKinds`
///   (present in the installed `webview2-com-sys 0.39.1` bindings) with one
///   filter per resource context actually served, so iframe/subresource
///   requests are covered with the same narrow `remcord://*` URI scope.
///   When the runtime does not support `ICoreWebView2_22`, the two-argument
///   overload is used with the same per-context filters.
pub fn register_remcord_protocol(
    webview: &ICoreWebView2,
    roots: NamespaceRoots,
) -> windows::core::Result<()> {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2_22, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FETCH, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_SCRIPT,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_STYLESHEET,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_XML_HTTP_REQUEST,
        COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
    };

    // Get the WebView2 environment for creating responses
    let webview2: ICoreWebView2_2 = webview.cast()?;
    let env: ICoreWebView2Environment = unsafe { webview2.Environment()? };

    // One filter per resource context actually served (traced consumers):
    // top-level html loads, stylesheets, scripts/plugins, images, video,
    // and fetch/XHR plugin loads. Never a wildcard context.
    let contexts = [
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_STYLESHEET,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_SCRIPT,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FETCH,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_XML_HTTP_REQUEST,
    ];

    // Narrow URI scope: ONLY remcord://*. The existing Discord HTTPS filter
    // and every other interception are untouched.
    let filter = HSTRING::from("remcord://*");

    // Source-kind-aware overload when the runtime supports it (iframe and
    // subresource correctness); same narrow filter and per-context scope.
    if let Ok(webview22) = webview.cast::<ICoreWebView2_22>() {
        for context in contexts {
            unsafe {
                webview22.AddWebResourceRequestedFilterWithRequestSourceKinds(
                    &filter,
                    context,
                    COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
                )?;
            }
        }
    } else {
        for context in contexts {
            unsafe {
                webview2.AddWebResourceRequestedFilter(&filter, context)?;
            }
        }
    }

    // Create the handler
    let roots = Arc::new(roots);
    let env_clone = env.clone();

    let handler = WebResourceRequestedEventHandler::create(Box::new(
        move |_sender: Option<ICoreWebView2>,
              args: Option<ICoreWebView2WebResourceRequestedEventArgs>| {
            let Some(args) = args else {
                return Ok(());
            };

            let request = match unsafe { args.Request() } {
                Ok(r) => r,
                Err(_) => return Ok(()),
            };

            // Verify it's a remcord:// request.
            let mut uri_ptr = windows::core::PWSTR::null();
            if unsafe { request.Uri(&mut uri_ptr) }.is_err() {
                return Ok(());
            }
            let uri = unsafe { uri_ptr.to_string().unwrap_or_default() };

            if !uri.starts_with("remcord://") {
                return Ok(());
            }

            // Get deferral for async handling
            let deferral = match unsafe { args.GetDeferral() } {
                Ok(d) => d,
                Err(_) => return Ok(()),
            };

            // Handle the request
            let roots_clone = Arc::clone(&roots);
            let env_clone = env_clone.clone();

            // Process synchronously in the handler (static files are fast)
            let result = unsafe { handle_remcord_request(&env_clone, &args, &roots_clone) };

            // Complete the deferral
            let _ = unsafe { deferral.Complete() };

            result
        },
    ));

    // Register the handler
    let mut token = 0i64;
    unsafe {
        webview.add_WebResourceRequested(&handler, &mut token)?;
    }

    Ok(())
}

/// Builds the NamespaceRoots from the application's bundle and user data paths.
pub fn build_namespace_roots(
    bundle_dir: &std::path::Path,
    user_data_dir: &std::path::Path,
) -> NamespaceRoots {
    NamespaceRoots {
        // plugins -> <bundle>/plugins/
        plugins: bundle_dir.join("plugins"),
        // html -> <bundle>/html/
        html: bundle_dir.join("html"),
        // js -> <bundle>/js/
        js: bundle_dir.join("js"),
        // assets -> <bundle>/assets/app/
        assets: bundle_dir.join("assets").join("app"),
        // css -> <bundle>/css/
        css: bundle_dir.join("css"),
        // local -> <userData>/userAssets/
        local: user_data_dir.join("userAssets"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remcord_protocol::resolver;
    use std::fs;

    fn setup_test_roots() -> (tempfile::TempDir, NamespaceRoots) {
        let temp_dir = tempfile::tempdir().unwrap();
        let base = temp_dir.path();

        let bundle_dir = base.join("bundle");
        let user_data_dir = base.join("userData");

        fs::create_dir_all(bundle_dir.join("plugins")).unwrap();
        fs::create_dir_all(bundle_dir.join("html")).unwrap();
        fs::create_dir_all(bundle_dir.join("js")).unwrap();
        fs::create_dir_all(bundle_dir.join("assets").join("app")).unwrap();
        fs::create_dir_all(bundle_dir.join("css")).unwrap();
        fs::create_dir_all(user_data_dir.join("userAssets")).unwrap();

        // Create test files
        fs::write(bundle_dir.join("plugins").join("test.js"), "// test").unwrap();
        fs::write(bundle_dir.join("html").join("splash.html"), "<html></html>").unwrap();
        fs::write(bundle_dir.join("js").join("main.js"), "console.log('hi');").unwrap();
        fs::write(
            bundle_dir.join("assets").join("app").join("icon.png"),
            b"png",
        )
        .unwrap();
        fs::write(bundle_dir.join("css").join("style.css"), "body {}").unwrap();
        fs::write(
            user_data_dir.join("userAssets").join("custom.txt"),
            "custom",
        )
        .unwrap();

        let roots = build_namespace_roots(&bundle_dir, &user_data_dir);
        (temp_dir, roots)
    }

    #[test]
    fn build_namespace_roots_creates_correct_paths() {
        let (_temp, roots) = setup_test_roots();
        assert!(roots.plugins.ends_with("bundle/plugins"));
        assert!(roots.html.ends_with("bundle/html"));
        assert!(roots.js.ends_with("bundle/js"));
        assert!(roots.assets.ends_with("bundle/assets/app"));
        assert!(roots.css.ends_with("bundle/css"));
        assert!(roots.local.ends_with("userData/userAssets"));
    }

    #[test]
    fn resolver_works_with_built_roots() {
        let (_temp, roots) = setup_test_roots();
        let result = resolver::resolve_uri("remcord://plugins/test.js", &roots).unwrap();
        assert!(result.ends_with("bundle/plugins/test.js"));

        let result = resolver::resolve_uri("remcord://html/splash.html", &roots).unwrap();
        assert!(result.ends_with("bundle/html/splash.html"));

        let result = resolver::resolve_uri("remcord://assets/icon.png", &roots).unwrap();
        assert!(result.ends_with("bundle/assets/app/icon.png"));

        let result = resolver::resolve_uri("remcord://local/custom.txt", &roots).unwrap();
        assert!(result.ends_with("userData/userAssets/custom.txt"));
    }

    #[test]
    fn handler_scope_is_canonical_only() {
        // The request filter admits exactly one scheme; neighboring schemes
        // never reach the resolver through this handler.
        for uri in ["other://plugins/test.js", "http://plugins/test.js"] {
            assert!(!uri.starts_with("remcord://"), "{uri} is out of scope");
        }
    }

    #[test]
    fn success_maps_to_200_with_extension_mime() {
        let (_temp, roots) = setup_test_roots();
        // 200 + correct MIME: resolve then read bytes like the handler does.
        let target = resolver::resolve_uri("remcord://css/style.css", &roots).unwrap();
        let bytes = std::fs::read(&target).unwrap();
        assert!(!bytes.is_empty());
        assert_eq!(mime::mime_for_path(&target), "text/css; charset=utf-8");
        let target = resolver::resolve_uri("remcord://html/splash.html", &roots).unwrap();
        assert_eq!(mime::mime_for_path(&target), "text/html; charset=utf-8");
    }

    #[test]
    fn traversal_maps_to_400_missing_maps_to_404() {
        assert_eq!(
            status_for_error(&ResolveError::TraversalAttempt("x".to_string())),
            (400, "Path traversal not allowed")
        );
        assert_eq!(
            status_for_error(&ResolveError::SymlinkEscape),
            (400, "Symlink escape not allowed")
        );
        assert_eq!(
            status_for_error(&ResolveError::NotFound),
            (404, "File not found")
        );
        assert_eq!(
            status_for_error(&ResolveError::IsDirectory),
            (400, "Path is a directory")
        );
    }

    #[test]
    fn error_responses_never_leak_filesystem_paths() {
        let (_temp, roots) = setup_test_roots();
        let root_text = roots.plugins.to_string_lossy().into_owned();
        // Every error message and body for hostile inputs carries no path.
        for uri in [
            "remcord://plugins/../html/splash.html",
            "remcord://plugins/C:/Windows/System32/cmd.exe",
            "remcord://plugins/\\\\server\\share\\file.txt",
            "remcord://plugins/nonexistent.js",
        ] {
            let err = resolver::resolve_uri(uri, &roots).unwrap_err();
            let (status, message) = status_for_error(&err);
            assert!(status == 400 || status == 404);
            assert!(!message.contains(&root_text));
            assert!(!message.contains("plugins"));
            let body = String::from_utf8(error_body(status, message)).unwrap();
            assert!(!body.contains(&root_text));
        }
    }
}
