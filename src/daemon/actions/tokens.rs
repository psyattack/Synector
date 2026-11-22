use std::collections::HashMap;

pub(super) fn interpolate_tokens(text: &str, context: &HashMap<String, String>) -> String {
    let mut result = text.to_string();
    for (k, v) in context {
        result = result.replace(&format!("{{{}}}", k), v);
    }
    if result.contains("{Clipboard}") {
        let cb = crate::clipboard::read_text().unwrap_or_else(|_| String::new());
        result = result.replace("{Clipboard}", &cb);
    }
    if result.contains("{Date}") {
        let dt = chrono::Local::now().format("%Y-%m-%d").to_string();
        result = result.replace("{Date}", &dt);
    }
    if result.contains("{Time}") {
        let tm = chrono::Local::now().format("%H:%M:%S").to_string();
        result = result.replace("{Time}", &tm);
    }
    while let Some(start) = result.find("{Env:") {
        if let Some(end) = result[start..].find('}') {
            let full_tag = &result[start..start + end + 1];
            let var_name = &full_tag[5..full_tag.len() - 1];
            let val = std::env::var(var_name).unwrap_or_default();
            result = result.replace(full_tag, &val);
        } else {
            break;
        }
    }
    if result.contains("{ProcessPath}") || result.contains("{ProcessDir}") {
        let proc_path = context.get("ProcessPath").cloned().or_else(|| {
            context.get("ProcessName").and_then(|name| crate::daemon::watcher::ProcessWatcher::find_exe_path_by_name(name))
        }).unwrap_or_default();
        let proc_dir = std::path::Path::new(&proc_path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        result = result.replace("{ProcessPath}", &proc_path);
        result = result.replace("{ProcessDir}", &proc_dir);
    }
    if result.contains("{WindowTitle}") {
        let wt = context.get("WindowTitle").cloned().unwrap_or_default();
        result = result.replace("{WindowTitle}", &wt);
    }
    result
}
