//! App icon rebrand contract (F29).
//!
//! `build/icon.ico` / `build/icon.png` already carry the Remcord artwork
//! (navy tile, white R, cyan bar; 512x512 PNG source, 4-image ICO) since
//! `ef67a92` ("assets: replace inherited product branding"). Three
//! consumers share those two files and must keep pointing at them:
//!
//! - the Windows exe embed (`native/build.rs` →
//!   `winres::WindowsResource::set_icon("../build/icon.ico")`),
//! - the Inno Setup icon (`native/installer/remcord.iss` → `SetupIconFile`),
//! - the Linux packaging (`electron-builder.ts` → `linux`/`deb` icon
//!   `"build/icon.png"`).
//!
//! This module pins that contract three ways so a future asset swap that
//! restores old branding, truncates a file, or drops a consumer fails fast
//! instead of shipping a half-rebranded product:
//!
//! - [`validate_icon_bytes`] parses both files structurally (ICO header +
//!   per-image bounds, PNG signature + `IHDR` dimensions), applies a size
//!   rail, and compares `sha256` digests against the rebranded_expected
//!   hashes. Unit tests run it against the real repo assets plus synthetic
//!   corruptions.
//! - [`validate_embedded_icons`] runs the same validation at startup against
//!   copies of the assets compiled into the binary (`include_bytes!`, so the
//!   check needs no repo or install-directory access at runtime): a corrupt
//!   or reverted asset fails the build's own boot instead of shipping.
//! - [`validate_icon_contract`] pins the consumer inventory the startup
//!   path passes in (same fail-fast shape as F4's
//!   `validate_mod_settings_contract`).
//! - The icon has no bridge surface: no capability, no facade method, no
//!   renderer input — deliberately, so there is nothing in
//!   `native/web/compatibility.js` to pin.
//!
//! Safety properties (all pinned by tests):
//!
//! - Pure byte validation only: no filesystem, network, registry, or shell
//!   access. The caller supplies the bytes; tests read the repo assets via
//!   `CARGO_MANIFEST_DIR`, never the live profile or install directory.
//! - Bounded inputs: files larger than [`MAX_ICON_FILE_BYTES`] are refused
//!   before parsing or hashing.
//! - No panics on hostile input: every out-of-bounds read maps to
//!   [`IconError`], including short headers, truncated directories, and
//!   offsets past the end of the file.

use sha2::Digest as _;
use thiserror::Error;

/// Repo-relative path of the shared Windows icon, as referenced from
/// `native/` (build script, installer script, and this module's tests).
pub const ICON_ICO_REL_PATH: &str = "../build/icon.ico";

/// Repo-relative path of the shared Linux/packaging icon.
pub const ICON_PNG_REL_PATH: &str = "../build/icon.png";

/// Every consumer that must keep pointing at the shared icon assets:
/// (consuming file, what it must reference). The startup path passes this
/// table back through [`validate_icon_contract`] so dropping or retargeting
/// a consumer fails the build-time tests and the boot check together.
pub const ICON_CONSUMERS: &[(&str, &str)] = &[
    (
        "native/build.rs",
        "winres::WindowsResource::set_icon(\"../build/icon.ico\")",
    ),
    (
        "native/installer/remcord.iss",
        "SetupIconFile=..\\..\\build\\icon.ico",
    ),
    ("electron-builder.ts", "linux/deb icon \"build/icon.png\""),
];

/// `sha256` of the rebranded `build/icon.ico` (lowercase hex). Any asset
/// swap — including a revert to old branding — changes the digest and must
/// update this constant explicitly.
pub const EXPECTED_ICON_ICO_SHA256: &str =
    "153a845071e6ce9c4deae5dbb43f85644b682d51ea5778e90044b91383cec58a";

/// `sha256` of the rebranded `build/icon.png` (lowercase hex).
pub const EXPECTED_ICON_PNG_SHA256: &str =
    "72dfa93e0f88033bd42b626f8e6b557db3a4275f083d77d71411fff89781140f";

/// Maximum accepted icon file size in bytes. The real assets are ~7–9 KiB;
/// this is a safety rail against hashing unbounded input, not a claim about
/// icon formats.
pub const MAX_ICON_FILE_BYTES: usize = 4 * 1024 * 1024;

