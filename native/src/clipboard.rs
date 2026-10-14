//! Host-owned image-to-clipboard copy (`clipboard.copy_image` capability).
//!
//! Traced Electron behavior (actual implementation, not declarations):
//! - `src/discord/ipc.ts` uses Electron's `clipboard` module for the
//!   debug/GPU text copies (already ported as the renderer-side
//!   `diagnostics.get_info` / `graphics.get_info` reads + a
//!   `navigator.clipboard` write). Image copies in Electron ride the
//!   Chromium context menu, which `src/discord/window.ts` suppresses
//!   outright — there is no Electron `copyImage(url)` IPC to port.
//!
//! Native contract (closest safe equivalent, documented differences):
//! - The renderer supplies exactly one `http`/`https` image URL (never
//!   bytes, paths, selectors, or dimensions). The host fetches the bytes
//!   itself with the same bounds discipline as gallery downloads (scheme
//!   allowlist, redirect cap, timeout, byte cap), refuses non-image
//!   payloads by `Content-Type` (SVG is markup, not a raster the clipboard
//!   bitmap format can carry), decodes with in-box WIC (PNG/JPEG/GIF/BMP;
//!   animated images resolve to their first frame), and places a `CF_DIB`
//!   on the Windows clipboard. No new dependency: WIC, the clipboard, and
//!   the global heap all ship with Windows.
//! - Typed failures resolve `{ok: false, error: <CODE>}` (never reject) so
//!   the page can toast the reason, mirroring `gallery.add_url`. Only the
//!   fixed-vocabulary code crosses; the detail stays in stderr.

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::Graphics::Gdi::BITMAPINFOHEADER;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppBGRA, IWICImagingFactory,
    WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_DIB;

/// Maximum clipboard image URL length in bytes. Mirrors gallery's
/// `MAX_URL_BYTES`: the renderer supplies a location, never content.
pub const MAX_CLIPBOARD_IMAGE_URL_BYTES: usize = 2048;

/// Maximum downloaded image bytes. Smaller than the gallery media cap: the
/// decoded bitmap multiplies the download ~4x, so the fetch stays tight.
pub const CLIPBOARD_DOWNLOAD_MAX_BYTES: usize = 8 * 1024 * 1024;

/// Network timeout for one interactive copy. Shorter than gallery's 60 s:
/// the user is waiting on a toast, not a library import.
pub const CLIPBOARD_DOWNLOAD_TIMEOUT_SECS: u64 = 30;

/// Redirect cap, mirroring gallery downloads.
pub const CLIPBOARD_DOWNLOAD_MAX_REDIRECTS: u32 = 5;

/// Maximum decoded width/height in pixels. Bounds the `CF_DIB` allocation
/// before WIC hands us the pixels.
pub const MAX_CLIPBOARD_IMAGE_DIMENSION: u32 = 4096;

/// Maximum decoded pixel count (16 MP). At 4 bytes per pixel the clipboard
/// payload stays under ~64 MiB even at the cap.
pub const MAX_CLIPBOARD_IMAGE_PIXELS: u64 = 16_777_216;

/// Fixed-vocabulary failure for one clipboard image copy. Only `code`
/// crosses the bridge (inside `{ok: false, error}`); `message` is
/// stderr-only and never contains URLs, paths, or response bodies.
#[derive(Debug)]
pub struct ClipboardFailure {
    pub code: &'static str,
    pub message: String,
}

/// Dimensions of the bitmap placed on the clipboard.
pub struct CopiedImage {
    pub width: u32,
    pub height: u32,
}

fn failure(code: &'static str, message: impl Into<String>) -> ClipboardFailure {
    ClipboardFailure {
        code,
        message: message.into(),
    }
}

/// URL rule for `clipboard.copy_image`: trimmed, non-empty, within the byte
/// cap, `http`/`https` only. Anything else is refused fail-closed — the
/// host never fetches what the renderer cannot name on the open web, and
/// overlong input is rejected (never silently truncated).
pub fn validate_clipboard_image_url(url: &str) -> Result<String, ClipboardFailure> {
    let clean = url.trim();
    if clean.is_empty() || clean.len() > MAX_CLIPBOARD_IMAGE_URL_BYTES {
        return Err(failure(
            "INVALID_URL",
            "clipboard image URL must be 1..=2048 bytes",
        ));
    }
    let lower = clean.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err(failure(
            "INVALID_URL",
            "clipboard image URL must be http(s)",
        ));
    }
    Ok(clean.to_owned())
}

