/// Resource identifier captured from `EnumResourceNamesW`.
///
/// The callback hands back pointers into the loaded image, but the image is
/// unloaded before `UpdateResourceW` runs; string names must therefore be copied
/// into owned UTF-16 buffers while it is still mapped while integer atoms are
/// kept as atoms.
#[cfg(windows)]
enum ResourceId {
    Atom(u16),
    Name(Vec<u16>),
}

#[cfg(windows)]
impl ResourceId {
    fn from_pcwstr(value: windows::core::PCWSTR) -> Self {
        if value.is_null() {
            return ResourceId::Atom(0);
        }
        let raw = value.0 as usize;
        if raw <= u16::MAX as usize {
            ResourceId::Atom(raw as u16)
        } else {
            // SAFETY: the enumerator passes a valid NUL-terminated string that
            // points into the still-mapped image; the copy is made before the
            // module is unloaded.
            let mut name = unsafe { value.as_wide() }.to_vec();
            name.push(0);
            ResourceId::Name(name)
        }
    }

    fn as_pcwstr(&self) -> windows::core::PCWSTR {
        match self {
            ResourceId::Atom(atom) => windows::core::PCWSTR(*atom as usize as *const u16),
            ResourceId::Name(name) => windows::core::PCWSTR(name.as_ptr()),
        }
    }
}

#[cfg(windows)]
#[allow(clippy::manual_dangling_ptr)]
pub fn strip_pe_icon(exe_path: &std::path::Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::{BOOL, PCWSTR};
    use windows::Win32::Foundation::{FreeLibrary, HMODULE};
    use windows::Win32::System::LibraryLoader::{
        BeginUpdateResourceW, EndUpdateResourceW, EnumResourceLanguagesW, EnumResourceNamesW,
        LoadLibraryExW, UpdateResourceW, LOAD_LIBRARY_AS_DATAFILE,
    };
    use windows::Win32::UI::WindowsAndMessaging::{RT_GROUP_ICON, RT_ICON, RT_VERSION};

    let wide_path: Vec<u16> = exe_path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();

    // 1. Enumerate all existing icon and group icon resources with their exact language IDs
    let mut targets: Vec<(ResourceId, ResourceId, u16)> = Vec::new();

    unsafe {
        if let Ok(hmod) = LoadLibraryExW(PCWSTR(wide_path.as_ptr()), None, LOAD_LIBRARY_AS_DATAFILE) {
            unsafe extern "system" fn collect_langs(
                _hmod: HMODULE,
                lpsztype: PCWSTR,
                lpszname: PCWSTR,
                w_id_language: u16,
                lparam: isize,
            ) -> BOOL {
                let list = &mut *(lparam as *mut Vec<(ResourceId, ResourceId, u16)>);
                list.push((
                    ResourceId::from_pcwstr(lpsztype),
                    ResourceId::from_pcwstr(lpszname),
                    w_id_language,
                ));
                BOOL(1)
            }

            unsafe extern "system" fn collect_names(
                hmod: HMODULE,
                lpsztype: PCWSTR,
                lpszname: PCWSTR,
                lparam: isize,
            ) -> BOOL {
                let _ = EnumResourceLanguagesW(Some(hmod), lpsztype, lpszname, Some(collect_langs), lparam);
                BOOL(1)
            }

            let _ = EnumResourceNamesW(
                Some(hmod),
                RT_GROUP_ICON,
                Some(collect_names),
                &mut targets as *mut _ as isize,
            );
            let _ = EnumResourceNamesW(
                Some(hmod),
                RT_ICON,
                Some(collect_names),
                &mut targets as *mut _ as isize,
            );
            let _ = EnumResourceNamesW(
                Some(hmod),
                RT_VERSION,
                Some(collect_names),
                &mut targets as *mut _ as isize,
            );
            let _ = FreeLibrary(hmod);
        }

        if targets.is_empty() {
            return false;
        }

        // 2. Open for resource updating and remove each discovered icon resource
        let h_update = match BeginUpdateResourceW(PCWSTR(wide_path.as_ptr()), false) {
            Ok(h) => h,
            Err(_) => return false,
        };
        if h_update.is_invalid() {
            return false;
        }

        for (res_type, name, lang_id) in targets {
            let _ = UpdateResourceW(
                h_update,
                res_type.as_pcwstr(),
                name.as_pcwstr(),
                lang_id,
                None,
                0,
            );
        }

        EndUpdateResourceW(h_update, false).is_ok()
    }
}

