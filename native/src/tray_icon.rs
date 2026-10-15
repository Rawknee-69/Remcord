//! Windows system-tray icon (real OS surface for the F13 tray model).
//!
//! Electron parity (`src/discord/tray.ts`): an icon in the notification
//! area with a context menu and click-to-reopen. Runs on a dedicated STA
//! worker thread that owns its hidden window, icon, and menu; the main
//! (tao) thread never touches tray Win32 directly. Menu/click actions cross
//! back as [`crate::NativeEvent`]s over the tao event-loop proxy, so all
//! window work stays on the main thread.
//!
//! Design notes:
//! - The icon comes from the embedded rebranded PNG (`app_icon`), decoded
//!   through the same WIC path as clipboard images and converted to an
//!   `HICON` via `CreateIconIndirect` (color + alpha-derived mask). No
//!   loose files, no shell icon cache, no invented bytes.
//! - Dynamic favicon/badge icons (Electron `dynamic` mode) are NOT
//!   implemented: the icon is the static app mark in every mode. The mode
//!   still selects close/startup policy through the F13 model.
//! - Shutdown is orderly: dropping [`TrayHandle`] signals the worker,
//!   which removes the icon (`NIM_DELETE`) before its thread exits, so no
//!   orphan icon lingers after quit.
//! - Threading: the worker pumps its own `PeekMessageW` loop. tao never sees
//!   the tray window; the tray never touches tao or the WebView.
//! - `ThreadState` below is defined ONCE and shared by reference between the
//!   worker setup and the window procedure (same layout, same type): the
//!   pointer stored in `GWLP_USERDATA` is reclaimed exactly once at
//!   teardown.

use std::sync::mpsc;

/// Tray menu command ids (Electron `tray.ts` menu parity, reduced to the
/// host-ownable actions: reopen and quit).
const MENU_OPEN_ID: u32 = 1;
const MENU_QUIT_ID: u32 = 2;

/// Application-defined callback message for `Shell_NotifyIconW`.
const WM_TRAY_CALLBACK: u32 = 0x0400 + 0x100;

/// Tooltip text (fixed vocabulary, mirrors the window title).
const TRAY_TOOLTIP: &str = "Remcord";

/// Tray artwork bundled in `assets/` (Electron tray-icon parity). The
/// `dynamic` mode and the default `dsc-tray` use the embedded rebranded
/// mark; every other `ValidTrayIcons` value names its file. Unknown values
/// fall back to the mark (the F13 model maps them to `Static`, never an
/// error).
pub fn tray_art_bytes(mode: &str) -> &'static [u8] {
    match mode {
        "clsc-dsc-tray" => include_bytes!("../../assets/clsc-dsc-tray.png"),
        "ac_plug_colored" => include_bytes!("../../assets/ac_plug_colored.png"),
        "ac_white_plug" => include_bytes!("../../assets/ac_white_plug.png"),
        "ac_white_plug_hollow" => include_bytes!("../../assets/ac_white_plug_hollow.png"),
        "ac_black_plug" => include_bytes!("../../assets/ac_black_plug.png"),
        "ac_black_plug_hollow" => include_bytes!("../../assets/ac_black_plug_hollow.png"),
        _ => crate::app_icon::embedded_png_bytes(),
    }
}

/// Decode arbitrary PNG bytes (tray art selection) to top-down BGRA.
fn tray_bitmap_pixels_from(png: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let (width, height, bgra) =
        crate::clipboard::decode_image_bgra(png).map_err(|failure| failure.code.to_owned())?;
    if width == 0 || height == 0 {
        return Err("tray PNG has zero dimensions".to_owned());
    }
    Ok((width, height, bgra))
}