/// Downloads one remote image for clipboard placement. Only `http`/`https`
/// (validated before this runs); bounded bytes, bounded redirects, bounded
/// time. Returns `(bytes, mime)`. SVG is refused here: WIC cannot rasterize
/// markup into the bitmap the clipboard carries.
pub fn download_image_bytes(url: &str) -> Result<(Vec<u8>, String), ClipboardFailure> {
    let agent = ureq::AgentBuilder::new()
        .redirects(CLIPBOARD_DOWNLOAD_MAX_REDIRECTS)
        .timeout(std::time::Duration::from_secs(
            CLIPBOARD_DOWNLOAD_TIMEOUT_SECS,
        ))
        .build();
    let response = agent
        .get(url)
        .call()
        .map_err(|_| failure("DOWNLOAD_FAILED", "clipboard image download failed"))?;
    if !(200..300).contains(&response.status()) {
        return Err(failure(
            "DOWNLOAD_FAILED",
            "clipboard image download was not successful",
        ));
    }
    let mime = response
        .header("content-type")
        .and_then(|raw| raw.split(';').next())
        .map(str::trim)
        .filter(|mime| !mime.is_empty() && mime.len() <= 128)
        .unwrap_or("application/octet-stream")
        .to_owned();
    let lower = mime.to_ascii_lowercase();
    if !lower.starts_with("image/") || lower == "image/svg+xml" {
        return Err(failure(
            "NOT_AN_IMAGE",
            "clipboard image URL did not serve a raster image",
        ));
    }
    let mut bytes = Vec::new();
    use std::io::Read;
    response
        .into_reader()
        .take(CLIPBOARD_DOWNLOAD_MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            failure(
                "DOWNLOAD_FAILED",
                "clipboard image download could not be read",
            )
        })?;
    if bytes.is_empty() || bytes.len() > CLIPBOARD_DOWNLOAD_MAX_BYTES {
        return Err(failure(
            "TOO_LARGE",
            "clipboard image exceeds the download size limit",
        ));
    }
    Ok((bytes, mime))
}

/// Decodes image bytes to top-down 32-bit BGRA pixels with in-box WIC.
/// Returns `(width, height, pixels)`. Dimensions are bounds-checked before
/// the pixel buffer is allocated; animated images resolve to frame 0.
/// COM is initialized on the calling thread for exactly the duration of
/// the decode (balanced `CoUninitialize` only when this call initialized).
pub fn decode_image_bgra(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), ClipboardFailure> {
    if bytes.is_empty() {
        return Err(failure("DECODE_FAILED", "clipboard image bytes are empty"));
    }
    let decode_failed = |detail: &str| failure("DECODE_FAILED", detail);
    // `S_OK`/`S_FALSE` both mean "COM usable, ours to uninitialize".
    // `RPC_E_CHANGED_MODE` (already MTA) means "usable, not ours" — WIC's
    // factory is free-threaded, so the decode still proceeds.
    let own_com =
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.is_ok();
    let decoded = decode_image_bgra_inner(bytes);
    if own_com {
        unsafe { CoUninitialize() };
    }
    decoded.map_err(decode_failed)
}

fn decode_image_bgra_inner(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), &'static str> {
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
                .map_err(|_| "WIC factory unavailable")?;
        let stream = factory
            .CreateStream()
            .map_err(|_| "WIC stream unavailable")?;
        stream
            .InitializeFromMemory(bytes)
            .map_err(|_| "WIC stream rejected the bytes")?;
        let decoder = factory
            .CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)
            .map_err(|_| "no WIC decoder recognizes these bytes")?;
        let frame = decoder
            .GetFrame(0)
            .map_err(|_| "image has no first frame")?;
        let mut width = 0u32;
        let mut height = 0u32;
        frame
            .GetSize(&mut width, &mut height)
            .map_err(|_| "image size unreadable")?;
        if width == 0 || height == 0 {
            return Err("image has zero width or height");
        }
        if width > MAX_CLIPBOARD_IMAGE_DIMENSION || height > MAX_CLIPBOARD_IMAGE_DIMENSION {
            return Err("image dimensions exceed the clipboard limit");
        }
        let pixels = (width as u64)
            .checked_mul(height as u64)
            .filter(|count| *count <= MAX_CLIPBOARD_IMAGE_PIXELS)
            .ok_or("image pixel count exceeds the clipboard limit")?;
        let converter = factory
            .CreateFormatConverter()
            .map_err(|_| "WIC format converter unavailable")?;
        converter
            .Initialize(
                &frame,
                &GUID_WICPixelFormat32bppBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
            .map_err(|_| "image cannot convert to 32-bit BGRA")?;
        let stride = width.checked_mul(4).ok_or("image stride overflows")?;
        let len = pixels as usize * 4;
        let mut bgra = vec![0u8; len];
        converter
            .CopyPixels(std::ptr::null(), stride, &mut bgra)
            .map_err(|_| "image pixels unreadable")?;
        Ok((width, height, bgra))
    }
}