/// Copy of `build/icon.ico` compiled into the binary, so the startup check
/// below needs no repo or install-directory access at runtime. The `winres`
/// exe embed (`native/build.rs`) reads the same file at compile time.
const EMBEDDED_ICO: &[u8] = include_bytes!("../../build/icon.ico");

/// Copy of `build/icon.png` compiled into the binary (same rationale).
const EMBEDDED_PNG: &[u8] = include_bytes!("../../build/icon.png");

/// PNG file signature (8 bytes).
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// Parsed-and-pinned icon facts returned by [`validate_icon_bytes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IconReport {
    /// Length of the validated `icon.ico` in bytes.
    pub ico_bytes: usize,
    /// Number of images in the validated `icon.ico`.
    pub ico_images: usize,
    /// Length of the validated `icon.png` in bytes.
    pub png_bytes: usize,
    /// `IHDR` width of the validated `icon.png`.
    pub png_width: u32,
    /// `IHDR` height of the validated `icon.png`.
    pub png_height: u32,
}

/// Byte-level icon validation failures. Every variant is constructible from
/// hostile input without I/O and without panicking.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum IconError {
    /// Input exceeds [`MAX_ICON_FILE_BYTES`]; refused before parsing.
    #[error("{file} is {bytes} bytes, over the {max}-byte icon size rail")]
    TooLarge {
        file: &'static str,
        bytes: usize,
        max: usize,
    },
    /// Input ends before the named structure could be read.
    #[error("{file} is truncated: {what}")]
    Truncated {
        file: &'static str,
        what: &'static str,
    },
    /// Wrong magic: ICO reserved/type fields or the PNG signature.
    #[error("{file} has an unrecognized magic header")]
    BadMagic { file: &'static str },
    /// ICO image count outside `1..=256`.
    #[error("icon.ico declares {count} images, outside 1..=256")]
    BadImageCount { count: usize },
    /// An ICO directory entry points outside the file or has a zero length.
    #[error("icon.ico image {index} points outside the file")]
    ImageOutOfBounds { index: usize },
    /// PNG `IHDR` dimensions are zero.
    #[error("icon.png has zero IHDR dimensions")]
    BadDimensions,
    /// Structural parse passed but the digest is not the rebranded asset.
    #[error("{file} digest does not match the rebranded asset")]
    HashMismatch { file: &'static str },
}

/// Parses an ICO header and directory, returning the image count. All
/// offsets and lengths are bounds-checked against `bytes`.
fn parse_ico_image_count(bytes: &[u8]) -> Result<usize, IconError> {
    const FILE: &str = "icon.ico";
    if bytes.len() < 6 {
        return Err(IconError::Truncated {
            file: FILE,
            what: "6-byte ICO header",
        });
    }
    let reserved = u16::from_le_bytes([bytes[0], bytes[1]]);
    let icon_type = u16::from_le_bytes([bytes[2], bytes[3]]);
    if reserved != 0 || icon_type != 1 {
        return Err(IconError::BadMagic { file: FILE });
    }
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    if count == 0 || count > 256 {
        return Err(IconError::BadImageCount { count });
    }
    let directory_bytes = count
        .checked_mul(16)
        .expect("count <= 256, so count * 16 cannot overflow");
    let directory_end = 6usize
        .checked_add(directory_bytes)
        .expect("directory length cannot overflow");
    if bytes.len() < directory_end {
        return Err(IconError::Truncated {
            file: FILE,
            what: "ICO image directory",
        });
    }
    for index in 0..count {
        let entry = &bytes[6 + index * 16..6 + (index + 1) * 16];
        let size = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]) as usize;
        let offset = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]) as usize;
        if size == 0 {
            return Err(IconError::ImageOutOfBounds { index });
        }
        let image_end = offset
            .checked_add(size)
            .ok_or(IconError::ImageOutOfBounds { index })?;
        // Image data must live after the directory and inside the file.
        if offset < directory_end || image_end > bytes.len() {
            return Err(IconError::ImageOutOfBounds { index });
        }
    }
    Ok(count)
}

