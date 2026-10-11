//! Path resolver for the remcord:// custom protocol.
//!
//! Implements strict path resolution with security hardening beyond Electron's
//! weak `filePath.includes("..")` validation. All paths are canonicalized and
//! verified to remain within their authorized namespace root.

use std::path::{Path, PathBuf};
use thiserror::Error;

/// Fixed namespace identifiers. These are the only valid namespaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Namespace {
    Plugins,
    Html,
    Js,
    Assets,
    Css,
    Local,
}

impl Namespace {
    /// Parses a namespace from its string representation.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "plugins" => Some(Namespace::Plugins),
            "html" => Some(Namespace::Html),
            "js" => Some(Namespace::Js),
            "assets" => Some(Namespace::Assets),
            "css" => Some(Namespace::Css),
            "local" => Some(Namespace::Local),
            _ => None,
        }
    }

    /// Returns the namespace as a string.
    #[allow(dead_code)]
    pub fn as_str(self) -> &'static str {
        match self {
            Namespace::Plugins => "plugins",
            Namespace::Html => "html",
            Namespace::Js => "js",
            Namespace::Assets => "assets",
            Namespace::Css => "css",
            Namespace::Local => "local",
        }
    }
}

/// Base directory roots for each namespace.
/// These are host-owned and fixed at startup.
#[derive(Debug, Clone)]
pub struct NamespaceRoots {
    pub plugins: PathBuf,
    pub html: PathBuf,
    pub js: PathBuf,
    pub assets: PathBuf,
    pub css: PathBuf,
    pub local: PathBuf,
}

impl NamespaceRoots {
    /// Returns the canonical root path for a given namespace.
    pub fn root_for(&self, namespace: Namespace) -> &Path {
        match namespace {
            Namespace::Plugins => &self.plugins,
            Namespace::Html => &self.html,
            Namespace::Js => &self.js,
            Namespace::Assets => &self.assets,
            Namespace::Css => &self.css,
            Namespace::Local => &self.local,
        }
    }
}

/// Errors that can occur during path resolution.
#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum ResolveError {
    #[error("invalid namespace: {0}")]
    InvalidNamespace(String),

    #[error("empty path")]
    EmptyPath,

    #[error("malformed percent encoding: {0}")]
    MalformedPercentEncoding(String),

    #[error("absolute Windows path not allowed: {0}")]
    AbsoluteWindowsPath(String),

    #[error("drive-relative path not allowed: {0}")]
    DriveRelativePath(String),

    #[error("UNC path not allowed: {0}")]
    UncPath(String),

    #[error("path traversal attempt: {0}")]
    TraversalAttempt(String),

    #[error("namespace crossing attempt")]
    NamespaceCrossing,

    #[error("symlink or junction escape outside authorized root")]
    SymlinkEscape,

    #[error("target is a directory, not a file")]
    IsDirectory,

    #[error("file not found")]
    NotFound,

    #[error("target outside authorized root")]
    OutsideRoot,
}

/// Result type for resolution operations.
pub type ResolveResult<T> = Result<T, ResolveError>;

/// The legacy Electron scheme. Never registered with WebView2 and never
/// emitted by Remcord content; accepted only as a rewrite input so existing
/// user customizations (themes, plugins, Quick CSS) and the embedded upstream
/// artifact keep resolving. The rewrite happens before every security check,
/// so legacy URLs face the identical policy as canonical ones.
pub const LEGACY_SCHEME_PREFIX: &str = "legcord://";
/// The one canonical runtime scheme.
pub const CANONICAL_SCHEME_PREFIX: &str = "remcord://";

/// True when `uri` uses the legacy Electron scheme and will be rewritten into
/// the canonical resolver. Host-side logging only; no renderer surface.
pub fn is_legacy_uri(uri: &str) -> bool {
    uri.starts_with(LEGACY_SCHEME_PREFIX)
}