/// Builds one `CF_DIB` buffer (header + bottom-up rows) from top-down BGRA
/// pixels. Pure function: no COM, no clipboard, fully unit-testable.
/// Bottom-up (positive height) is the universally accepted orientation;
/// the 4-byte BGRA stride is already DWORD-aligned so no row padding is
/// needed.
pub fn build_clipboard_dib(
    width: u32,
    height: u32,
    bgra_top_down: &[u8],
) -> Result<Vec<u8>, ClipboardFailure> {
    if width == 0
        || height == 0
        || width > MAX_CLIPBOARD_IMAGE_DIMENSION
        || height > MAX_CLIPBOARD_IMAGE_DIMENSION
    {
        return Err(failure(
            "IMAGE_TOO_LARGE",
            "clipboard image dimensions are outside 1..=4096",
        ));
    }
    let pixel_count = (width as u64)
        .checked_mul(height as u64)
        .filter(|count| *count <= MAX_CLIPBOARD_IMAGE_PIXELS)
        .ok_or_else(|| {
            failure(
                "IMAGE_TOO_LARGE",
                "clipboard image pixel count exceeds the limit",
            )
        })?;
    if bgra_top_down.len() != pixel_count as usize * 4 {
        return Err(failure(
            "DECODE_FAILED",
            "clipboard pixel buffer does not match its dimensions",
        ));
    }
    let header = BITMAPINFOHEADER {
        biSize: 40,
        biWidth: width as i32,
        biHeight: height as i32,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: 0, // BI_RGB: no compression, no palette.
        biSizeImage: (pixel_count as u32).saturating_mul(4),
        biXPelsPerMeter: 0,
        biYPelsPerMeter: 0,
        biClrUsed: 0,
        biClrImportant: 0,
    };
    let header_bytes: &[u8] =
        unsafe { std::slice::from_raw_parts(&header as *const BITMAPINFOHEADER as *const u8, 40) };
    let stride = width as usize * 4;
    let mut dib = Vec::with_capacity(40 + bgra_top_down.len());
    dib.extend_from_slice(header_bytes);
    for row in (0..height as usize).rev() {
        let start = row * stride;
        dib.extend_from_slice(&bgra_top_down[start..start + stride]);
    }
    Ok(dib)
}

/// Places one `CF_DIB` buffer on the Windows clipboard, replacing its
/// current contents. A clipboard held open by another application resolves
/// `CLIPBOARD_BUSY` (never blocks or retries); every other Win32 failure
/// resolves `CLIPBOARD_FAILED`. On success the system owns the handle.
pub fn write_dib_to_clipboard(dib: &[u8]) -> Result<(), ClipboardFailure> {
    if dib.len() < 44 || dib.len() > MAX_CLIPBOARD_IMAGE_PIXELS as usize * 4 + 40 {
        return Err(failure(
            "IMAGE_TOO_LARGE",
            "clipboard DIB is outside the size limit",
        ));
    }
    unsafe {
        let handle: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, dib.len())
            .map_err(|_| failure("CLIPBOARD_FAILED", "clipboard allocation failed"))?;
        let copy_ok = (|| {
            let dest = GlobalLock(handle);
            if dest.is_null() {
                return false;
            }
            std::ptr::copy_nonoverlapping(dib.as_ptr(), dest as *mut u8, dib.len());
            let _ = GlobalUnlock(handle);
            true
        })();
        if !copy_ok {
            let _ = GlobalFree(Some(handle));
            return Err(failure("CLIPBOARD_FAILED", "clipboard copy failed"));
        }
        if OpenClipboard(None).is_err() {
            let _ = GlobalFree(Some(handle));
            return Err(failure(
                "CLIPBOARD_BUSY",
                "clipboard is held open by another application",
            ));
        }
        let placed = EmptyClipboard()
            .and_then(|()| SetClipboardData(CF_DIB.0 as u32, Some(HANDLE(handle.0))))
            .is_ok();
        let _ = CloseClipboard();
        if !placed {
            let _ = GlobalFree(Some(handle));
            return Err(failure(
                "CLIPBOARD_FAILED",
                "clipboard could not take the image",
            ));
        }
        Ok(())
    }
}