/// Parses PNG signature + `IHDR` dimensions, returning `(width, height)`.
fn parse_png_dimensions(bytes: &[u8]) -> Result<(u32, u32), IconError> {
    const FILE: &str = "icon.png";
    // Signature (8) + length (4) + "IHDR" (4) + width/height (8) = 24 bytes.
    if bytes.len() < 24 {
        return Err(IconError::Truncated {
            file: FILE,
            what: "24-byte PNG signature + IHDR",
        });
    }
    if bytes[0..8] != PNG_SIGNATURE[..] {
        return Err(IconError::BadMagic { file: FILE });
    }
    if bytes[12..16] != *b"IHDR" {
        return Err(IconError::Truncated {
            file: FILE,
            what: "first PNG chunk must be IHDR",
        });
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    if width == 0 || height == 0 {
        return Err(IconError::BadDimensions);
    }
    Ok((width, height))
}

/// Lowercase hex `sha256` of `bytes` (no extra dependency; same loop as
/// `native/build.rs` uses for its embedded artifacts).
fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = sha2::Sha256::new();
    sha2::Digest::update(&mut hasher, bytes);
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// Validates both shared icon assets structurally and by digest, returning
/// the parsed facts. The caller supplies the bytes (tests read the repo
/// assets; the binary never reads icon files at runtime — the exe icon is
/// already embedded by `winres` at compile time).
pub fn validate_icon_bytes(ico: &[u8], png: &[u8]) -> Result<IconReport, IconError> {
    if ico.len() > MAX_ICON_FILE_BYTES {
        return Err(IconError::TooLarge {
            file: "icon.ico",
            bytes: ico.len(),
            max: MAX_ICON_FILE_BYTES,
        });
    }
    if png.len() > MAX_ICON_FILE_BYTES {
        return Err(IconError::TooLarge {
            file: "icon.png",
            bytes: png.len(),
            max: MAX_ICON_FILE_BYTES,
        });
    }
    let ico_images = parse_ico_image_count(ico)?;
    let (png_width, png_height) = parse_png_dimensions(png)?;
    if sha256_hex(ico) != EXPECTED_ICON_ICO_SHA256 {
        return Err(IconError::HashMismatch { file: "icon.ico" });
    }
    if sha256_hex(png) != EXPECTED_ICON_PNG_SHA256 {
        return Err(IconError::HashMismatch { file: "icon.png" });
    }
    Ok(IconReport {
        ico_bytes: ico.len(),
        ico_images,
        png_bytes: png.len(),
        png_width,
        png_height,
    })
}

/// Validates the compiled-in icon copies, returning the parsed facts. The
/// startup path calls this once and fails fast on any error: a truncated or
/// reverted asset can never boot a half-rebranded binary.
pub fn validate_embedded_icons() -> Result<IconReport, IconError> {
    validate_icon_bytes(EMBEDDED_ICO, EMBEDDED_PNG)
}

/// Pins the F29 consumer inventory. The startup path passes the asset paths
/// and consumer table it was built with; any drift from the constants above
/// (wrong path, dropped or added consumer, reordered table) fails fast with
/// a message naming the mismatch. Mirrors F4's contract-validator shape.
pub fn validate_icon_contract(
    ico_rel_path: &str,
    png_rel_path: &str,
    consumers: &[(&str, &str)],
) -> Result<(), String> {
    if ico_rel_path != ICON_ICO_REL_PATH {
        return Err(format!(
            "[app-icon] icon.ico path must be {ICON_ICO_REL_PATH:?}, found {ico_rel_path:?}"
        ));
    }
    if png_rel_path != ICON_PNG_REL_PATH {
        return Err(format!(
            "[app-icon] icon.png path must be {ICON_PNG_REL_PATH:?}, found {png_rel_path:?}"
        ));
    }
    if consumers != ICON_CONSUMERS {
        return Err(format!(
            "[app-icon] icon consumer inventory drifted: expected {} consumers {:?}, found {} consumers {:?}",
            ICON_CONSUMERS.len(),
            ICON_CONSUMERS,
            consumers.len(),
            consumers
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Absolute path of a repo-relative asset (tests run with CWD = the
    /// `native/` package dir, so anchor on the manifest dir instead).
    fn repo_asset(rel: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
    }

    fn read_repo_asset(rel: &str) -> Vec<u8> {
        std::fs::read(repo_asset(rel))
            .unwrap_or_else(|error| panic!("test requires repo asset {rel}: {error}"))
    }

    /// Minimal 1-image ICO builder for corruption tests: 6-byte header +
    /// one 16-byte entry + `image_len` payload bytes at the given offset.
    fn build_ico(count: u16, size: u32, offset: u32, image_len: usize) -> Vec<u8> {
        let mut ico = Vec::new();
        ico.extend_from_u16_bytes(count, size, offset);
        ico.extend(std::iter::repeat_n(0xABu8, image_len));
        ico
    }

    trait PushHeader {
        fn extend_from_u16_bytes(&mut self, count: u16, size: u32, offset: u32);
    }

    impl PushHeader for Vec<u8> {
        fn extend_from_u16_bytes(&mut self, count: u16, size: u32, offset: u32) {
            self.extend_from_slice(&[0, 0]); // reserved
            self.extend_from_slice(&[1, 0]); // type: icon
            self.extend_from_slice(&count.to_le_bytes());
            // One directory entry: 16x16, 0 colors, reserved 0, planes 1,
            // 32bpp, size, offset.
            let mut entry = vec![16u8, 16, 0, 0, 1, 0, 32, 0];
            entry.extend_from_slice(&size.to_le_bytes());
            entry.extend_from_slice(&offset.to_le_bytes());
            self.extend_from_slice(&entry);
        }
    }

    /// Minimal valid PNG (signature + `IHDR` with the given dimensions).
    fn build_png(width: u32, height: u32) -> Vec<u8> {
        let mut png = Vec::from(&PNG_SIGNATURE[..]);
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&width.to_be_bytes());
        png.extend_from_slice(&height.to_be_bytes());
        png.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit truecolor
        png.extend_from_slice(&[0, 0, 0, 0]); // fake CRC
        png
    }

    #[test]
    fn rebranded_ico_parses_with_images() {
        let ico = read_repo_asset(ICON_ICO_REL_PATH);
        let images = parse_ico_image_count(&ico).expect("rebranded icon.ico must parse");
        assert!(images >= 1, "icon.ico must contain at least one image");
    }

    #[test]
    fn rebranded_png_parses_at_expected_size() {
        let png = read_repo_asset(ICON_PNG_REL_PATH);
        let (width, height) = parse_png_dimensions(&png).expect("rebranded icon.png must parse");
        assert_eq!((width, height), (512, 512));
    }

    #[test]
    fn rebranded_assets_validate_with_digests() {
        let ico = read_repo_asset(ICON_ICO_REL_PATH);
        let png = read_repo_asset(ICON_PNG_REL_PATH);
        let report = validate_icon_bytes(&ico, &png).expect("rebranded assets must validate");
        assert_eq!(report.ico_bytes, ico.len());
        assert_eq!(report.png_bytes, png.len());
        assert_eq!((report.png_width, report.png_height), (512, 512));
    }

    #[test]
    fn embedded_copies_match_repo_assets() {
        // The compiled-in copies must validate identically: otherwise the
        // binary would boot-check different bytes than the repo ships.
        let report = validate_embedded_icons().expect("embedded icon copies must validate");
        let ico = read_repo_asset(ICON_ICO_REL_PATH);
        let png = read_repo_asset(ICON_PNG_REL_PATH);
        assert_eq!(report.ico_bytes, ico.len());
        assert_eq!(report.png_bytes, png.len());
        assert!(report.ico_images >= 1);
    }

    #[test]
    fn swapped_old_branding_fails_hash() {
        // A structurally valid but different asset (e.g. the pre-rebrand
        // icon) must NOT validate: the digest pin is the rebrand guard.
        let png = read_repo_asset(ICON_PNG_REL_PATH);
        let mut other_ico = build_ico(1, 4, 22, 4);
        // Repair the payload so structure parses; digest still differs.
        other_ico.truncate(22 + 4);
        let ico = read_repo_asset(ICON_ICO_REL_PATH);
        assert_eq!(
            validate_icon_bytes(&other_ico, &png),
            Err(IconError::HashMismatch { file: "icon.ico" })
        );
        let mut other_png = build_png(512, 512);
        other_png.extend_from_slice(b"rebrand-drift");
        assert_eq!(
            validate_icon_bytes(&ico, &other_png),
            Err(IconError::HashMismatch { file: "icon.png" })
        );
    }

    #[test]
    fn truncated_ico_header_rejected() {
        assert_eq!(
            parse_ico_image_count(&[0, 0, 1]),
            Err(IconError::Truncated {
                file: "icon.ico",
                what: "6-byte ICO header",
            })
        );
    }

    #[test]
    fn non_icon_type_rejected() {
        // Reserved/type = 0/2 (cursor type, not icon).
        assert_eq!(
            parse_ico_image_count(&[0, 0, 2, 0, 1, 0]),
            Err(IconError::BadMagic { file: "icon.ico" })
        );
    }

    #[test]
    fn zero_image_count_rejected() {
        assert_eq!(
            parse_ico_image_count(&[0, 0, 1, 0, 0, 0]),
            Err(IconError::BadImageCount { count: 0 })
        );
    }

    #[test]
    fn truncated_directory_rejected() {
        // Declares 2 images but carries only one entry.
        let mut ico = vec![0u8, 0, 1, 0, 2, 0];
        ico.extend(std::iter::repeat_n(0u8, 16));
        assert_eq!(
            parse_ico_image_count(&ico),
            Err(IconError::Truncated {
                file: "icon.ico",
                what: "ICO image directory",
            })
        );
    }

    #[test]
    fn out_of_bounds_image_rejected() {
        // Entry offset points past the end of the file.
        let ico = build_ico(1, 4, 10_000, 4);
        assert_eq!(
            parse_ico_image_count(&ico),
            Err(IconError::ImageOutOfBounds { index: 0 })
        );
    }

    #[test]
    fn zero_length_image_rejected() {
        let ico = build_ico(1, 0, 22, 0);
        assert_eq!(
            parse_ico_image_count(&ico),
            Err(IconError::ImageOutOfBounds { index: 0 })
        );
    }

    #[test]
    fn bad_png_signature_rejected() {
        let mut png = build_png(512, 512);
        png[0] = 0x00;
        assert_eq!(
            parse_png_dimensions(&png),
            Err(IconError::BadMagic { file: "icon.png" })
        );
    }

    #[test]
    fn truncated_png_rejected() {
        assert_eq!(
            parse_png_dimensions(&PNG_SIGNATURE[..]),
            Err(IconError::Truncated {
                file: "icon.png",
                what: "24-byte PNG signature + IHDR",
            })
        );
    }

    #[test]
    fn zero_png_dimensions_rejected() {
        assert_eq!(
            parse_png_dimensions(&build_png(0, 512)),
            Err(IconError::BadDimensions)
        );
    }

    #[test]
    fn oversize_inputs_refused_before_parsing() {
        let big = vec![0u8; MAX_ICON_FILE_BYTES + 1];
        let small = build_png(1, 1);
        assert_eq!(
            validate_icon_bytes(&big, &small),
            Err(IconError::TooLarge {
                file: "icon.ico",
                bytes: big.len(),
                max: MAX_ICON_FILE_BYTES,
            })
        );
        assert_eq!(
            validate_icon_bytes(&small, &big),
            Err(IconError::TooLarge {
                file: "icon.png",
                bytes: big.len(),
                max: MAX_ICON_FILE_BYTES,
            })
        );
    }

    #[test]
    fn contract_accepts_current_inventory() {
        validate_icon_contract(ICON_ICO_REL_PATH, ICON_PNG_REL_PATH, ICON_CONSUMERS)
            .expect("current inventory must satisfy the contract");
    }

    #[test]
    fn contract_rejects_drift() {
        assert!(
            validate_icon_contract("build/old.ico", ICON_PNG_REL_PATH, ICON_CONSUMERS).is_err()
        );
        assert!(
            validate_icon_contract(ICON_ICO_REL_PATH, "build/old.png", ICON_CONSUMERS).is_err()
        );
        assert!(validate_icon_contract(ICON_ICO_REL_PATH, ICON_PNG_REL_PATH, &[]).is_err());
        let mut extra = ICON_CONSUMERS.to_vec();
        extra.push(("some/new/consumer.ts", "icon.png"));
        assert!(validate_icon_contract(ICON_ICO_REL_PATH, ICON_PNG_REL_PATH, &extra).is_err());
    }
}
