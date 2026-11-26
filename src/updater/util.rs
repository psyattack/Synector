use std::path::PathBuf;

/// Absolute path of a Windows system tool, resolved under `%SystemRoot%\System32`.
///
/// Update helpers must never be spawned by bare name: CreateProcess searches the executable's
/// directory and the inherited working directory before System32, so a planted binary could run
/// with the (possibly elevated) daemon's rights. A missing tool is an error, not a silent fallback
/// to the search order.
#[cfg(windows)]
pub(super) fn system32_tool(name: &str) -> Result<PathBuf, String> {
    let system_root = std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("WINDIR"))
        .map(PathBuf::from)
        .ok_or_else(|| "Cannot locate the Windows directory (SystemRoot is not set)".to_string())?;
    let tool = system_root.join("System32").join(name);
    if !tool.is_file() {
        return Err(format!(
            "Required Windows tool {} is missing; the update cannot continue",
            tool.display()
        ));
    }
    Ok(tool)
}

/// Escapes a value for a PowerShell single-quoted literal by doubling every apostrophe.
///
/// Every path interpolated into the restart script goes through this helper: an
/// apostrophe inside a single-quoted PowerShell string would otherwise end the literal, breaking
/// the script or injecting commands into an elevated process.
pub(super) fn escape_ps_literal(value: &str) -> String {
    crate::win_args::ps_single_quoted(value)
}