/// Copies one image URL to the Windows clipboard: validate, download,
/// decode, build the DIB, place it. Single attempt, honest typed errors.
pub fn copy_image_url_to_clipboard(url: &str) -> Result<CopiedImage, ClipboardFailure> {
    let clean = validate_clipboard_image_url(url)?;
    let (bytes, _) = download_image_bytes(&clean)?;
    let (width, height, bgra) = decode_image_bgra(&bytes)?;
    let dib = build_clipboard_dib(width, height, &bgra)?;
    write_dib_to_clipboard(&dib)?;
    Ok(CopiedImage { width, height })
}

#[cfg(test)]
mod tests {
    use super::{
        build_clipboard_dib, decode_image_bgra, validate_clipboard_image_url,
        CLIPBOARD_DOWNLOAD_MAX_BYTES, MAX_CLIPBOARD_IMAGE_DIMENSION, MAX_CLIPBOARD_IMAGE_PIXELS,
        MAX_CLIPBOARD_IMAGE_URL_BYTES,
    };

    #[test]
    fn accepts_http_and_https_image_urls() {
        let got = validate_clipboard_image_url("https://cdn.discordapp.com/attachments/a/b.png")
            .expect("https must validate");
        assert_eq!(got, "https://cdn.discordapp.com/attachments/a/b.png");
        assert!(validate_clipboard_image_url("http://example.com/i.jpg").is_ok());
    }

    #[test]
    fn trims_surrounding_whitespace_but_rejects_the_rest() {
        let got = validate_clipboard_image_url("  https://example.com/i.png\n")
            .expect("padded https must validate");
        assert_eq!(got, "https://example.com/i.png");
        for hostile in [
            "",
            "   ",
            "ftp://example.com/i.png",
            "file:///C:/Windows/pic.png",
            "javascript:alert(1)",
            "data:image/png;base64,iVBOR",
            "//example.com/i.png",
            "example.com/i.png",
            "HT TP://example.com/i.png",
        ] {
            assert!(
                validate_clipboard_image_url(hostile).is_err(),
                "{hostile:?} must be refused"
            );
        }
    }

    #[test]
    fn rejects_overlong_urls_fail_closed() {
        let long = format!("https://example.com/{}.png", "a".repeat(2500));
        assert!(long.len() > MAX_CLIPBOARD_IMAGE_URL_BYTES);
        assert!(validate_clipboard_image_url(&long).is_err());
        // Exactly at the cap still validates: the bound is inclusive.
        let edge = format!("https://example.com/{}.png", "a".repeat(2024));
        assert_eq!(edge.len(), MAX_CLIPBOARD_IMAGE_URL_BYTES);
        assert!(validate_clipboard_image_url(&edge).is_ok());
    }

    #[test]
    fn error_codes_are_fixed_vocabulary() {
        let failure = validate_clipboard_image_url("ftp://example.com/i.png")
            .expect_err("ftp must be refused");
        assert_eq!(failure.code, "INVALID_URL");
        // The detail never echoes the hostile input.
        assert!(!failure.message.contains("ftp"));
    }

    #[test]
    fn builds_a_bottom_up_dib_from_top_down_bgra() {
        // 2x2: red, green / blue, white (BGRA order, top row first).
        let pixels = vec![
            0, 0, 255, 255, 0, 255, 0, 255, // top row: red, green
            255, 0, 0, 255, 255, 255, 255, 255, // bottom row: blue, white
        ];
        let dib = build_clipboard_dib(2, 2, &pixels).expect("2x2 must build");
        assert_eq!(dib.len(), 40 + 16);
        // BITMAPINFOHEADER, little-endian: size, width, height (+2 bottom-up).
        assert_eq!(&dib[0..4], &40u32.to_le_bytes());
        assert_eq!(&dib[4..8], &2i32.to_le_bytes());
        assert_eq!(&dib[8..12], &2i32.to_le_bytes());
        assert_eq!(&dib[14..16], &32u16.to_le_bytes());
        // First pixel row in the DIB is the image's BOTTOM row (blue, white).
        assert_eq!(&dib[40..48], &[255, 0, 0, 255, 255, 255, 255, 255]);
        // Second row is the TOP row (red, green).
        assert_eq!(&dib[48..56], &[0, 0, 255, 255, 0, 255, 0, 255]);
    }

