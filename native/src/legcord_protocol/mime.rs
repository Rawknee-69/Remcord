//! MIME type mapping for the legcord protocol.
//!
//! Extension-based MIME mapping for all known legcord:// resources.
//! No MIME sniffing — content-type is determined exclusively by file extension.

use std::collections::HashMap;
use std::sync::LazyLock;

/// Extension to MIME type mapping.
/// All extensions used by current legcord:// consumers are included.
static MIME_MAP: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    let mut map = HashMap::new();
    // Text
    map.insert("html", "text/html; charset=utf-8");
    map.insert("htm", "text/html; charset=utf-8");
    map.insert("js", "application/javascript; charset=utf-8");
    map.insert("mjs", "application/javascript; charset=utf-8");
    map.insert("ts", "application/typescript; charset=utf-8");
    map.insert("json", "application/json; charset=utf-8");
    map.insert("css", "text/css; charset=utf-8");
    map.insert("txt", "text/plain; charset=utf-8");
    map.insert("md", "text/markdown; charset=utf-8");
    map.insert("wasm", "application/wasm");
    // Images
    map.insert("png", "image/png");
    map.insert("svg", "image/svg+xml");
    map.insert("jpg", "image/jpeg");
    map.insert("jpeg", "image/jpeg");
    map.insert("webp", "image/webp");
    map.insert("ico", "image/x-icon");
    map.insert("gif", "image/gif");
    map.insert("bmp", "image/bmp");
    map.insert("avif", "image/avif");
    // Fonts
    map.insert("woff", "font/woff");
    map.insert("woff2", "font/woff2");
    map.insert("ttf", "font/ttf");
    map.insert("otf", "font/otf");
    map.insert("eot", "application/vnd.ms-fontobject");
    // Media
    map.insert("webm", "video/webm");
    map.insert("mp4", "video/mp4");
    map.insert("map", "application/json; charset=utf-8");
    map.insert("mp3", "audio/mpeg");
    map.insert("wav", "audio/wav");
    map.insert("ogg", "audio/ogg");
    map.insert("m4a", "audio/mp4");
    // Archives
    map.insert("zip", "application/zip");
    map
});

/// Returns the MIME type for a given file extension, or `application/octet-stream` if unknown.
pub fn mime_for_extension(ext: &str) -> &'static str {
    MIME_MAP
        .get(ext)
        .copied()
        .unwrap_or("application/octet-stream")
}

/// Returns the MIME type for a given file path by extracting its extension.
pub fn mime_for_path(path: &std::path::Path) -> &'static str {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(mime_for_extension)
        .unwrap_or("application/octet-stream")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_extensions() {
        assert_eq!(mime_for_extension("html"), "text/html; charset=utf-8");
        assert_eq!(
            mime_for_extension("js"),
            "application/javascript; charset=utf-8"
        );
        assert_eq!(mime_for_extension("css"), "text/css; charset=utf-8");
        assert_eq!(mime_for_extension("png"), "image/png");
        assert_eq!(mime_for_extension("svg"), "image/svg+xml");
        assert_eq!(mime_for_extension("jpg"), "image/jpeg");
        assert_eq!(mime_for_extension("jpeg"), "image/jpeg");
        assert_eq!(mime_for_extension("webp"), "image/webp");
        assert_eq!(mime_for_extension("webm"), "video/webm");
        assert_eq!(
            mime_for_extension("json"),
            "application/json; charset=utf-8"
        );
        assert_eq!(mime_for_extension("wasm"), "application/wasm");
        assert_eq!(mime_for_extension("woff2"), "font/woff2");
    }

    #[test]
    fn unknown_extension_defaults_to_octet_stream() {
        assert_eq!(mime_for_extension("unknown"), "application/octet-stream");
        assert_eq!(mime_for_extension(""), "application/octet-stream");
    }

    #[test]
    fn path_based_lookup() {
        assert_eq!(
            mime_for_path(std::path::Path::new("test.html")),
            "text/html; charset=utf-8"
        );
        assert_eq!(
            mime_for_path(std::path::Path::new("assets/image.png")),
            "image/png"
        );
        assert_eq!(
            mime_for_path(std::path::Path::new("script.js")),
            "application/javascript; charset=utf-8"
        );
        assert_eq!(
            mime_for_path(std::path::Path::new("style.css")),
            "text/css; charset=utf-8"
        );
        assert_eq!(
            mime_for_path(std::path::Path::new("no_extension")),
            "application/octet-stream"
        );
    }

    #[test]
    fn case_insensitive_extension() {
        // MIME map uses lowercase keys; path extension preserves case
        assert_eq!(
            mime_for_path(std::path::Path::new("test.HTML")),
            "application/octet-stream"
        );
        assert_eq!(
            mime_for_path(std::path::Path::new("test.Html")),
            "application/octet-stream"
        );
        assert_eq!(
            mime_for_path(std::path::Path::new("test.html")),
            "text/html; charset=utf-8"
        );
    }
}
