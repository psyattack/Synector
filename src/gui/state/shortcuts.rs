//! Desktop shortcut glue used by the rules list: the folder picker and the `.lnk` writer.

use super::GuiState;

impl GuiState {
    pub fn create_shortcut_dialog(&mut self, rule: &crate::models::Rule) {
        let desktop = dirs_desktop().or_else(|| std::env::var_os("USERPROFILE").map(|u| std::path::PathBuf::from(u).join("Desktop")));
        let mut dialog = rfd::FileDialog::new().set_title(self.loc().dialog_select_shortcut_folder);
        if let Some(d) = desktop {
            dialog = dialog.set_directory(d);
        }
        if let Some(folder) = dialog.pick_folder() {
            match crate::shortcut::create_rule_shortcut(rule, &folder) {
                Ok(path) => {
                    let path_str = path.to_string_lossy();
                    self.set_toast(self.loc().toast_shortcut_created.replace("{}", &path_str));
                }
                Err(err) => {
                    self.set_toast(self.loc().toast_shortcut_failed.replace("{}", &err));
                }
            }
        }
    }
}

fn dirs_desktop() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE").map(|u| std::path::PathBuf::from(u).join("Desktop"))
    }
}