    #[test]
    fn dib_build_rejects_dimension_and_buffer_mismatches() {
        let pixels = vec![0u8; 16];
        assert!(build_clipboard_dib(0, 2, &pixels).is_err());
        assert!(build_clipboard_dib(2, 0, &pixels).is_err());
        assert!(build_clipboard_dib(2, 2, &[0u8; 15]).is_err());
        assert!(build_clipboard_dib(2, 2, &[0u8; 17]).is_err());
        let oversize = MAX_CLIPBOARD_IMAGE_DIMENSION + 1;
        assert!(build_clipboard_dib(oversize, 2, &pixels).is_err());
        // A pixel count past the cap is refused even when each side fits.
        assert!(build_clipboard_dib(4096, 4096, &[]).is_err());
        assert_eq!(CLIPBOARD_DOWNLOAD_MAX_BYTES, 8 * 1024 * 1024);
        assert_eq!(MAX_CLIPBOARD_IMAGE_PIXELS, 16_777_216);
    }

    /// Minimal 1x1 red PNG (color type 2, bit depth 8): decodes without
    /// network or clipboard access — WIC ships with Windows.
    const RED_1X1_PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8,
        0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xFB, 0x03, 0x0F, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn decodes_a_1x1_png_to_bgra_through_wic() {
        let (width, height, pixels) =
            decode_image_bgra(RED_1X1_PNG).expect("in-box WIC must decode a 1x1 PNG");
        assert_eq!((width, height), (1, 1));
        // Red in BGRA: blue=0, green=0, red=255, alpha=255.
        assert_eq!(pixels, vec![0, 0, 255, 255]);
    }

    #[test]
    fn decode_rejects_non_images_fail_closed() {
        for hostile in [
            vec![],
            b"not an image at all".to_vec(),
            b"%PDF-1.4 fake".to_vec(),
            b"<svg xmlns='http://www.w3.org/2000/svg'></svg>".to_vec(),
        ] {
            let failure = decode_image_bgra(&hostile).expect_err("must refuse");
            assert_eq!(failure.code, "DECODE_FAILED");
        }
    }

    /// Live round trip (ignored by default): serves the 1x1 PNG over
    /// loopback, copies it through the real pipeline
    /// (fetch → WIC → `CF_DIB` → clipboard), and reads the DIB back.
    /// Run explicitly on a machine with a window station:
    /// `REMCORD_NATIVE_CLIPBOARD_SMOKE=1 cargo test clipboard -- --ignored`.
    /// Opt-in only: it replaces the real clipboard contents.
    #[test]
    #[ignore]
    fn live_loopback_copy_round_trip() {
        if std::env::var("REMCORD_NATIVE_CLIPBOARD_SMOKE").as_deref() != Ok("1") {
            return;
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback bind");
        let port = listener.local_addr().expect("local addr").port();
        let server = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept().expect("accept");
            let mut head = [0u8; 1024];
            let _ = stream.read(&mut head);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                RED_1X1_PNG.len()
            );
            stream.write_all(response.as_bytes()).expect("headers");
            stream.write_all(RED_1X1_PNG).expect("body");
        });
        let url = format!("http://127.0.0.1:{port}/red.png");
        let copied =
            super::copy_image_url_to_clipboard(&url).expect("live loopback copy must succeed");
        assert_eq!((copied.width, copied.height), (1, 1));
        server.join().expect("server thread");
        // Read the DIB back: 40-byte header + one bottom-up BGRA pixel.
        unsafe {
            use windows::Win32::Foundation::HGLOBAL;
            use windows::Win32::System::DataExchange::{
                CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
            };
            use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
            use windows::Win32::System::Ole::CF_DIB;
            let format = CF_DIB.0 as u32;
            assert!(IsClipboardFormatAvailable(format).is_ok());
            OpenClipboard(None).expect("open clipboard for read-back");
            let dib = (|| {
                let handle = GetClipboardData(format).ok()?;
                let global = HGLOBAL(handle.0);
                let ptr = GlobalLock(global);
                if ptr.is_null() {
                    return None;
                }
                let size = GlobalSize(global);
                let bytes = std::slice::from_raw_parts(ptr as *const u8, size).to_vec();
                let _ = GlobalUnlock(global);
                Some(bytes)
            })();
            let _ = CloseClipboard();
            let dib = dib.expect("DIB must read back");
            assert_eq!(dib.len(), 44);
            assert_eq!(&dib[0..4], &40u32.to_le_bytes());
            assert_eq!(&dib[4..8], &1i32.to_le_bytes());
            assert_eq!(&dib[8..12], &1i32.to_le_bytes());
            assert_eq!(&dib[40..44], &[0, 0, 255, 255]);
        }
    }
}