/// Live handle to the running tray worker. Dropping removes the icon and
/// stops the worker thread.
pub struct TrayHandle {
    shutdown_tx: mpsc::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for TrayHandle {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Spawn the tray worker and add the icon. Returns the live handle, or a
/// fixed-vocabulary error when the icon cannot be built or the worker
/// fails to start. The caller treats `Err` as "no usable tray" (close
/// quits, startup shows) — never a boot failure.
///
/// The icon is built on the worker thread itself: `HICON` is a raw pointer
/// wrapper that is not `Send`, so only the PNG bytes (static, `Send`)
/// cross the spawn boundary. `art` selects the `tray` setting's artwork
/// (see [`tray_art_bytes`]).
pub fn spawn_tray(
    proxy: tao::event_loop::EventLoopProxy<crate::NativeEvent>,
    art: &'static [u8],
) -> Result<TrayHandle, String> {
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();
    let thread = std::thread::Builder::new()
        .name("remcord-tray-worker".to_owned())
        .spawn(move || run_tray_worker(proxy, art, ready_tx, shutdown_rx))
        .map_err(|error| format!("tray worker failed to start: {error}"))?;
    match ready_rx
        .recv()
        .map_err(|_| "tray worker startup channel closed".to_owned())?
    {
        Ok(()) => Ok(TrayHandle {
            shutdown_tx,
            thread: Some(thread),
        }),
        Err(reason) => Err(reason),
    }
}

/// Build an `HICON` from tray artwork bytes: 32-bit color bitmap (bottom-up
/// DIB, as `CreateIconIndirect` requires) plus a 1-bpp AND mask derived
/// from the alpha channel (opaque where alpha >= 128). Caller must
/// `DestroyIcon` the result.
#[cfg(target_os = "windows")]
fn build_tray_icon_from(
    png: &[u8],
) -> Result<windows::Win32::UI::WindowsAndMessaging::HICON, String> {
    use windows::Win32::{
        Foundation::HWND,
        Graphics::Gdi::{
            CreateBitmap, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, BITMAPINFO,
            BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        },
        UI::WindowsAndMessaging::{CreateIconIndirect, ICONINFO},
    };
    let (width, height, bgra) = tray_bitmap_pixels_from(png)?;
    let (width, height) = (width as i32, height as i32);
    unsafe {
        let screen_dc = CreateCompatibleDC(None);
        if screen_dc.is_invalid() {
            return Err("tray device context unavailable".to_owned());
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            ..Default::default()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let color =
            match CreateDIBSection(Some(screen_dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(bitmap) => bitmap,
                Err(_) => {
                    let _ = DeleteDC(screen_dc);
                    return Err("tray color bitmap unavailable".to_owned());
                }
            };
        if bits.is_null() {
            let _ = DeleteObject(color.into());
            let _ = DeleteDC(screen_dc);
            return Err("tray color bits unavailable".to_owned());
        }
        // DIB rows are bottom-up: flip the top-down WIC rows.
        let stride = width as usize * 4;
        let dst = std::slice::from_raw_parts_mut(bits as *mut u8, stride * height as usize);
        for row in 0..height as usize {
            let src_off = row * stride;
            let dst_off = (height as usize - 1 - row) * stride;
            dst[dst_off..dst_off + stride].copy_from_slice(&bgra[src_off..src_off + stride]);
        }
        // 1-bpp AND mask: 0 bit (black) = opaque, 1 bit (white) =
        // transparent. Derive from alpha so translucent pixels do not
        // leave dark fringes.
        let mask_row_bytes = (width as usize).div_ceil(32) * 4;
        let mut mask_bits = vec![0u8; mask_row_bytes * height as usize];
        for y in 0..height as usize {
            for x in 0..width as usize {
                // Bottom-up DIB row order for the mask as well.
                let src_alpha = bgra[(y * width as usize + x) * 4 + 3];
                if src_alpha < 128 {
                    let byte = (height as usize - 1 - y) * mask_row_bytes + x / 8;
                    mask_bits[byte] |= 1 << (7 - (x % 8));
                }
            }
        }
        let mask = CreateBitmap(width, height, 1, 1, Some(mask_bits.as_ptr() as *const _));
        if mask.is_invalid() {
            let _ = DeleteObject(color.into());
            let _ = DeleteDC(screen_dc);
            return Err("tray mask bitmap unavailable".to_owned());
        }
        let icon_info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = match CreateIconIndirect(&icon_info) {
            Ok(icon) => icon,
            Err(_) => {
                let _ = DeleteObject(mask.into());
                let _ = DeleteObject(color.into());
                let _ = DeleteDC(screen_dc);
                return Err("tray icon creation failed".to_owned());
            }
        };
        let _ = DeleteObject(mask.into());
        let _ = DeleteObject(color.into());
        let _ = DeleteDC(screen_dc);
        let _ = HWND::default();
        Ok(icon)
    }
}

#[cfg(not(target_os = "windows"))]
fn build_tray_icon() -> Result<(), String> {
    Err("tray is Windows-only".to_owned())
}

/// Worker-owned tray state, shared by reference with the window procedure
/// (single definition — layout drift between setup and teardown is
/// impossible by construction). `hwnd`/`icon` are intentionally never read
/// after setup: holding them keeps the window and icon alive for the
/// worker's lifetime (destroying either early would orphan or blank the
/// tray icon).
#[cfg(target_os = "windows")]
#[allow(dead_code)]
struct ThreadState {
    proxy: tao::event_loop::EventLoopProxy<crate::NativeEvent>,
    menu: windows::Win32::UI::WindowsAndMessaging::HMENU,
    hwnd: windows::Win32::Foundation::HWND,
    icon: windows::Win32::UI::WindowsAndMessaging::HICON,
}

/// Worker body: STA COM, hidden window, icon add, message pump until the
/// main thread drops the handle.
#[cfg(target_os = "windows")]
fn run_tray_worker(
    proxy: tao::event_loop::EventLoopProxy<crate::NativeEvent>,
    art: &'static [u8],
    ready_tx: mpsc::Sender<Result<(), String>>,
    shutdown_rx: mpsc::Receiver<()>,
) {
    use windows::{
        core::{w, PCWSTR},
        Win32::{
            Foundation::HWND,
            System::{
                Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
                LibraryLoader::GetModuleHandleW,
            },
            UI::{
                Shell::{
                    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE,
                    NOTIFYICONDATAW,
                },
                WindowsAndMessaging::{
                    AppendMenuW, CreatePopupMenu, CreateWindowExW, DestroyIcon, DestroyMenu,
                    DestroyWindow, DispatchMessageW, GetWindowLongPtrW, PeekMessageW,
                    RegisterClassW, SetWindowLongPtrW, TranslateMessage, GWLP_USERDATA, HICON,
                    MF_SEPARATOR, MF_STRING, MSG, PM_REMOVE, WM_QUIT, WNDCLASSW, WS_OVERLAPPED,
                },
            },
        },
    };
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let report = |result: Result<(), String>| {
        let _ = ready_tx.send(result);
    };
    // The icon is a raw handle (not Send): build it on this thread from
    // the static PNG bytes before creating any window.
    let icon = match build_tray_icon_from(art) {
        Ok(icon) => icon,
        Err(reason) => {
            report(Err(format!("tray icon unavailable: {reason}")));
            return;
        }
    };
    // Hidden message window (never shown; parked off-screen).
    let class_name = w!("RemcordTrayWorker");
    let instance = unsafe { GetModuleHandleW(None).unwrap_or_default() };
    let registered = unsafe {
        RegisterClassW(&WNDCLASSW {
            style: Default::default(),
            lpfnWndProc: Some(tray_wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance.into(),
            hIcon: HICON::default(),
            hCursor: Default::default(),
            hbrBackground: Default::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: class_name,
        })
    };
    if registered == 0 {
        report(Err("tray window class registration failed".to_owned()));
        return;
    }
    let hwnd = match unsafe {
        CreateWindowExW(
            Default::default(),
            class_name,
            w!("RemcordTrayWorker"),
            WS_OVERLAPPED,
            -32000,
            -32000,
            8,
            8,
            None,
            None,
            Some(instance.into()),
            None,
        )
    } {
        Ok(hwnd) => hwnd,
        Err(_) => {
            report(Err("tray worker window creation failed".to_owned()));
            unsafe {
                let _ = DestroyIcon(icon);
            }
            return;
        }
    };
    let menu = unsafe { CreatePopupMenu().unwrap_or_default() };
    if menu.is_invalid() {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        report(Err("tray menu creation failed".to_owned()));
        return;
    }
    // Release every worker-owned resource exactly once: remove the icon,
    // reclaim the state box, destroy menu/window/icon, uninitialize COM.
    // HICON/HMENU/HWND are Copy handles, so the closure can run on both
    // exit paths without moves.
    let teardown = move |hwnd: HWND| {
        let mut delete = unsafe { std::mem::zeroed::<NOTIFYICONDATAW>() };
        delete.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        delete.hWnd = hwnd;
        delete.uID = 1;
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &delete);
            let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ThreadState;
            if !raw.is_null() {
                drop(Box::from_raw(raw));
            }
            let _ = DestroyMenu(menu);
            let _ = DestroyWindow(hwnd);
            let _ = DestroyIcon(icon);
            CoUninitialize();
        }
    };
    unsafe {
        let _ = AppendMenuW(menu, MF_STRING, MENU_OPEN_ID as usize, w!("&Open Remcord"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, MENU_QUIT_ID as usize, w!("&Quit Remcord"));
    }
    let state = Box::new(ThreadState {
        proxy,
        menu,
        hwnd,
        icon,
    });
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
    }
    // Add the notification icon.
    let mut tip = [0u16; 128];
    for (slot, code) in TRAY_TOOLTIP.encode_utf16().take(tip.len() - 1).enumerate() {
        tip[slot] = code;
    }
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = 1;
    data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    data.uCallbackMessage = WM_TRAY_CALLBACK;
    data.hIcon = icon;
    data.szTip = tip;
    let added = unsafe { Shell_NotifyIconW(NIM_ADD, &data).as_bool() };
    if !added {
        teardown(hwnd);
        report(Err("tray icon registration failed".to_owned()));
        return;
    }
    report(Ok(()));
    // Message pump: tray callbacks/menus here; shutdown is polled between
    // messages so quit never hangs on an idle queue.
    loop {
        if shutdown_rx.try_recv().is_ok() {
            break;
        }
        let mut msg = MSG::default();
        let has_message = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() };
        if has_message {
            if msg.message == WM_QUIT {
                break;
            }
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            continue;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    teardown(hwnd);
}

/// Window procedure for the hidden tray window: icon callbacks and menu
/// commands. State rides in `GWLP_USERDATA` (set once before the icon is
/// added, reclaimed exactly once at teardown).
#[cfg(target_os = "windows")]
unsafe extern "system" fn tray_wnd_proc(
    hwnd: windows::Win32::Foundation::HWND,
    message: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::{
        Foundation::{LRESULT, POINT},
        UI::WindowsAndMessaging::{
            DefWindowProcW, GetCursorPos, GetWindowLongPtrW, SetForegroundWindow, TrackPopupMenu,
            GWLP_USERDATA, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RIGHTBUTTON, WM_COMMAND,
            WM_LBUTTONUP, WM_RBUTTONUP,
        },
    };
    if message == WM_TRAY_CALLBACK {
        let event = lparam.0 as u32;
        let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const ThreadState;
        if raw.is_null() {
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }
        let state = &*raw;
        if event == WM_LBUTTONUP {
            let _ = state.proxy.send_event(crate::NativeEvent::TrayRestore);
        } else if event == WM_RBUTTONUP {
            let mut cursor = POINT::default();
            if GetCursorPos(&mut cursor).is_ok() {
                let _ = SetForegroundWindow(hwnd);
                let _ = TrackPopupMenu(
                    state.menu,
                    TPM_LEFTALIGN | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
                    cursor.x,
                    cursor.y,
                    None,
                    hwnd,
                    None,
                );
            }
        }
        return LRESULT(0);
    }
    if message == WM_COMMAND {
        let id = (wparam.0 & 0xFFFF) as u32;
        let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const ThreadState;
        if !raw.is_null() {
            let state = &*raw;
            if id == MENU_OPEN_ID {
                let _ = state.proxy.send_event(crate::NativeEvent::TrayRestore);
            } else if id == MENU_QUIT_ID {
                let _ = state.proxy.send_event(crate::NativeEvent::TrayQuit);
            }
        }
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, message, wparam, lparam)
}

#[cfg(not(target_os = "windows"))]
fn run_tray_worker(
    _proxy: tao::event_loop::EventLoopProxy<crate::NativeEvent>,
    ready_tx: mpsc::Sender<Result<(), String>>,
    _shutdown_rx: mpsc::Receiver<()>,
) {
    let _ = ready_tx.send(Err("tray is Windows-only".to_owned()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_and_menu_ids_are_fixed() {
        assert_eq!(TRAY_TOOLTIP, "Remcord");
        assert_eq!(MENU_OPEN_ID, 1);
        assert_eq!(MENU_QUIT_ID, 2);
        assert_ne!(MENU_OPEN_ID, MENU_QUIT_ID);
    }

    #[test]
    fn every_tray_setting_resolves_embedded_artwork() {
        // Electron's nine ValidTrayIcons: each names compiled-in bytes
        // (dynamic + default fall back to the rebranded mark); nothing
        // reads the repo or install dir at runtime.
        for mode in [
            "dynamic",
            "dsc-tray",
            "clsc-dsc-tray",
            "ac_plug_colored",
            "ac_white_plug",
            "ac_white_plug_hollow",
            "ac_black_plug",
            "ac_black_plug_hollow",
            "disabled",
        ] {
            assert!(
                !tray_art_bytes(mode).is_empty(),
                "{mode} must resolve artwork"
            );
        }
        assert_eq!(
            tray_art_bytes("dynamic"),
            crate::app_icon::embedded_png_bytes()
        );
        assert_eq!(
            tray_art_bytes("dsc-tray"),
            crate::app_icon::embedded_png_bytes()
        );
        assert_ne!(
            tray_art_bytes("ac_plug_colored"),
            crate::app_icon::embedded_png_bytes()
        );
    }

    #[test]
    fn embedded_png_decodes_for_icon_use() {
        let (width, height, bgra) = tray_bitmap_pixels_from(crate::app_icon::embedded_png_bytes())
            .expect("embedded PNG must decode");
        assert!(width > 0 && height > 0);
        assert_eq!(bgra.len(), width as usize * height as usize * 4);
    }

    /// The built icon is a real GDI icon object: `GetIconInfo` resolves it
    /// and the color bitmap dimensions match the decoded PNG (proves the
    /// DIB + mask construction above produces a well-formed `HICON`
    /// instead of a silently blank tray slot). Windows-only: needs a
    /// desktop session (like the tray itself).
    #[cfg(target_os = "windows")]
    #[test]
    fn built_icon_is_a_well_formed_gdi_icon() {
        use windows::Win32::{
            Graphics::Gdi::{GetObjectW, BITMAP},
            UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo},
        };
        let icon = build_tray_icon_from(crate::app_icon::embedded_png_bytes())
            .expect("tray icon must build");
        let (width, height, _) = tray_bitmap_pixels_from(crate::app_icon::embedded_png_bytes())
            .expect("embedded PNG must decode");
        unsafe {
            let mut icon_info =
                std::mem::zeroed::<windows::Win32::UI::WindowsAndMessaging::ICONINFO>();
            // Rebuild through the same constructor the worker uses, then
            // interrogate the object GDI actually holds.
            let _ = DestroyIcon(icon);
            let rebuilt = build_tray_icon_from(crate::app_icon::embedded_png_bytes())
                .expect("tray icon must rebuild");
            assert!(GetIconInfo(rebuilt, &mut icon_info).is_ok());
            assert!(!icon_info.hbmColor.is_invalid());
            assert!(!icon_info.hbmMask.is_invalid());
            let mut bitmap = BITMAP::default();
            let bytes = GetObjectW(
                icon_info.hbmColor.into(),
                std::mem::size_of::<BITMAP>() as i32,
                Some(&mut bitmap as *mut _ as *mut std::ffi::c_void),
            );
            assert!(bytes > 0, "color bitmap must be queryable");
            assert_eq!(bitmap.bmWidth, width as i32);
            assert_eq!(bitmap.bmHeight, height as i32);
            assert_eq!(bitmap.bmBitsPixel, 32);
            let _ = windows::Win32::Graphics::Gdi::DeleteObject(icon_info.hbmColor.into());
            let _ = windows::Win32::Graphics::Gdi::DeleteObject(icon_info.hbmMask.into());
            let _ = DestroyIcon(rebuilt);
        }
    }
}
