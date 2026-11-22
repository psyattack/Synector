use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use super::icon::{GLOBAL_ENGINE, GLOBAL_RT};

static AVAILABLE_UPDATE: Mutex<Option<crate::updater::ReleaseInfo>> = Mutex::new(None);

static IS_UPDATING: AtomicBool = AtomicBool::new(false);
static UPDATE_PROGRESS_PCT: AtomicU32 = AtomicU32::new(0);
static UPDATE_TASK_HANDLE: Mutex<Option<tokio::task::AbortHandle>> = Mutex::new(None);

pub fn set_available_update(info: Option<crate::updater::ReleaseInfo>) {
    if let Ok(mut lock) = AVAILABLE_UPDATE.lock() {
        *lock = info;
    }
}

pub fn get_available_update() -> Option<crate::updater::ReleaseInfo> {
    AVAILABLE_UPDATE.lock().ok().and_then(|l| l.clone())
}

pub fn set_updating(updating: bool, progress: Option<f32>) {
    IS_UPDATING.store(updating, Ordering::SeqCst);
    let pct = progress.map(|p| (p * 100.0).clamp(0.0, 100.0) as u32).unwrap_or(0);
    UPDATE_PROGRESS_PCT.store(pct, Ordering::SeqCst);
}

pub fn is_updating() -> bool {
    IS_UPDATING.load(Ordering::SeqCst)
}

#[cfg(test)]
pub fn get_update_progress_pct() -> u32 {
    UPDATE_PROGRESS_PCT.load(Ordering::SeqCst)
}

pub fn abort_active_update() {
    if let Ok(mut lock) = UPDATE_TASK_HANDLE.lock() {
        if let Some(handle) = lock.take() {
            handle.abort();
        }
    }
    set_updating(false, None);
    crate::updater::clean_temp_update_files();
    crate::daemon::overlay_manager::hide_update_progress_overlay();
}

pub fn start_update_download(show_overlay: bool) {
    let update_info = match get_available_update() {
        Some(info) => info,
        None => return,
    };

    if is_updating() {
        return;
    }

    set_updating(true, Some(0.0));
    let version = update_info.version.clone();

    if show_overlay {
        crate::daemon::overlay_manager::show_update_progress_overlay(&version);
    }

    if let Ok(lock) = GLOBAL_ENGINE.lock() {
        if let Some(engine) = lock.as_ref() {
            if let Ok(rt_lock) = GLOBAL_RT.lock() {
                if let Some(rt) = rt_lock.as_ref() {
                    let eng_clone = Arc::clone(engine);
                    let ver = version.clone();
                    let handle = rt.spawn(async move {
                        {
                            let mut eng = eng_clone.lock().await;
                            eng.begin_update_download();
                            eng.set_update_status("Connecting...".to_string());
                        }

                        let eng_progress = Arc::clone(&eng_clone);
                        let ver_for_overlay = ver.clone();
                        let res = crate::updater::download_and_install_daemon(&update_info, move |pct| {
                            set_updating(true, Some(pct));
                            let gui_open = crate::single_instance::is_gui_running();
                            let active = crate::daemon::overlay_manager::get_active_overlay_ids();
                            if !gui_open {
                                if !active.contains("overlay_update_progress") {
                                    crate::daemon::overlay_manager::show_update_progress_overlay(&ver_for_overlay);
                                }
                            } else if active.contains("overlay_update_progress") {
                                crate::daemon::overlay_manager::hide_update_progress_overlay();
                            }
                            let eng_p = Arc::clone(&eng_progress);
                            tokio::spawn(async move {
                                let mut eng = eng_p.lock().await;
                                eng.set_update_progress(pct);
                                eng.set_update_status(format!("{:.0}%", pct * 100.0));
                            });
                        }).await;

                        match res {
                            Ok(()) => {
                                // Successfully applied update; apply_update_and_restart exits process
                            }
                            Err(err) => {
                                eprintln!("[DAEMON] Update error: {err}");
                                set_updating(false, None);
                                crate::updater::clean_temp_update_files();
                                let mut eng = eng_clone.lock().await;
                                eng.fail_update(format!("Error: {err}"));
                                eng.log("ERROR", "UPDATER", &format!("Update failed: {err}"));
                                tokio::time::sleep(tokio::time::Duration::from_millis(4000)).await;
                                crate::daemon::overlay_manager::hide_update_progress_overlay();
                            }
                        }
                    });

                    if let Ok(mut h_lock) = UPDATE_TASK_HANDLE.lock() {
                        *h_lock = Some(handle.abort_handle());
                    }
                }
            }
        }
    }
}

pub fn trigger_update_flow(silent: bool) {
    if is_updating() {
        return;
    }

    if get_available_update().is_none() {
        return;
    }

    if crate::single_instance::is_gui_running() {
        // 1. If GUI is open
        if !silent {
            crate::single_instance::focus_existing_gui_window();
            if let Ok(lock) = GLOBAL_ENGINE.lock() {
                if let Some(engine) = lock.as_ref() {
                    if let Ok(rt_lock) = GLOBAL_RT.lock() {
                        if let Some(rt) = rt_lock.as_ref() {
                            let eng_clone = Arc::clone(engine);
                            rt.spawn(async move {
                                let mut eng = eng_clone.lock().await;
                                eng.request_open_update_modal();
                            });
                        }
                    }
                }
            }
        } else {
            start_update_download(false);
        }
    } else {
        // 2. If GUI is closed and only daemon is running:
        start_update_download(!silent);
    }
}

pub(super) fn handle_tray_update_click() {
    trigger_update_flow(false);
}