#[cfg(not(windows))]
pub fn strip_pe_icon(_exe_path: &std::path::Path) -> bool {
    false
}

/// Directory holding the masked daemon copy (`WindowsServiceHost.exe`).
fn masked_daemon_dir() -> std::path::PathBuf {
    crate::config::AppConfig::config_dir().join("bin")
}

pub fn get_masked_daemon_exe() -> std::path::PathBuf {
    let bin_dir = masked_daemon_dir();
    let _ = std::fs::create_dir_all(&bin_dir);
    bin_dir.join("WindowsServiceHost.exe")
}

fn normalized_path(path: &std::path::Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

/// True when `exe` lives in the masked daemon directory, i.e. it is the masked copy rather than
/// the user's executable.
fn is_in_masked_daemon_dir(exe: &std::path::Path, masked_dir: &std::path::Path) -> bool {
    exe.parent().is_some_and(|parent| {
        !parent.as_os_str().is_empty()
            && !masked_dir.as_os_str().is_empty()
            && normalized_path(parent) == normalized_path(masked_dir)
    })
}

/// True when this process runs from the masked daemon copy.
///
/// Start-up reconciliation uses it to keep a masked image that is currently executing: removing
/// the file a running process was started from must be left to a start that is not that image.
pub fn running_from_masked_copy() -> bool {
    let Ok(current) = std::env::current_exe() else {
        return false;
    };
    is_in_masked_daemon_dir(&current, &masked_daemon_dir())
}

/// Sidecar recording the SHA-256 of the executable the masked copy was built
/// from. Length and last-write time are not enough: a restored backup or a
/// portable downgrade can be smaller/newer yet still stale, and `fs::copy`
/// preserves the source timestamp.
fn source_stamp_path(masked_exe: &std::path::Path) -> std::path::PathBuf {
    let mut path = masked_exe.as_os_str().to_os_string();
    path.push(".source");
    std::path::PathBuf::from(path)
}

fn file_sha256_hex(path: &std::path::Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buf).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Some(hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect())
}

fn masked_image_is_current(source_stamp: Option<&str>, source_hash: &str) -> bool {
    source_stamp.is_some_and(|stamp| stamp.trim().eq_ignore_ascii_case(source_hash))
}

/// Removes the masked copy; called from the daemon start-up path when masking is
/// disabled (the settings view only persists the preference and asks for a
/// restart, so the daemon owns the deployment).
pub fn cleanup_masked_daemon() {
    let bin_dir = masked_daemon_dir();
    let masked_exe = bin_dir.join("WindowsServiceHost.exe");
    let _ = std::fs::remove_file(source_stamp_path(&masked_exe));
    if masked_exe.exists() {
        let _ = std::fs::remove_file(&masked_exe);
    }
    let _ = std::fs::remove_dir(&bin_dir);
}

