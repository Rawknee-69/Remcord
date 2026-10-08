//! legcord:// custom protocol implementation for WebView2.
//!
//! This module provides:
//! - Custom scheme registration and request handling via WebResourceRequested
//! - Secure path resolution with hardening beyond Electron's weak validation
//! - MIME type mapping for all known legcord:// resource types
//! - Namespace-based root mapping matching Electron's behavior

pub mod csp;
pub mod handler;
pub mod mime;
pub mod resolver;
pub mod scheme;

#[allow(unused_imports)]
pub use csp::{extend_csp_for_legcord, LEGCORD_CSP_DIRECTIVES};
#[allow(unused_imports)]
pub use handler::{build_namespace_roots, register_legcord_protocol, status_for_error};
pub use resolver::NamespaceRoots;
// Re-exported for integration tests and future consumers.
#[allow(unused_imports)]
pub use mime::{mime_for_extension, mime_for_path};
#[allow(unused_imports)]
pub use resolver::{resolve_uri, Namespace, ResolveError};