/// Resolves a remcord:// URI to a filesystem path.
///
/// Legacy `legcord://` URLs are rewritten to `remcord://` before any other
/// check and face the identical policy below — one canonical resolver, no
/// second security surface.
///
/// Performs the following security checks:
/// 1. Validates the namespace is one of the fixed set
/// 2. Parses the URI properly (scheme, authority, path, query, fragment)
/// 3. Validates authority/host form matches `remcord://namespace/path` syntax
/// 4. Decodes path exactly once per repository URL semantics
/// 5. Rejects malformed percent encoding
/// 6. Rejects absolute Windows paths (C:\, C:/, \\server\share, \\?\, etc.)
/// 7. Rejects drive-relative paths (C:foo)
/// 8. Handles both `/` and `\` as Windows path separators
/// 9. Normalizes path components (., .., repeated separators)
/// 10. Canonicalizes the fixed base directory
/// 11. Canonicalizes the target when it exists
/// 12. Verifies canonical target is contained within canonical base
/// 13. Rejects symlink/junction escape outside authorized root
/// 14. Rejects directories
/// 15. Rejects empty path
/// 16. Prevents namespace crossing
/// 17. Does not accept renderer-provided absolute roots
/// 18. Does not expose filesystem paths in errors
pub fn resolve_uri(uri: &str, roots: &NamespaceRoots) -> ResolveResult<PathBuf> {
    // Narrow legacy compatibility: rewrite the old Electron scheme into the
    // canonical one before any security decision. Everything below — raw-text
    // traversal checks, decoding policy, containment — runs on the rewritten
    // URI exactly as if the caller had used the canonical scheme.
    let rewritten;
    let uri = if let Some(rest) = uri.strip_prefix(LEGACY_SCHEME_PREFIX) {
        rewritten = format!("{CANONICAL_SCHEME_PREFIX}{rest}");
        rewritten.as_str()
    } else {
        uri
    };
    // Security-critical: check the RAW uri string BEFORE url::Url normalizes it.
    // url::Url resolves dot segments and decodes %2e, so traversal would be
    // invisible after parsing. All traversal/encoding checks run on raw text.
    if !uri.starts_with("remcord://") {
        return Err(ResolveError::InvalidNamespace(
            "scheme must be 'remcord'".to_string(),
        ));
    }
    let rest = &uri["remcord://".len()..];

    // Manual host/path split so dot segments survive until we inspect them.
    let (host, raw_path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, ""),
    };
    if host.is_empty() {
        return Err(ResolveError::InvalidNamespace(
            "missing namespace (host)".to_string(),
        ));
    }
    let namespace = Namespace::from_str(host)
        .ok_or_else(|| ResolveError::InvalidNamespace(host.to_string()))?;

    // Reject empty path
    if raw_path.is_empty() || raw_path == "/" {
        return Err(ResolveError::EmptyPath);
    }

    // Leading double slash is UNC-like (\\server\share or //server/share).
    // Interior repeated separators are normalized later; only the leading
    // position is ambiguous with UNC, so it fails closed here.
    // NOTE: extended-length / UNC raw checks run BEFORE the query/fragment
    // gate because the \\?\ prefix itself contains a literal '?'.
    let raw_lower = raw_path.to_ascii_lowercase();
    if raw_lower.contains("\\\\?\\")
        || raw_lower.contains("\\\\?/")
        || raw_lower.contains("%5c%5c%3f")
    {
        return Err(ResolveError::AbsoluteWindowsPath(
            "extended-length path not allowed".to_string(),
        ));
    }
    if raw_lower.contains("%5c%5c") || raw_path.contains("\\\\") {
        return Err(ResolveError::UncPath("UNC path not allowed".to_string()));
    }
    if raw_path.starts_with("//") {
        return Err(ResolveError::UncPath("UNC path not allowed".to_string()));
    }

    // Reject query and fragment delimiters in raw form. Consumers do not use
    // them; Electron's net.fetch would have folded them into the path.
    // Literal '?' or '#' always introduces query/fragment — encoded %3F/%23
    // are fine and decode to filename characters below.
    if rest.contains('?') || rest.contains('#') {
        return Err(ResolveError::TraversalAttempt(
            "query and fragment not supported".to_string(),
        ));
    }

    // Validate percent-encoding well-formedness on the raw path: every '%'
    // must be followed by exactly two hex digits.
    validate_percent_encoding(raw_path)?;

    // Still run url::Url::parse as an overall URI well-formedness gate.
    // Its normalized path is NOT used for security decisions.
    let parsed =
        url::Url::parse(uri).map_err(|e| ResolveError::MalformedPercentEncoding(e.to_string()))?;
    if parsed.scheme() != "remcord" {
        return Err(ResolveError::InvalidNamespace(
            "scheme must be 'remcord'".to_string(),
        ));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(ResolveError::TraversalAttempt(
            "query and fragment not supported".to_string(),
        ));
    }

    // Strip exactly one leading slash for decoding.
    let path_to_decode = raw_path.strip_prefix('/').unwrap_or(raw_path);

    // Decode the path exactly once.
    let decoded_path = percent_encoding::percent_decode_str(path_to_decode)
        .decode_utf8()
        .map_err(|e| ResolveError::MalformedPercentEncoding(e.to_string()))?
        .into_owned();

    // Double-encoding hardening: a single decode must never leave behind a
    // second layer of traversal-relevant encodings (%2e dot, %2f/%5c
    // separators, %3a drive colon). If raw contained %25 and the single
    // decode still yields encoded dots/separators/colons, reject.
    let decoded_lower = decoded_path.to_ascii_lowercase();
    if raw_lower.contains("%25")
        && (decoded_lower.contains("%2e")
            || decoded_lower.contains("%2f")
            || decoded_lower.contains("%5c")
            || decoded_lower.contains("%3a"))
    {
        return Err(ResolveError::TraversalAttempt(
            "double-encoded traversal not allowed".to_string(),
        ));
    }

    // Validate the DECODED path for Windows-specific dangerous forms. This
    // catches C%3A / %2e%2e / %5c forms that are invisible in raw text.
    validate_windows_path_safety(&decoded_path)?;

    // Normalize path separators: both '/' and '\' are Windows separators.
    let normalized_path = decoded_path.replace('\\', "/");

    // A decoded leading slash (e.g. %2fserver/share or \/absolute) is UNC
    // or absolute-path syntax — fail closed before component splitting.
    if normalized_path.starts_with('/') {
        return Err(ResolveError::UncPath("UNC path not allowed".to_string()));
    }

    // Split into components and normalize
    let components: Vec<&str> = normalized_path
        .split('/')
        .filter(|c| !c.is_empty() && *c != ".")
        .collect();

    // Check for directory traversal attempts in components
    for component in &components {
        if *component == ".." {
            return Err(ResolveError::TraversalAttempt(
                "parent directory reference (..) not allowed".to_string(),
            ));
        }
        // A colon inside any component is drive/ADS/prefix-confusion syntax
        // (C:, file:../, alternate data streams). No valid remcord asset
        // name contains one.
        if component.contains(':') {
            return Err(check_colon_component(component));
        }
    }

    // Reject if any component is a Windows absolute path form
    for component in &components {
        validate_windows_path_safety(component)?;
    }

    // Build the relative path from components
    let relative_path: PathBuf = components.iter().collect();

    // Ensure we have a non-empty path after normalization
    if relative_path.as_os_str().is_empty() {
        return Err(ResolveError::EmptyPath);
    }

    // Get the canonical base root for this namespace
    let base_root = roots.root_for(namespace);
    let canonical_base = std::fs::canonicalize(base_root).map_err(|_| ResolveError::OutsideRoot)?;

    // Build the target path
    let target_path = base_root.join(&relative_path);

    // Canonicalize the target (this resolves symlinks/junctions)
    let canonical_target = std::fs::canonicalize(&target_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ResolveError::NotFound
        } else {
            ResolveError::OutsideRoot
        }
    })?;

    // Verify the canonical target is contained within the canonical base
    if !canonical_target.starts_with(&canonical_base) {
        return Err(ResolveError::SymlinkEscape);
    }

    // Ensure it's a file, not a directory
    let metadata = std::fs::metadata(&canonical_target).map_err(|_| ResolveError::NotFound)?;
    if metadata.is_dir() {
        return Err(ResolveError::IsDirectory);
    }

    // Verify it's not a symlink/junction that escapes (additional check)
    // On Windows, canonicalize already resolves symlinks, so the starts_with check above covers this
    // But we also check the original target path wasn't a symlink pointing outside
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Check if the target_path (before canonicalization) was a symlink/junction
        // by comparing file attributes
        if let Ok(meta) = std::fs::symlink_metadata(&target_path) {
            // FILE_ATTRIBUTE_REPARSE_POINT = 0x400
            if meta.file_attributes() & 0x400 != 0 {
                // It's a reparse point (symlink/junction) - verify it didn't escape
                // The canonical_target check above already ensures this, but we double-check
                if !canonical_target.starts_with(&canonical_base) {
                    return Err(ResolveError::SymlinkEscape);
                }
            }
        }
    }

    Ok(canonical_target)
}

