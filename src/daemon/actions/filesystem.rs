use std::collections::HashMap;

use super::support::command::{apply_no_window, run_command_bounded_async, ARCHIVE_TIMEOUT};
use super::ActionRunner;
use crate::models::Action;

pub(super) async fn execute_archive_action(action: &Action, context: &HashMap<String, String>) -> Result<(), String> {
    match action {
        Action::Archive {
            operation,
            format,
            source_path,
            destination_path,
            compression_level,
            overwrite,
        } => {
            let src = ActionRunner::interpolate_tokens(source_path, context);
            let dst = ActionRunner::interpolate_tokens(destination_path, context);
            super::execute_archive(*operation, *format, &src, &dst, *compression_level, *overwrite).await?;
            Ok(())
        }
        _ => unreachable!("archive handler received a non-archive action"),
    }
}

pub(super) fn execute_fs_action(action: &Action, context: &HashMap<String, String>) -> Result<(), String> {
    match action {
        Action::FileSystem {
            operation,
            path,
            target_path,
            content,
            create_parents,
            overwrite,
        } => {
            let p = ActionRunner::interpolate_tokens(path, context);
            let target = target_path.as_ref().map(|s| ActionRunner::interpolate_tokens(s, context));
            let body = content.as_ref().map(|s| ActionRunner::interpolate_tokens(s, context));
            super::execute_file_system(*operation, &p, target.as_deref(), body.as_deref(), *create_parents, *overwrite)?;
            Ok(())
        }
        _ => unreachable!("filesystem handler received a non-filesystem action"),
    }
}

