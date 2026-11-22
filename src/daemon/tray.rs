mod bitmap;
mod icon;
mod menu;
mod update;

pub use bitmap::*;
pub use icon::*;
pub use menu::*;
pub use update::*;

#[cfg(all(test, windows))]
use windows::core::PCWSTR;
#[cfg(all(test, windows))]
use windows::Win32::Foundation::HWND;
#[cfg(all(test, windows))]
use windows::Win32::UI::Shell::{ExtractIconExW, NOTIFYICONDATAW};
#[cfg(all(test, windows))]
use windows::Win32::UI::WindowsAndMessaging::{HICON, IDI_APPLICATION};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn test_build_tray_icon_data_flags() {
        use windows::Win32::UI::Shell::{NIF_ICON, NIF_MESSAGE, NIF_TIP};

        let hwnd = HWND(0x1234 as *mut core::ffi::c_void);
        let icon = HICON(0x5678 as *mut core::ffi::c_void);
        let nid = build_tray_icon_data(hwnd, false, icon);
        assert_eq!(nid.cbSize, std::mem::size_of::<NOTIFYICONDATAW>() as u32);
        assert_eq!(nid.hWnd, hwnd);
        assert_eq!(nid.uID, 1);
        assert_eq!(nid.uCallbackMessage, WM_TRAYICON);
        assert_ne!(nid.uFlags.0 & NIF_ICON.0, 0);
        assert_ne!(nid.uFlags.0 & NIF_MESSAGE.0, 0);
        assert_ne!(nid.uFlags.0 & NIF_TIP.0, 0);
        assert_eq!(nid.hIcon, icon);
        assert_eq!(
            String::from_utf16_lossy(&nid.szTip[.."Synector".len()]),
            "Synector"
        );

        let masked = build_tray_icon_data(hwnd, true, icon);
        assert_eq!(
            String::from_utf16_lossy(&masked.szTip[.."Windows Service Host".len()]),
            "Windows Service Host"
        );
    }

    #[test]
    #[cfg(windows)]
    fn test_tray_icon_ownership_wrapper() {
        use windows::Win32::UI::WindowsAndMessaging::LoadIconW;

        unsafe {
            let shared = TrayIconHandle::shared(LoadIconW(None, IDI_APPLICATION).unwrap_or_default());
            assert!(!shared.is_owned());
            drop(shared);

            let system_root =
                std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
            let mut shell32 = std::path::PathBuf::from(system_root);
            shell32.push("System32");
            shell32.push("shell32.dll");
            let path_w: Vec<u16> = shell32
                .to_string_lossy()
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let mut hicon = HICON::default();
            let count = ExtractIconExW(PCWSTR(path_w.as_ptr()), 0, None, Some(&mut hicon), 1);
            if count > 0 && !hicon.is_invalid() {
                let owned = TrayIconHandle::owned(hicon);
                assert!(owned.is_owned());
            }
        }
    }

    #[test]
    #[cfg(windows)]
    fn test_tray_menu_bitmaps_creation_and_cleanup() {
        for is_dark in [true, false] {
            for size in [16, 20, 24, 32] {
                let open_bmp = generate_open_menu_bitmap(size, is_dark);
                assert!(open_bmp.is_some(), "Open menu bitmap should be generated successfully for size {size}, dark={is_dark}");
                if let Some(bmp) = open_bmp {
                    unsafe {
                        let _ = windows::Win32::Graphics::Gdi::DeleteObject(bmp.into());
                    }
                }

                let update_bmp = generate_update_menu_bitmap(size, is_dark);
                assert!(update_bmp.is_some(), "Update menu bitmap should be generated successfully for size {size}, dark={is_dark}");
                if let Some(bmp) = update_bmp {
                    unsafe {
                        let _ = windows::Win32::Graphics::Gdi::DeleteObject(bmp.into());
                    }
                }

                let exit_bmp = generate_exit_menu_bitmap(size, is_dark);
                assert!(exit_bmp.is_some(), "Exit menu bitmap should be generated successfully for size {size}, dark={is_dark}");
                if let Some(bmp) = exit_bmp {
                    unsafe {
                        let _ = windows::Win32::Graphics::Gdi::DeleteObject(bmp.into());
                    }
                }
            }
        }
    }

    #[test]
    fn test_tray_available_update_state() {
        set_available_update(None);
        assert!(get_available_update().is_none());

        let release = crate::updater::ReleaseInfo {
            version: "3.0.0".to_string(),
            tag_name: "v3.0.0".to_string(),
            name: "v3.0.0".to_string(),
            body: "".to_string(),
            html_url: "".to_string(),
            portable: None,
            installer: None,
        };

        set_available_update(Some(release.clone()));
        assert_eq!(get_available_update(), Some(release));

        set_available_update(None);
        assert!(get_available_update().is_none());
    }

    #[test]
    fn test_tray_updating_state() {
        set_updating(false, None);
        assert!(!is_updating());
        assert_eq!(get_update_progress_pct(), 0);

        set_updating(true, Some(0.42));
        assert!(is_updating());
        assert_eq!(get_update_progress_pct(), 42);

        set_updating(false, None);
        assert!(!is_updating());
    }
}
