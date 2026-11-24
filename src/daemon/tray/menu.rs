#[cfg(windows)]
use std::sync::OnceLock;

#[cfg(windows)]
use windows::core::{w, PWSTR};
#[cfg(windows)]
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::DeleteObject;
#[cfg(windows)]
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, GetCursorPos, GetSystemMetrics, InsertMenuItemW, PostMessageW,
    SetForegroundWindow, SetMenuDefaultItem, TrackPopupMenu, MENUITEMINFOW, MFS_DISABLED, MFT_SEPARATOR,
    MIIM_BITMAP, MIIM_FTYPE, MIIM_ID, MIIM_STATE, MIIM_STRING, SM_CXSMICON, SM_CYSMICON, TPM_BOTTOMALIGN,
    TPM_LEFTALIGN, TPM_RIGHTBUTTON, WM_NULL,
};

use super::{generate_exit_menu_bitmap, generate_open_menu_bitmap, generate_update_menu_bitmap};
use super::update::{get_available_update, is_updating};

pub(super) const IDM_OPEN: usize = 1001;
pub(super) const IDM_UPDATE: usize = 1002;
pub(super) const IDM_EXIT: usize = 1003;

#[cfg(windows)]
pub(super) unsafe fn show_tray_menu(hwnd: HWND) {
    let config = crate::config::Settings::read();
    let is_dark = config.effective_theme() != crate::config::AppTheme::Light;
    apply_menu_theme(Some(hwnd), is_dark);

    let loc = crate::i18n::get(config.language);
    let mut open_w: Vec<u16> = loc.tray_menu_open.encode_utf16().chain(std::iter::once(0)).collect();
    let mut exit_w: Vec<u16> = loc.tray_menu_exit.encode_utf16().chain(std::iter::once(0)).collect();

    let available_update = get_available_update();
    let icon_size = GetSystemMetrics(SM_CXSMICON)
        .max(GetSystemMetrics(SM_CYSMICON))
        .max(16) as u32;
    let hbmp_open = generate_open_menu_bitmap(icon_size, is_dark);
    let hbmp_update = if available_update.is_some() {
        generate_update_menu_bitmap(icon_size, is_dark)
    } else {
        None
    };
    let hbmp_exit = generate_exit_menu_bitmap(icon_size, is_dark);

    if let Ok(hmenu) = CreatePopupMenu() {
        let mut menu_idx = 0;

        // 1. Open Synector (Bold default item with app window / dashboard icon)
        let mut mask_open = MIIM_ID | MIIM_STRING;
        if hbmp_open.is_some() {
            mask_open |= MIIM_BITMAP;
        }
        let item_open = MENUITEMINFOW {
            cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
            fMask: mask_open,
            wID: IDM_OPEN as u32,
            dwTypeData: PWSTR(open_w.as_mut_ptr()),
            cch: open_w.len() as u32,
            hbmpItem: hbmp_open.unwrap_or_default(),
            ..Default::default()
        };
        let _ = InsertMenuItemW(hmenu, menu_idx, true, &item_open);
        let _ = SetMenuDefaultItem(hmenu, IDM_OPEN as u32, 0);
        menu_idx += 1;

        // 2. Update to vX.X.X / Updating... (If available or updating, placed directly under Open)
        let mut update_w: Option<Vec<u16>> = None;
        let is_upd = is_updating();
        if is_upd || available_update.is_some() {
            let update_label = if is_upd {
                loc.tray_menu_updating.to_string()
            } else if let Some(ref info) = available_update {
                loc.tray_menu_update.replace("{}", &info.version)
            } else {
                loc.tray_menu_updating.to_string()
            };
            let mut w_str: Vec<u16> = update_label.encode_utf16().chain(std::iter::once(0)).collect();
            let mut mask_upd = MIIM_ID | MIIM_STRING;
            if is_upd {
                mask_upd |= MIIM_STATE;
            }
            if hbmp_update.is_some() {
                mask_upd |= MIIM_BITMAP;
            }
            let item_update = MENUITEMINFOW {
                cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                fMask: mask_upd,
                fState: if is_upd { MFS_DISABLED } else { Default::default() },
                wID: IDM_UPDATE as u32,
                dwTypeData: PWSTR(w_str.as_mut_ptr()),
                cch: w_str.len() as u32,
                hbmpItem: hbmp_update.unwrap_or_default(),
                ..Default::default()
            };
            let _ = InsertMenuItemW(hmenu, menu_idx, true, &item_update);
            update_w = Some(w_str);
            menu_idx += 1;
        }

        // 3. Separator
        let sep = MENUITEMINFOW {
            cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_FTYPE,
            fType: MFT_SEPARATOR,
            ..Default::default()
        };
        let _ = InsertMenuItemW(hmenu, menu_idx, true, &sep);
        menu_idx += 1;

        // 4. Exit (with power/exit icon)
        let mut mask_exit = MIIM_ID | MIIM_STRING;
        if hbmp_exit.is_some() {
            mask_exit |= MIIM_BITMAP;
        }
        let item_exit = MENUITEMINFOW {
            cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
            fMask: mask_exit,
            wID: IDM_EXIT as u32,
            dwTypeData: PWSTR(exit_w.as_mut_ptr()),
            cch: exit_w.len() as u32,
            hbmpItem: hbmp_exit.unwrap_or_default(),
            ..Default::default()
        };
        let _ = InsertMenuItemW(hmenu, menu_idx, true, &item_exit);

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(
            hmenu,
            TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RIGHTBUTTON,
            pt.x,
            pt.y,
            None,
            hwnd,
            None,
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(hmenu);
        drop(update_w);
    }

    if let Some(bmp) = hbmp_open {
        let _ = DeleteObject(bmp.into());
    }
    if let Some(bmp) = hbmp_update {
        let _ = DeleteObject(bmp.into());
    }
    if let Some(bmp) = hbmp_exit {
        let _ = DeleteObject(bmp.into());
    }
}

#[cfg(windows)]
struct UxThemeFns {
    // uxtheme.dll ordinals:
    // 135: SetPreferredAppMode (0 = Default, 1 = AllowDark, 2 = ForceDark, 3 = ForceLight)
    // 136: FlushMenuThemes
    // 133: AllowDarkModeForWindow
    set_preferred_app_mode: Option<unsafe extern "system" fn(i32) -> i32>,
    flush_menu_themes: Option<unsafe extern "system" fn()>,
    allow_dark_for_window: Option<unsafe extern "system" fn(HWND, bool) -> bool>,
}

/// Ordinals are resolved once; the module stays loaded for the process lifetime so the resolved
/// addresses remain valid and repeated menu opens do not raise its reference count.
#[cfg(windows)]
fn uxtheme_fns() -> &'static UxThemeFns {
    static FUNCS: OnceLock<UxThemeFns> = OnceLock::new();
    FUNCS.get_or_init(|| unsafe {
        let mut fns = UxThemeFns {
            set_preferred_app_mode: None,
            flush_menu_themes: None,
            allow_dark_for_window: None,
        };
        if let Ok(uxtheme) = LoadLibraryW(w!("uxtheme.dll")) {
            fns.set_preferred_app_mode = std::mem::transmute::<
                Option<unsafe extern "system" fn() -> isize>,
                Option<unsafe extern "system" fn(i32) -> i32>,
            >(GetProcAddress(uxtheme, windows::core::PCSTR(135 as _)));
            fns.flush_menu_themes = std::mem::transmute::<
                Option<unsafe extern "system" fn() -> isize>,
                Option<unsafe extern "system" fn()>,
            >(GetProcAddress(uxtheme, windows::core::PCSTR(136 as _)));
            fns.allow_dark_for_window = std::mem::transmute::<
                Option<unsafe extern "system" fn() -> isize>,
                Option<unsafe extern "system" fn(HWND, bool) -> bool>,
            >(GetProcAddress(uxtheme, windows::core::PCSTR(133 as _)));
        }
        fns
    })
}

#[cfg(windows)]
pub fn apply_menu_theme(hwnd: Option<HWND>, is_dark: bool) {
    let fns = uxtheme_fns();
    let mode = if is_dark { 2 } else { 3 };
    unsafe {
        if let Some(set_mode) = fns.set_preferred_app_mode {
            set_mode(mode);
        }

        if let Some(flush) = fns.flush_menu_themes {
            flush();
        }

        if let (Some(h), Some(allow_wnd)) = (hwnd, fns.allow_dark_for_window) {
            let _ = allow_wnd(h, is_dark);
        }
    }
}