pub async fn execute_archive(
    operation: crate::models::ArchiveOperation,
    format: crate::models::ArchiveFormat,
    source: &str,
    destination: &str,
    _compression_level: Option<u8>,
    overwrite: bool,
) -> Result<(), String> {
    use std::fs::File;
    use std::path::Path;
    use crate::models::{ArchiveFormat, ArchiveOperation};

    let src_path = Path::new(source);
    let dst_path = Path::new(destination);

    match operation {
        ArchiveOperation::Pack => {
            if !src_path.exists() {
                return Err(format!("Archive source does not exist: {source}"));
            }
            if dst_path.exists() && !overwrite {
                return Err(format!("Archive destination already exists and overwrite is disabled: {destination}"));
            }
            if let Some(parent) = dst_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            match format {
                ArchiveFormat::Zip => {
                    let file = File::create(dst_path).map_err(|e| format!("Failed to create zip file: {e}"))?;
                    let mut zip = zip::ZipWriter::new(file);
                    let options = zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Deflated);

                    if src_path.is_file() {
                        let file_name = src_path.file_name().and_then(|f| f.to_str()).unwrap_or("file");
                        zip.start_file(file_name, options).map_err(|e| e.to_string())?;
                        let mut f = File::open(src_path).map_err(|e| e.to_string())?;
                        std::io::copy(&mut f, &mut zip).map_err(|e| e.to_string())?;
                    } else if src_path.is_dir() {
                        fn add_dir(dir: &Path, prefix: &Path, zip: &mut zip::ZipWriter<File>, options: zip::write::SimpleFileOptions) -> Result<(), String> {
                            for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
                                let entry = entry.map_err(|e| e.to_string())?;
                                let path = entry.path();
                                let rel = path.strip_prefix(prefix).map_err(|e| e.to_string())?;
                                let rel_str = rel.to_string_lossy().replace('\\', "/");
                                if path.is_dir() {
                                    zip.add_directory(&rel_str, options).map_err(|e| e.to_string())?;
                                    add_dir(&path, prefix, zip, options)?;
                                } else if path.is_file() {
                                    zip.start_file(&rel_str, options).map_err(|e| e.to_string())?;
                                    let mut f = File::open(&path).map_err(|e| e.to_string())?;
                                    std::io::copy(&mut f, zip).map_err(|e| e.to_string())?;
                                }
                            }
                            Ok(())
                        }
                        add_dir(src_path, src_path, &mut zip, options)?;
                    }
                    zip.finish().map_err(|e| format!("Failed to finalize zip archive: {e}"))?;
                    Ok(())
                }
                ArchiveFormat::Tar | ArchiveFormat::TarGz | ArchiveFormat::TarXz | ArchiveFormat::SevenZip => {
                    let src_parent = src_path.parent().unwrap_or(Path::new("."));
                    let src_name = src_path.file_name().unwrap_or_default();
                    let mut cmd = std::process::Command::new("tar.exe");
                    cmd.args(["-a", "-cf", destination, "-C", &src_parent.to_string_lossy(), &src_name.to_string_lossy()]);
                    #[cfg(windows)]
                    {
                        use windows::Win32::System::Threading::CREATE_NO_WINDOW;
                        use std::os::windows::process::CommandExt;
                        cmd.creation_flags(CREATE_NO_WINDOW.0);
                    }
                    let status = run_command_bounded_async(cmd, ARCHIVE_TIMEOUT).await?;
                    if status.timed_out {
                        return Err(format!(
                            "tar.exe archive creation timed out after {}s",
                            ARCHIVE_TIMEOUT.as_secs()
                        ));
                    }
                    if !status.succeeded() {
                        return Err(format!("tar.exe archive creation returned exit code {:?}", status.exit_code()));
                    }
                    Ok(())
                }
                ArchiveFormat::Rar => {
                    let rar_paths = [
                        "rar.exe",
                        r"C:\Program Files\WinRAR\rar.exe",
                        r"C:\Program Files (x86)\WinRAR\rar.exe",
                    ];
                    let mut found_cmd = None;
                    for p in rar_paths {
                        let mut probe = std::process::Command::new(p);
                        probe.arg("-?");
                        apply_no_window(&mut probe);
                        if let Ok(out) =
                            run_command_bounded_async(probe, std::time::Duration::from_secs(5)).await
                        {
                            if out.status.is_some() {
                                found_cmd = Some(p);
                                break;
                            }
                        }
                    }
                    if let Some(cmd_path) = found_cmd {
                        let mut cmd = std::process::Command::new(cmd_path);
                        cmd.args(["a", "-ep1", "-r", destination, source]);
                        apply_no_window(&mut cmd);
                        let status = run_command_bounded_async(cmd, ARCHIVE_TIMEOUT).await?;
                        if status.timed_out {
                            return Err(format!(
                                "rar.exe timed out after {}s",
                                ARCHIVE_TIMEOUT.as_secs()
                            ));
                        }
                        if !status.succeeded() {
                            return Err(format!("rar.exe returned exit code {:?}", status.exit_code()));
                        }
                        Ok(())
                    } else {
                        Err("Creating .rar archives requires WinRAR (rar.exe) in PATH or Program Files. Tip: Use Zip or 7z for native compression.".to_string())
                    }
                }
            }
        }
        ArchiveOperation::Unpack => {
            if !src_path.exists() {
                return Err(format!("Archive source file does not exist: {source}"));
            }
            std::fs::create_dir_all(dst_path).map_err(|e| format!("Failed to create destination folder: {e}"))?;

            match format {
                ArchiveFormat::Zip => {
                    let file = File::open(src_path).map_err(|e| format!("Failed to open zip file: {e}"))?;
                    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("Invalid zip archive: {e}"))?;
                    for i in 0..archive.len() {
                        let mut item = archive.by_index(i).map_err(|e| e.to_string())?;
                        let outpath = match item.enclosed_name() {
                            Some(p) => dst_path.join(p),
                            None => continue,
                        };
                        if item.is_dir() {
                            std::fs::create_dir_all(&outpath).map_err(|e| e.to_string())?;
                        } else {
                            if let Some(p) = outpath.parent() {
                                if !p.exists() {
                                    std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
                                }
                            }
                            if outpath.exists() && !overwrite {
                                continue;
                            }
                            let mut outfile = File::create(&outpath).map_err(|e| e.to_string())?;
                            std::io::copy(&mut item, &mut outfile).map_err(|e| e.to_string())?;
                        }
                    }
                    Ok(())
                }
                ArchiveFormat::Tar | ArchiveFormat::TarGz | ArchiveFormat::TarXz | ArchiveFormat::SevenZip | ArchiveFormat::Rar => {
                    let mut cmd = std::process::Command::new("tar.exe");
                    cmd.args(["-xf", source, "-C", destination]);
                    #[cfg(windows)]
                    {
                        use windows::Win32::System::Threading::CREATE_NO_WINDOW;
                        use std::os::windows::process::CommandExt;
                        cmd.creation_flags(CREATE_NO_WINDOW.0);
                    }
                    match run_command_bounded_async(cmd, ARCHIVE_TIMEOUT).await {
                        Ok(status) if status.succeeded() => Ok(()),
                        Ok(status) => {
                            if format == ArchiveFormat::Rar {
                                let mut unrar = std::process::Command::new("unrar.exe");
                                unrar.args(["x", "-o+", source, destination]);
                                apply_no_window(&mut unrar);
                                if let Ok(s) =
                                    run_command_bounded_async(unrar, ARCHIVE_TIMEOUT).await
                                {
                                    if s.succeeded() {
                                        return Ok(());
                                    }
                                }
                            }
                            let detail = if status.timed_out {
                                " (timed out)".to_string()
                            } else {
                                format!(" (exit code {:?})", status.exit_code())
                            };
                            Err(format!("Failed to unpack {:?} archive using tar.exe or system extractors{detail}", format))
                        }
                        Err(e) => Err(format!("Failed to run tar.exe: {e}")),
                    }
                }
            }
        }
    }
}