/// Every '%' in the raw path must be followed by exactly two hex digits.
fn validate_percent_encoding(path: &str) -> ResolveResult<()> {
    let bytes = path.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len()
                || !bytes[i + 1].is_ascii_hexdigit()
                || !bytes[i + 2].is_ascii_hexdigit()
            {
                return Err(ResolveError::MalformedPercentEncoding(
                    "invalid percent-encoding".to_string(),
                ));
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    Ok(())
}

/// Classifies a colon-bearing component: drive syntax (C:...) vs generic
/// prefix-confusion / ADS syntax (file:...). No valid asset name has one.
fn check_colon_component(component: &str) -> ResolveError {
    let mut chars = component.chars();
    let first = chars.next().unwrap_or('\0');
    let second = chars.next().unwrap_or('\0');
    if first.is_ascii_alphabetic() && second == ':' {
        let third = chars.next().unwrap_or('\0');
        if third == '\\' || third == '/' {
            return ResolveError::AbsoluteWindowsPath(
                "absolute drive path not allowed".to_string(),
            );
        }
        return ResolveError::DriveRelativePath("drive-relative path not allowed".to_string());
    }
    ResolveError::TraversalAttempt("colon in path component not allowed".to_string())
}

/// Validates that a path component doesn't contain Windows-specific dangerous forms.
fn validate_windows_path_safety(path: &str) -> ResolveResult<()> {
    let path = path.trim();

    // Check for absolute Windows drive paths: C:\..., C:/...
    if path.len() >= 3 {
        let first_char = path.chars().next().unwrap_or('\0');
        let second_char = path.chars().nth(1).unwrap_or('\0');
        let third_char = path.chars().nth(2).unwrap_or('\0');

        if first_char.is_ascii_alphabetic() && second_char == ':' {
            if third_char == '\\' || third_char == '/' {
                return Err(ResolveError::AbsoluteWindowsPath(
                    "absolute drive path not allowed".to_string(),
                ));
            }
            // Drive-relative: C:foo
            return Err(ResolveError::DriveRelativePath(
                "drive-relative path not allowed".to_string(),
            ));
        }
    }

    // Check for UNC paths: \\server\share\... or //server/share/...
    if path.starts_with("\\\\") || path.starts_with("//") {
        return Err(ResolveError::UncPath("UNC path not allowed".to_string()));
    }

    // Check for extended-length paths: \\?\...
    if path.starts_with("\\\\?\\") || path.starts_with("\\\\?/") {
        return Err(ResolveError::AbsoluteWindowsPath(
            "extended-length path not allowed".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_test_roots() -> (tempfile::TempDir, NamespaceRoots) {
        let temp_dir = tempfile::tempdir().unwrap();
        let base = temp_dir.path();

        let roots = NamespaceRoots {
            plugins: base.join("plugins"),
            html: base.join("html"),
            js: base.join("js"),
            assets: base.join("assets"),
            css: base.join("css"),
            local: base.join("local"),
        };

        // Create all roots
        fs::create_dir_all(&roots.plugins).unwrap();
        fs::create_dir_all(&roots.html).unwrap();
        fs::create_dir_all(&roots.js).unwrap();
        fs::create_dir_all(&roots.assets).unwrap();
        fs::create_dir_all(&roots.css).unwrap();
        fs::create_dir_all(&roots.local).unwrap();

        // Create some test files
        fs::write(roots.plugins.join("test.js"), "// test plugin").unwrap();
        fs::write(roots.html.join("splash.html"), "<html>test</html>").unwrap();
        fs::write(roots.js.join("main.js"), "console.log('test');").unwrap();
        fs::write(roots.assets.join("image.png"), b"fake png").unwrap();
        fs::write(roots.css.join("style.css"), "body {}").unwrap();
        fs::write(roots.local.join("user.txt"), "user data").unwrap();

        // Create nested structure
        fs::create_dir_all(roots.plugins.join("rpc")).unwrap();
        fs::write(roots.plugins.join("rpc").join("index.js"), "// rpc plugin").unwrap();

        (temp_dir, roots)
    }

    #[test]
    fn valid_namespace_and_path() {
        let (_temp, roots) = setup_test_roots();
        let result = resolve_uri("remcord://plugins/test.js", &roots).unwrap();
        assert!(result.ends_with("plugins/test.js"));
        assert!(result.exists());
    }

    #[test]
    fn nested_path() {
        let (_temp, roots) = setup_test_roots();
        let result = resolve_uri("remcord://plugins/rpc/index.js", &roots).unwrap();
        assert!(result.ends_with("plugins/rpc/index.js"));
        assert!(result.exists());
    }

    #[test]
    fn dot_segment_rejected() {
        let (_temp, roots) = setup_test_roots();
        let result = resolve_uri("remcord://plugins/../html/splash.html", &roots);
        assert!(matches!(result, Err(ResolveError::TraversalAttempt(_))));
    }

    #[test]
    fn backslash_dot_segment_rejected() {
        let (_temp, roots) = setup_test_roots();
        let result = resolve_uri("remcord://plugins/..\\html/splash.html", &roots);
        assert!(matches!(result, Err(ResolveError::TraversalAttempt(_))));
    }

    #[test]
    fn mixed_separators() {
        let (_temp, roots) = setup_test_roots();
        let result = resolve_uri("remcord://plugins/rpc\\index.js", &roots).unwrap();
        assert!(result.ends_with("plugins/rpc/index.js"));
    }

    #[test]
    fn encoded_traversal_rejected() {
        let (_temp, roots) = setup_test_roots();
        let result = resolve_uri("remcord://plugins/%2e%2e/html/splash.html", &roots);
        assert!(matches!(result, Err(ResolveError::TraversalAttempt(_))));
    }

    #[test]
    fn double_encoded_traversal_rejected() {
        let (_temp, roots) = setup_test_roots();
        let result = resolve_uri("remcord://plugins/%252e%252e/html/splash.html", &roots);
        assert!(matches!(result, Err(ResolveError::TraversalAttempt(_))));
    }

    #[test]
    fn absolute_windows_path_rejected() {
        let (_temp, roots) = setup_test_roots();
        let result = resolve_uri("remcord://plugins/C:/Windows/System32/cmd.exe", &roots);
        assert!(matches!(result, Err(ResolveError::AbsoluteWindowsPath(_))));
    }

    #[test]
    fn absolute_windows_path_backslash_rejected() {
        let (_temp, roots) = setup_test_roots();
        let result = resolve_uri("remcord://plugins/C:\\Windows\\System32\\cmd.exe", &roots);
        assert!(matches!(result, Err(ResolveError::AbsoluteWindowsPath(_))));
    }

    #[test]
    fn drive_relative_rejected() {
        let (_temp, roots) = setup_test_roots();
        let result = resolve_uri("remcord://plugins/C:foo", &roots);
        assert!(matches!(result, Err(ResolveError::DriveRelativePath(_))));
    }

    #[test]
    fn unc_path_rejected() {
        let (_temp, roots) = setup_test_roots();
        let result = resolve_uri("remcord://plugins/\\\\server\\share\\file.txt", &roots);
        assert!(matches!(result, Err(ResolveError::UncPath(_))));
    }

    #[test]
    fn unc_path_forward_slash_rejected() {
        let (_temp, roots) = setup_test_roots();
        let result = resolve_uri("remcord://plugins//server/share/file.txt", &roots);
        assert!(matches!(result, Err(ResolveError::UncPath(_))));
    }

    #[test]
    fn extended_length_path_rejected() {
        let (_temp, roots) = setup_test_roots();
        let result = resolve_uri("remcord://plugins/\\\\?\\C:\\Windows\\file.txt", &roots);
        assert!(matches!(result, Err(ResolveError::AbsoluteWindowsPath(_))));
    }

    #[test]
    fn prefix_confusion_rejected() {
        let (_temp, roots) = setup_test_roots();
        // Try to confuse the parser with scheme-like prefixes
        let result = resolve_uri("remcord://plugins/file:../etc/passwd", &roots);
        assert!(matches!(result, Err(ResolveError::TraversalAttempt(_))));
    }

    #[test]
    fn symlink_escape_rejected() {
        let temp_dir = tempfile::tempdir().unwrap();
        let base = temp_dir.path();

        let roots = NamespaceRoots {
            plugins: base.join("plugins"),
            html: base.join("html"),
            js: base.join("js"),
            assets: base.join("assets"),
            css: base.join("css"),
            local: base.join("local"),
        };

        fs::create_dir_all(&roots.plugins).unwrap();
        fs::create_dir_all(&roots.html).unwrap();

        // Create a file outside the root
        let outside = base.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.txt"), "secret").unwrap();

        // Create a symlink inside plugins pointing outside
        #[cfg(windows)]
        let link_result = std::os::windows::fs::symlink_file(
            outside.join("secret.txt"),
            roots.plugins.join("escape.txt"),
        );
        #[cfg(not(windows))]
        let link_result = std::os::unix::fs::symlink(
            outside.join("secret.txt"),
            roots.plugins.join("escape.txt"),
        );
        if link_result.is_err() {
            eprintln!("SKIP: link creation needs privileges in this environment");
            return;
        }

        let result = resolve_uri("remcord://plugins/escape.txt", &roots);
        assert!(matches!(result, Err(ResolveError::SymlinkEscape)));
    }

    #[test]
    fn namespace_crossing_rejected() {
        let (_temp, roots) = setup_test_roots();
        // Try to access html from plugins namespace
        let result = resolve_uri("remcord://plugins/../html/splash.html", &roots);
        assert!(matches!(result, Err(ResolveError::TraversalAttempt(_))));
    }

    #[test]
    fn empty_path_rejected() {
        let (_temp, roots) = setup_test_roots();
        let result = resolve_uri("remcord://plugins/", &roots);
        assert!(matches!(result, Err(ResolveError::EmptyPath)));
    }

    #[test]
    fn directory_target_rejected() {
        let (_temp, roots) = setup_test_roots();
        // Create a directory
        fs::create_dir_all(roots.plugins.join("subdir")).unwrap();
        let result = resolve_uri("remcord://plugins/subdir", &roots);
        assert!(matches!(result, Err(ResolveError::IsDirectory)));
    }

    #[test]
    fn missing_target_returns_not_found() {
        let (_temp, roots) = setup_test_roots();
        let result = resolve_uri("remcord://plugins/nonexistent.js", &roots);
        assert!(matches!(result, Err(ResolveError::NotFound)));
    }

    #[test]
    fn malformed_percent_encoding_rejected() {
        let (_temp, roots) = setup_test_roots();
        let result = resolve_uri("remcord://plugins/%ZZ", &roots);
        assert!(matches!(
            result,
            Err(ResolveError::MalformedPercentEncoding(_))
        ));
    }

    #[test]
    fn repeated_separators_normalized() {
        let (_temp, roots) = setup_test_roots();
        // Interior repeats normalize; a LEADING double slash is UNC-like and
        // rejected (see unc_path_forward_slash_rejected).
        let result = resolve_uri("remcord://plugins/rpc//index.js", &roots).unwrap();
        assert!(result.ends_with("plugins/rpc/index.js"));
        let result = resolve_uri("remcord://plugins/rpc///index.js", &roots).unwrap();
        assert!(result.ends_with("plugins/rpc/index.js"));
    }

    #[test]
    fn unicode_filename() {
        let temp_dir = tempfile::tempdir().unwrap();
        let base = temp_dir.path();

        let roots = NamespaceRoots {
            plugins: base.join("plugins"),
            html: base.join("html"),
            js: base.join("js"),
            assets: base.join("assets"),
            css: base.join("css"),
            local: base.join("local"),
        };

        fs::create_dir_all(&roots.assets).unwrap();
        let unicode_name = "测试图片.png";
        fs::write(roots.assets.join(unicode_name), b"fake").unwrap();

        let encoded = percent_encoding::percent_encode(
            unicode_name.as_bytes(),
            percent_encoding::NON_ALPHANUMERIC,
        )
        .to_string();
        let uri = format!("remcord://assets/{}", encoded);
        let result = resolve_uri(&uri, &roots).unwrap();
        assert!(result.ends_with(unicode_name));
    }

    #[test]
    fn query_and_fragment_rejected() {
        let (_temp, roots) = setup_test_roots();
        let result = resolve_uri("remcord://plugins/test.js?query=1", &roots);
        assert!(matches!(result, Err(ResolveError::TraversalAttempt(_))));

        let result = resolve_uri("remcord://plugins/test.js#fragment", &roots);
        assert!(matches!(result, Err(ResolveError::TraversalAttempt(_))));
    }

    #[test]
    fn invalid_namespace_rejected() {
        let (_temp, roots) = setup_test_roots();
        let result = resolve_uri("remcord://invalid/test.js", &roots);
        assert!(matches!(result, Err(ResolveError::InvalidNamespace(_))));
    }

    #[test]
    fn all_valid_namespaces_resolve() {
        let (_temp, roots) = setup_test_roots();
        assert!(resolve_uri("remcord://plugins/test.js", &roots).is_ok());
        assert!(resolve_uri("remcord://html/splash.html", &roots).is_ok());
        assert!(resolve_uri("remcord://js/main.js", &roots).is_ok());
        assert!(resolve_uri("remcord://assets/image.png", &roots).is_ok());
        assert!(resolve_uri("remcord://css/style.css", &roots).is_ok());
        assert!(resolve_uri("remcord://local/user.txt", &roots).is_ok());
    }

    #[test]
    fn legacy_scheme_rewrites_to_the_same_file() {
        // Existing user customizations and the embedded upstream artifact
        // reference the old Electron scheme. The rewrite must land on the
        // identical file the canonical URL resolves to.
        let (_temp, roots) = setup_test_roots();
        assert!(is_legacy_uri("legcord://plugins/test.js"));
        assert!(!is_legacy_uri("remcord://plugins/test.js"));
        let canonical = resolve_uri("remcord://plugins/test.js", &roots).unwrap();
        let legacy = resolve_uri("legcord://plugins/test.js", &roots).unwrap();
        assert_eq!(canonical, legacy);
        assert!(resolve_uri("legcord://html/splash.html", &roots).is_ok());
        assert!(resolve_uri("legcord://assets/image.png", &roots).is_ok());
    }

    #[test]
    fn legacy_scheme_faces_the_identical_security_policy() {
        // The rewrite happens before every check: traversal, UNC, drive
        // paths, queries, and unknown namespaces fail for legacy URLs exactly
        // as they do for canonical ones. No second surface is exposed.
        let (_temp, roots) = setup_test_roots();
        let result = resolve_uri("legcord://plugins/../html/splash.html", &roots);
        assert!(matches!(result, Err(ResolveError::TraversalAttempt(_))));
        let result = resolve_uri("legcord://plugins/C:/Windows/System32/cmd.exe", &roots);
        assert!(matches!(result, Err(ResolveError::AbsoluteWindowsPath(_))));
        let result = resolve_uri("legcord://plugins//server/share/file.txt", &roots);
        assert!(matches!(result, Err(ResolveError::UncPath(_))));
        let result = resolve_uri("legcord://plugins/test.js?query=1", &roots);
        assert!(matches!(result, Err(ResolveError::TraversalAttempt(_))));
        let result = resolve_uri("legcord://invalid/test.js", &roots);
        assert!(matches!(result, Err(ResolveError::InvalidNamespace(_))));
        let result = resolve_uri("legcord://plugins/nonexistent.js", &roots);
        assert!(matches!(result, Err(ResolveError::NotFound)));
    }
}