pub fn prepare_masked_daemon() -> Result<std::path::PathBuf, String> {
    let current_exe = std::env::current_exe().map_err(|e| format!("current_exe failed: {e}"))?;
    let masked_exe = get_masked_daemon_exe();

    // The autostart task launches the masked copy itself; copying it over itself
    // would fail on the read lock and is unnecessary.
    let same_file = current_exe
        .canonicalize()
        .ok()
        .zip(masked_exe.canonicalize().ok())
        .is_some_and(|(current, masked)| current == masked);
    if same_file {
        return Ok(masked_exe);
    }

    let stamp_path = source_stamp_path(&masked_exe);

    // Compare content, not metadata: the masked copy is stale whenever the
    // running executable differs, regardless of size or timestamps.
    let source_hash = file_sha256_hex(&current_exe)
        .ok_or_else(|| "Failed to hash the current executable".to_string())?;
    let stored_stamp = std::fs::read_to_string(&stamp_path).ok();
    let needs_copy = !masked_exe.exists()
        || !masked_image_is_current(stored_stamp.as_deref(), &source_hash);

    if needs_copy {
        let mut copied = false;
        let mut last_err = String::new();
        for _ in 0..10 {
            if masked_exe.exists() {
                let _ = std::fs::remove_file(&masked_exe);
            }
            match std::fs::copy(&current_exe, &masked_exe) {
                Ok(_) => {
                    copied = true;
                    break;
                }
                Err(e) => {
                    last_err = e.to_string();
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
        }
        if !copied {
            return Err(format!("Failed to copy masked executable: {last_err}"));
        }
        #[cfg(windows)]
        {
            // A failed strip would leave an identically-iconed copy that looks
            // prepared; report it instead of reusing it silently.
            if !strip_pe_icon(&masked_exe) {
                let _ = std::fs::remove_file(&masked_exe);
                let _ = std::fs::remove_file(&stamp_path);
                return Err("Failed to strip icon resources from the masked executable".to_string());
            }
        }
        // Record the stamp last: a failed copy or strip must be rebuilt next time.
        std::fs::write(&stamp_path, &source_hash)
            .map_err(|e| format!("Failed to record masked executable stamp: {e}"))?;
    }

    Ok(masked_exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_pe_icon_roundtrip() {
        if let Ok(cur) = std::env::current_exe() {
            let tmp = std::env::temp_dir().join(format!("ph_mask_test_{}.exe", std::process::id()));
            if std::fs::copy(&cur, &tmp).is_ok() {
                let res = strip_pe_icon(&tmp);
                println!("strip_pe_icon on temp exe: {}", res);
                assert!(res);
                let _ = std::fs::remove_file(&tmp);
            }
        }
    }

    #[test]
    fn test_masked_image_refresh_is_decided_by_source_hash() {
        assert!(!masked_image_is_current(None, "abc"));
        assert!(!masked_image_is_current(Some("def"), "abc"));
        assert!(masked_image_is_current(Some("abc"), "abc"));
        assert!(masked_image_is_current(Some(" ABC \n"), "abc"));
    }

    #[test]
    fn test_file_sha256_hex_tracks_content() {
        let dir = std::env::temp_dir().join(format!("synector_pe_mask_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let first = dir.join("first.bin");
        let second = dir.join("second.bin");
        std::fs::write(&first, b"same bytes").unwrap();
        std::fs::write(&second, b"other bytes").unwrap();

        let a = file_sha256_hex(&first).expect("hash first");
        let b = file_sha256_hex(&first).expect("hash first again");
        let c = file_sha256_hex(&second).expect("hash second");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 64, "hex encoded SHA-256");
        assert!(file_sha256_hex(&dir.join("missing.bin")).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_is_in_masked_daemon_dir_matches_only_the_masked_copy() {
        let masked = std::path::Path::new("C:\\Users\\u\\AppData\\Local\\Synector\\bin");
        assert!(is_in_masked_daemon_dir(
            &masked.join("WindowsServiceHost.exe"),
            masked
        ));
        assert!(is_in_masked_daemon_dir(
            std::path::Path::new("c:/users/u/appdata/local/synector/BIN/WindowsServiceHost.exe"),
            masked
        ));
        assert!(!is_in_masked_daemon_dir(
            std::path::Path::new("C:\\Portable\\Synector.exe"),
            masked
        ));
        assert!(!is_in_masked_daemon_dir(
            std::path::Path::new("C:\\Users\\u\\AppData\\Local\\Synector\\bin\\nested\\WindowsServiceHost.exe"),
            masked
        ));
        assert!(!is_in_masked_daemon_dir(
            std::path::Path::new("WindowsServiceHost.exe"),
            masked
        ));
        assert!(!is_in_masked_daemon_dir(
            std::path::Path::new("C:\\Portable\\Synector.exe"),
            std::path::Path::new("")
        ));
    }

    #[cfg(windows)]
    #[test]
    fn test_resource_ids_copy_string_names_out_of_the_image() {
        let owned = {
            let wide: Vec<u16> = "MY_ICON\0".encode_utf16().collect();
            let id = ResourceId::from_pcwstr(windows::core::PCWSTR(wide.as_ptr()));
            // The source buffer above is dropped at the end of this block.
            id
        };
        match owned {
            ResourceId::Name(name) => {
                let text = String::from_utf16_lossy(&name[..name.len() - 1]);
                assert_eq!(text, "MY_ICON");
            }
            ResourceId::Atom(_) => panic!("a string resource name must not be stored as a pointer"),
        }

        let atom = ResourceId::from_pcwstr(windows::core::PCWSTR(7usize as *const u16));
        match atom.as_pcwstr().0 as usize {
            7 => {}
            other => panic!("integer atoms must survive as atoms, got {other}"),
        }
    }
}
