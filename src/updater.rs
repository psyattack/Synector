mod apply;
mod install;
mod install_mode;
mod portable;
mod release;
mod temp;
mod util;

pub use apply::*;
pub use install::*;
pub use install_mode::*;
pub use portable::*;
pub use release::*;
pub use temp::*;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use super::util::escape_ps_literal;

    const ZIP_SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const SETUP_SHA: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

    fn release_json() -> String {
        format!(
            r###"{{
            "tag_name": "v2.9.0",
            "name": "Synector v2.9.0",
            "body": "## What's Changed\n* Added installer",
            "html_url": "https://github.com/psyattack/Synector/releases/tag/v2.9.0",
            "assets": [
                {{
                    "name": "checksums.txt",
                    "browser_download_url": "https://github.com/psyattack/Synector/releases/download/v2.9.0/checksums.txt",
                    "size": 128
                }},
                {{
                    "name": "Synector-v2.9.0-Windows-x64.zip",
                    "browser_download_url": "https://github.com/psyattack/Synector/releases/download/v2.9.0/Synector-v2.9.0-Windows-x64.zip",
                    "size": 15728640,
                    "digest": "sha256:{ZIP_SHA}"
                }},
                {{
                    "name": "Synector-Setup-2.9.0-Windows-x64.exe",
                    "browser_download_url": "https://github.com/psyattack/Synector/releases/download/v2.9.0/Synector-Setup-2.9.0-Windows-x64.exe",
                    "size": 16777216,
                    "digest": "sha256:{SETUP_SHA}"
                }}
            ]
        }}"###
        )
    }

    #[test]
    fn test_clean_temp_update_files_no_panic() {
        clean_temp_update_files();
    }

    /// The `.old` gate is unconditional and independent of daemon liveness: the file next to the
    /// executable is removed, while the executable itself and unrelated suffixes stay untouched.
    #[test]
    fn test_remove_old_executable_beside_the_running_binary() {
        let dir = std::env::temp_dir().join(format!(
            "synector-old-exe-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");

        let exe = dir.join("Synector.exe");
        std::fs::write(&exe, b"current").expect("write executable");
        let old = exe.with_extension("old");
        std::fs::write(&old, b"previous").expect("write stale executable");
        let unrelated = dir.join("Synector.old.txt");
        std::fs::write(&unrelated, b"keep").expect("write unrelated file");

        remove_old_executable_beside(&exe);

        assert!(!old.exists(), "the stale .old executable must be removed");
        assert!(exe.exists(), "the current executable must stay untouched");
        assert!(unrelated.exists(), "unrelated suffixes must stay untouched");

        // A second run without a stale file is a no-op.
        remove_old_executable_beside(&exe);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_version_parsing() {
        assert_eq!(parse_version("1.0.0"), Some((1, 0, 0)));
        assert_eq!(parse_version("v1.0.0"), Some((1, 0, 0)));
        assert_eq!(parse_version("V2.4.9"), Some((2, 4, 9)));
        assert_eq!(parse_version("1.2"), Some((1, 2, 0)));
        assert_eq!(parse_version("v1.1.0-beta.1"), Some((1, 1, 0)));
        assert_eq!(parse_version("v1.3.0+build.42"), Some((1, 3, 0)));
    }

    #[test]
    fn test_is_newer_version() {
        assert!(is_newer_version("1.0.0", "1.0.1"));
        assert!(is_newer_version("1.0.0", "v1.1.0"));
        assert!(is_newer_version("1.0.0", "v2.0.0"));
        assert!(is_newer_version("1.2.3", "v1.3.0"));
        assert!(is_newer_version("1.2.3", "v1.2.4"));

        assert!(!is_newer_version("1.0.0", "1.0.0"));
        assert!(!is_newer_version("1.0.0", "v1.0.0"));
        assert!(!is_newer_version("1.1.0", "v1.0.9"));
        assert!(!is_newer_version("2.0.0", "v1.9.9"));
        assert!(!is_newer_version("1.0.5", "0.9.9"));
    }

    #[test]
    fn test_github_release_deserialization_and_asset_selection() {
        let release: GhRelease = serde_json::from_str(&release_json()).expect("failed to deserialize release");
        let info = release_from_gh(release, "2.8.0")
            .expect("newer release should parse")
            .expect("release should be newer than 2.8.0");

        assert_eq!(info.version, "2.9.0");
        assert_eq!(info.tag_name, "v2.9.0");

        let portable = info.portable.as_ref().expect("portable asset");
        assert_eq!(portable.name, "Synector-v2.9.0-Windows-x64.zip");
        assert_eq!(portable.size, 15_728_640);
        assert_eq!(portable.sha256.as_deref(), Some(ZIP_SHA));

        let installer = info.installer.as_ref().expect("installer asset");
        assert_eq!(installer.name, "Synector-Setup-2.9.0-Windows-x64.exe");
        assert_eq!(installer.size, 16_777_216);
        assert_eq!(installer.sha256.as_deref(), Some(SETUP_SHA));

        assert_eq!(
            info.asset_for_mode(InstallMode::Portable).map(|a| a.name.as_str()),
            Some("Synector-v2.9.0-Windows-x64.zip")
        );
        assert_eq!(
            info.asset_for_mode(InstallMode::Installed).map(|a| a.name.as_str()),
            Some("Synector-Setup-2.9.0-Windows-x64.exe")
        );

        let same_release: GhRelease = serde_json::from_str(&release_json()).unwrap();
        assert!(
            release_from_gh(same_release, "2.9.0").unwrap().is_none(),
            "an equal version must not be offered as an update"
        );
    }

    #[test]
    fn test_github_release_without_update_assets_is_rejected() {
        let json = r#"{
            "tag_name": "v3.0.0",
            "name": "Synector v3.0.0",
            "body": "",
            "html_url": "https://github.com/psyattack/Synector/releases/tag/v3.0.0",
            "assets": [
                {"name": "checksums.txt", "browser_download_url": "https://example.com/checksums.txt", "size": 10}
            ]
        }"#;
        let release: GhRelease = serde_json::from_str(json).unwrap();
        assert!(release_from_gh(release, "2.9.0").is_err());
    }

    #[test]
    fn test_parse_sha256_digest() {
        assert_eq!(
            parse_sha256_digest(&format!("sha256:{}", ZIP_SHA.to_uppercase())),
            Some(ZIP_SHA.to_string())
        );
        assert_eq!(parse_sha256_digest(&format!("  SHA256:{ZIP_SHA}  ")), Some(ZIP_SHA.to_string()));
        assert_eq!(parse_sha256_digest("sha256:abc"), None);
        assert_eq!(parse_sha256_digest("md5:0123"), None);
        assert_eq!(parse_sha256_digest(""), None);
    }

    #[test]
    fn test_hex_encode() {
        assert_eq!(hex_encode(&[0x00, 0x0f, 0xa5, 0xff]), "000fa5ff");
        assert_eq!(hex_encode(&[]), "");
    }

    #[test]
    fn test_resolve_install_mode() {
        let masked_dir = Path::new("C:\\Users\\u\\AppData\\Local\\Synector\\bin");
        let program_files = Path::new("C:\\Program Files\\Synector");

        assert_eq!(
            resolve_install_mode(
                &program_files.join("Synector.exe"),
                Some(program_files),
                masked_dir
            ),
            InstallMode::Installed
        );
        assert_eq!(
            resolve_install_mode(
                Path::new("d:\\apps\\synector\\SYNECTOR.EXE"),
                Some(Path::new("D:\\Apps\\Synector\\")),
                masked_dir
            ),
            InstallMode::Installed,
            "paths compare case-insensitively and ignore a trailing separator"
        );
        assert_eq!(
            resolve_install_mode(
                Path::new("D:\\Tools\\Portable\\Synector.exe"),
                Some(program_files),
                masked_dir
            ),
            InstallMode::Portable
        );
        assert_eq!(
            resolve_install_mode(Path::new("D:\\Tools\\Synector.exe"), None, masked_dir),
            InstallMode::Portable,
            "no installer registration means a portable copy"
        );

        // The masked daemon of an installed application is recognized by the copied executable
        // it runs next to the user configuration.
        let temp = std::env::temp_dir().join(format!("synector_mode_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();
        std::fs::write(temp.join("Synector.exe"), b"MZ").unwrap();
        assert_eq!(
            resolve_install_mode(
                &masked_dir.join("WindowsServiceHost.exe"),
                Some(&temp),
                masked_dir
            ),
            InstallMode::Installed
        );
        assert_eq!(
            resolve_install_mode(
                &masked_dir.join("WindowsServiceHost.exe"),
                Some(&temp.join("missing")),
                masked_dir
            ),
            InstallMode::Portable
        );
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_is_zip_archive() {
        let temp = std::env::temp_dir().join(format!("synector_zip_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();

        let zip = temp.join("archive.zip");
        std::fs::write(&zip, b"PK\x03\x04rest").unwrap();
        assert!(is_zip_archive(&zip));

        let plain = temp.join("plain.bin");
        std::fs::write(&plain, b"MZbinary").unwrap();
        assert!(!is_zip_archive(&plain));
        assert!(!is_zip_archive(&temp.join("missing.zip")));

        let _ = std::fs::remove_dir_all(&temp);
    }

    /// Writes a zip archive with the given `(file name, contents)` entries.
    fn write_zip(zip_path: &Path, files: &[(&str, &[u8])]) {
        write_zip_with_options(zip_path, files, zip::write::SimpleFileOptions::default());
    }

    fn write_zip_with_options(
        zip_path: &Path,
        files: &[(&str, &[u8])],
        options: zip::write::SimpleFileOptions,
    ) {
        use std::io::Write as _;
        let mut writer = zip::ZipWriter::new(std::fs::File::create(zip_path).unwrap());
        for (name, contents) in files {
            writer.start_file(*name, options).unwrap();
            writer.write_all(contents).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn test_extract_exe_from_zip() {
        let temp_dir = std::env::temp_dir().join(format!("synector_test_zip_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let zip_path = temp_dir.join("test_archive.zip");
        write_zip(&zip_path, &[("Synector.exe", b"MZ_MOCK_EXECUTABLE_BINARY_DATA")]);

        let extracted_target = temp_dir.join("Extracted_Synector.exe");
        let result = extract_exe_from_zip(&zip_path, &extracted_target);
        assert!(result.is_ok(), "Extraction failed: {:?}", result.err());
        assert!(extracted_target.exists());
        let extracted_data = std::fs::read(&extracted_target).unwrap();
        assert_eq!(extracted_data, b"MZ_MOCK_EXECUTABLE_BINARY_DATA");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_extract_rejects_archive_without_synector_exe() {
        let temp_dir = std::env::temp_dir().join(format!("synector_test_no_exe_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let zip_path = temp_dir.join("no_synector.zip");
        write_zip(&zip_path, &[("helper.exe", b"MZ_NOT_SYNECTOR"), ("readme.txt", b"notes")]);

        let extracted_target = temp_dir.join("Extracted.exe");
        let err = extract_exe_from_zip(&zip_path, &extracted_target)
            .expect_err("an archive without Synector.exe must be rejected");
        assert!(err.contains("Synector.exe"), "unexpected error: {err}");
        assert!(!extracted_target.exists(), "an arbitrary .exe must never be staged");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_extract_synector_exe_from_subfolder() {
        let temp_dir = std::env::temp_dir().join(format!("synector_test_zip_nested_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let zip_path = temp_dir.join("nested.zip");
        write_zip(&zip_path, &[("folder/Synector.exe", b"MZ_NESTED_EXECUTABLE")]);

        let target = temp_dir.join("Nested.exe");
        extract_exe_from_zip(&zip_path, &target).expect("a nested Synector.exe must be accepted");
        assert_eq!(std::fs::read(&target).unwrap(), b"MZ_NESTED_EXECUTABLE");
        assert!(!target.with_extension("extracting").exists());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_extract_accepts_lowercase_synector_exe() {
        let temp_dir = std::env::temp_dir().join(format!("synector_test_zip_lower_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let zip_path = temp_dir.join("lowercase.zip");
        write_zip(&zip_path, &[("synector.exe", b"MZ_LOWERCASE_EXECUTABLE")]);

        let target = temp_dir.join("Lowercase.exe");
        extract_exe_from_zip(&zip_path, &target).expect("the file name match must be case-insensitive");
        assert_eq!(std::fs::read(&target).unwrap(), b"MZ_LOWERCASE_EXECUTABLE");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_extract_replaces_a_stale_staged_executable() {
        let temp_dir = std::env::temp_dir().join(format!("synector_test_zip_replace_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let zip_path = temp_dir.join("fresh.zip");
        write_zip(&zip_path, &[("Synector.exe", b"MZ_FRESH_EXECUTABLE")]);

        let target = temp_dir.join("Staged.exe");
        std::fs::write(&target, b"STALE_PENDING_EXECUTABLE").unwrap();
        extract_exe_from_zip(&zip_path, &target)
            .expect("a stale staged executable must be replaced");
        assert_eq!(std::fs::read(&target).unwrap(), b"MZ_FRESH_EXECUTABLE");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_extract_rejects_duplicate_synector_exe() {
        let temp_dir = std::env::temp_dir().join(format!("synector_test_zip_dup_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let zip_path = temp_dir.join("duplicate.zip");
        write_zip(
            &zip_path,
            &[("Synector.exe", b"MZ_FIRST"), ("folder/synector.exe", b"MZ_SECOND")],
        );

        let target = temp_dir.join("Duplicate.exe");
        let err = extract_exe_from_zip(&zip_path, &target)
            .expect_err("an archive with two Synector.exe entries must be rejected");
        assert!(err.contains("more than one Synector.exe"), "unexpected error: {err}");
        assert!(!target.exists(), "an ambiguous archive must not stage anything");
        assert!(!target.with_extension("extracting").exists());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_extract_rejects_unsafe_entry_path() {
        let temp_dir = std::env::temp_dir().join(format!("synector_test_zip_unsafe_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let zip_path = temp_dir.join("unsafe.zip");
        write_zip(&zip_path, &[("../Synector.exe", b"MZ_ESCAPE")]);

        let target = temp_dir.join("Unsafe.exe");
        let err = extract_exe_from_zip(&zip_path, &target)
            .expect_err("an archive entry outside the archive root must be rejected");
        assert!(err.contains("unsafe path"), "unexpected error: {err}");
        assert!(!target.exists());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_extract_rejects_executable_above_limit() {
        let temp_dir = std::env::temp_dir().join(format!("synector_test_zip_limit_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let zip_path = temp_dir.join("too_large.zip");
        write_zip(&zip_path, &[("Synector.exe", b"MZ_FAR_TOO_LARGE_FOR_THE_LIMIT")]);

        let target = temp_dir.join("TooLarge.exe");
        let err = super::portable::extract_exe_with_limit(&zip_path, &target, 16)
            .expect_err("an executable above the limit must be rejected");
        assert!(err.contains("limit is 16"), "unexpected error: {err}");
        assert!(!target.exists());
        assert!(!target.with_extension("extracting").exists());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_extract_rejects_corrupted_entry_checksum() {
        let temp_dir = std::env::temp_dir().join(format!("synector_test_zip_crc_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let zip_path = temp_dir.join("corrupt.zip");
        let payload = b"MZ_CORRUPTED_CRC_PAYLOAD_0123456789";
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        write_zip_with_options(&zip_path, &[("Synector.exe", payload)], stored);

        // Corrupt one payload byte while keeping the sizes intact, so the reader reaches EOF and
        // the checksum check is what fails.
        let mut bytes = std::fs::read(&zip_path).unwrap();
        let position = bytes
            .windows(payload.len())
            .position(|window| window == payload)
            .expect("the stored payload must be present in the archive");
        bytes[position + 4] ^= 0xff;
        std::fs::write(&zip_path, bytes).unwrap();

        let target = temp_dir.join("Corrupted.exe");
        let err = extract_exe_from_zip(&zip_path, &target)
            .expect_err("an entry with a broken checksum must be rejected");
        assert!(!err.is_empty());
        assert!(!target.exists());
        assert!(!target.with_extension("extracting").exists());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_missing_asset_digest_is_rejected() {
        let err = verify_asset_digest("Synector-v2.9.0-Windows-x64.zip", None, ZIP_SHA)
            .expect_err("an asset without a digest must not be applied");
        assert!(
            err.contains("no published SHA-256 digest"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_asset_digest_is_verified_case_insensitively() {
        assert!(verify_asset_digest("asset", Some(ZIP_SHA), &ZIP_SHA.to_ascii_uppercase()).is_ok());
        assert!(verify_asset_digest("asset", Some(ZIP_SHA), SETUP_SHA).is_err());
    }

    #[test]
    fn test_escape_ps_literal() {
        assert_eq!(
            escape_ps_literal(r"C:\Users\O'Brien\Synector.exe"),
            r"C:\Users\O''Brien\Synector.exe"
        );
        assert_eq!(
            escape_ps_literal(r"C:\Synector\Synector.exe"),
            r"C:\Synector\Synector.exe"
        );
        assert_eq!(escape_ps_literal("'"), "''");
        assert_eq!(escape_ps_literal("it's 'quoted'"), "it''s ''quoted''");
    }

    #[test]
    fn test_is_masked_daemon_exe() {
        let masked_dir = Path::new("C:\\Users\\u\\AppData\\Local\\Synector\\bin");
        assert!(is_masked_daemon_exe(
            &masked_dir.join("WindowsServiceHost.exe"),
            masked_dir
        ));
        assert!(is_masked_daemon_exe(
            Path::new("C:\\Users\\u\\AppData\\Local\\Synector\\Bin\\WindowsServiceHost.exe"),
            masked_dir
        ));
        assert!(!is_masked_daemon_exe(
            Path::new("D:\\Tools\\Portable\\Synector.exe"),
            masked_dir
        ));
    }

    #[test]
    fn test_portable_update_target_prefers_the_recorded_gui_executable() {
        let dir = std::env::temp_dir().join(format!("synector_update_target_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let recorded = dir.join("Synector.exe");
        let running_gui = dir.join("RunningSynector.exe");
        std::fs::write(&recorded, b"MZ").unwrap();

        // The path the GUI handed to the daemon wins, and the window lookup is not even consulted:
        // it is the only source that still works after the GUI window has closed.
        assert_eq!(
            resolve_portable_target_from(Some(recorded.clone()), || {
                panic!("a valid recorded executable must not need the GUI-window lookup")
            }),
            Some(recorded.clone())
        );

        // A recorded path that no longer exists must fall back instead of staging onto a stale file.
        let stale = dir.join("gone.exe");
        assert_eq!(
            resolve_portable_target_from(Some(stale), || Some(running_gui.clone())),
            Some(running_gui.clone())
        );
        assert_eq!(
            resolve_portable_target_from(None, || Some(running_gui.clone())),
            Some(running_gui.clone())
        );
        assert_eq!(resolve_portable_target_from(None, || None), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_update_temp_entry_staleness() {
        let temp = std::env::temp_dir().join(format!("synector_stale_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();
        let entry = temp.join("entry.zip");
        std::fs::write(&entry, b"x").unwrap();

        let now = std::time::SystemTime::now();
        assert!(
            !update_temp_entry_is_stale(&entry, now),
            "a fresh entry may belong to a live instance"
        );

        let file = std::fs::OpenOptions::new().write(true).open(&entry).unwrap();
        file.set_modified(now - STALE_UPDATE_TEMP_AGE - std::time::Duration::from_secs(60))
            .unwrap();
        drop(file);
        assert!(
            update_temp_entry_is_stale(&entry, now),
            "an old entry is a leftover"
        );
        assert!(!update_temp_entry_is_stale(&temp.join("missing"), now));

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_update_temp_entry_ownership() {
        let temp = std::env::temp_dir();
        let own_dir = temp.join(format!("{UPDATE_TEMP_PREFIX}{}", std::process::id()));
        let own_log = temp.join(format!("{UPDATE_TEMP_PREFIX}{}.log", std::process::id()));
        let other_pid = std::process::id().wrapping_add(1);
        let other_dir = temp.join(format!("{UPDATE_TEMP_PREFIX}{other_pid}"));

        assert!(update_temp_entry_is_ours(&own_dir));
        assert!(update_temp_entry_is_ours(&own_log));
        assert!(!update_temp_entry_is_ours(&other_dir));
        assert!(!update_temp_entry_is_ours(&temp.join("unrelated.txt")));
    }

    #[test]
    fn test_swap_failure_restores_previous_executable() {
        let temp = std::env::temp_dir().join(format!("synector_swap_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();
        let exe = temp.join("Synector.exe");
        let pending = temp.join("Synector.pending");
        let old = temp.join("Synector.old");
        std::fs::write(&exe, b"OLD_BINARY").unwrap();

        // No staged file: the second rename fails after the running executable was moved aside.
        // The previous executable must be restored and the failure reported.
        let err = swap_pending_executable(&exe, &pending, &old)
            .expect_err("a half-completed swap must fail");
        assert!(err.contains("restored"), "unexpected error: {err}");
        assert_eq!(
            std::fs::read(&exe).unwrap(),
            b"OLD_BINARY",
            "the previous executable must be back at its original path"
        );
        assert!(!old.exists());

        // With a staged file the swap succeeds and the previous executable is kept as .old.
        std::fs::write(&pending, b"NEW_BINARY").unwrap();
        assert!(swap_pending_executable(&exe, &pending, &old).unwrap());
        assert_eq!(std::fs::read(&exe).unwrap(), b"NEW_BINARY");
        assert_eq!(std::fs::read(&old).unwrap(), b"OLD_BINARY");
        assert!(!pending.exists());

        // When the running executable cannot be renamed aside, the caller falls back to the
        // detached restart script instead of reporting success.
        std::fs::remove_file(&old).unwrap();
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("keep.txt"), b"keep").unwrap();
        std::fs::write(&pending, b"NEW_BINARY").unwrap();
        assert!(!swap_pending_executable(&exe, &pending, &old).unwrap());
        assert_eq!(std::fs::read(&exe).unwrap(), b"NEW_BINARY");

        let _ = std::fs::remove_dir_all(&temp);
    }
}