pub fn execute_file_system(
    operation: crate::models::FileSystemOperation,
    path: &str,
    target_path: Option<&str>,
    content: Option<&str>,
    create_parents: bool,
    overwrite: bool,
) -> Result<(), String> {
    use std::path::Path;
    use crate::models::FileSystemOperation;

    let src = Path::new(path);

    match operation {
        FileSystemOperation::CreateFile => {
            if let Some(parent) = src.parent() {
                if create_parents && !parent.exists() {
                    std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create parent directories: {e}"))?;
                }
            }
            if src.exists() && !overwrite {
                return Ok(());
            }
            std::fs::File::create(src).map_err(|e| format!("Failed to create file {:?}: {e}", src))?;
        }
        FileSystemOperation::CreateDirectory => {
            std::fs::create_dir_all(src).map_err(|e| format!("Failed to create directory {:?}: {e}", src))?;
        }
        FileSystemOperation::Rename => {
            let target_str = target_path.ok_or_else(|| "Target path must be specified for Rename".to_string())?;
            let dst = Path::new(target_str);
            if let Some(parent) = dst.parent() {
                if create_parents && !parent.exists() {
                    std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create parent directories: {e}"))?;
                }
            }
            std::fs::rename(src, dst).map_err(|e| format!("Failed to rename {:?} to {:?}: {e}", src, dst))?;
        }
        FileSystemOperation::Copy => {
            let target_str = target_path.ok_or_else(|| "Target path must be specified for Copy/Clone".to_string())?;
            let dst = Path::new(target_str);
            if src.is_file() {
                if dst.exists() && !overwrite {
                    return Ok(());
                }
                if let Some(parent) = dst.parent() {
                    if create_parents && !parent.exists() {
                        std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create parent directories: {e}"))?;
                    }
                }
                std::fs::copy(src, dst).map_err(|e| format!("Failed to copy file {:?} to {:?}: {e}", src, dst))?;
            } else if src.is_dir() {
                fn copy_dir_all(src: &Path, dst: &Path, overwrite: bool) -> Result<(), String> {
                    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
                    for entry in std::fs::read_dir(src).map_err(|e| e.to_string())? {
                        let entry = entry.map_err(|e| e.to_string())?;
                        let entry_path = entry.path();
                        let target_entry = dst.join(entry.file_name());
                        if entry_path.is_dir() {
                            copy_dir_all(&entry_path, &target_entry, overwrite)?;
                        } else {
                            if target_entry.exists() && !overwrite {
                                continue;
                            }
                            std::fs::copy(&entry_path, &target_entry).map_err(|e| e.to_string())?;
                        }
                    }
                    Ok(())
                }
                copy_dir_all(src, dst, overwrite)?;
            } else {
                return Err(format!("Source path does not exist: {:?}", src));
            }
        }
        FileSystemOperation::Delete => {
            if src.exists() {
                if src.is_file() {
                    std::fs::remove_file(src).map_err(|e| format!("Failed to delete file {:?}: {e}", src))?;
                } else if src.is_dir() {
                    std::fs::remove_dir_all(src).map_err(|e| format!("Failed to delete directory {:?}: {e}", src))?;
                }
            }
        }
        FileSystemOperation::WriteText => {
            if src.exists() && !overwrite {
                return Ok(());
            }
            if let Some(parent) = src.parent() {
                if create_parents && !parent.exists() {
                    std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create parent directories: {e}"))?;
                }
            }
            let data = content.unwrap_or_default();
            std::fs::write(src, data).map_err(|e| format!("Failed to write text to {:?}: {e}", src))?;
        }
        FileSystemOperation::AppendText => {
            use std::io::Write;
            if let Some(parent) = src.parent() {
                if create_parents && !parent.exists() {
                    std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create parent directories: {e}"))?;
                }
            }
            let data = content.unwrap_or_default();
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(src)
                .map_err(|e| format!("Failed to open file for append {:?}: {e}", src))?;
            file.write_all(data.as_bytes()).map_err(|e| format!("Failed to append to {:?}: {e}", src))?;
        }
    }
    Ok(())
}
