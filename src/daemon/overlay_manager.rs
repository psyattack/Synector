/// Kill-on-close Job Object for overlay children: Windows terminates every
/// overlay process when the daemon handle set closes, including a daemon crash
/// or forced kill that never runs the shutdown path.
#[cfg(windows)]
mod job;
mod monitors;
mod supervisor;
mod text;
mod update;

pub use monitors::get_system_monitors;
#[cfg(windows)]
pub(crate) use monitors::monitor_scale;
pub use supervisor::{
    check_process_terminated, flash_overlay, get_active_overlay_ids, get_calibrated_position,
    hide_all, hide_all_sync, hide_overlay, hide_overlay_if_generation, init, overlay_generation,
    save_calibrated_position, show_overlay, toggle_overlay,
};
pub use text::{live_overlay_text_if_changed, update_overlay_text};
pub use update::{
    hide_update_progress_overlay, show_update_notification, show_update_progress_overlay,
};

#[cfg(test)]
mod tests {
    use super::monitors::monitor_bounds_to_logical;
    use super::supervisor::{active_map, spawn_overlay_process};
    use super::text::{get_overlay_text, live_overlay_text, live_overlay_text_path};
    use super::*;
    use crate::models::OverlayAnchor;
    use crate::models::OverlayDefinition;
    use std::process::Command;

    #[test]
    fn test_get_system_monitors_use_production_anchor_math() {
        let monitors = get_system_monitors();
        assert!(!monitors.is_empty(), "System monitors list must not be empty");
        assert!(monitors.iter().any(|m| m.is_primary), "Must include at least one primary monitor");
        assert_eq!(monitors[0].index, 0);
        assert!(monitors[0].is_primary, "First monitor must always be primary after sorting");

        let mut def = OverlayDefinition::default_new_overlay();
        def.rect.width = 200.0;
        def.rect.height = 100.0;
        def.anchor = OverlayAnchor::TopLeft;
        def.rect.x = 10.0;
        def.rect.y = 20.0;
        def.monitor_index = Some(0);

        // The production placement path is the one overlay_window uses; the old
        // cfg(test)-only copy could drift from it without failing a test.
        let monitor = &monitors[0];
        let (rel_x, rel_y) = crate::models::compute_overlay_screen_pos_with_anchor(
            def.anchor,
            &def,
            monitor.width as f32,
            monitor.height as f32,
        );
        assert_eq!(monitor.x as f32 + rel_x, monitor.x as f32 + 10.0);
        assert_eq!(monitor.y as f32 + rel_y, monitor.y as f32 + 20.0);
    }

    #[test]
    fn test_monitor_bounds_convert_physical_pixels_to_logical_points() {
        // 100% scaling is the identity.
        assert_eq!(monitor_bounds_to_logical(0, 0, 2560, 1440, 96), (0, 0, 2560, 1440));
        // 150% secondary monitor: physical (2560, 0) / 1920x1080 -> logical.
        assert_eq!(monitor_bounds_to_logical(2560, 0, 1920, 1080, 144), (1707, 0, 1280, 720));
        // 200% primary monitor.
        assert_eq!(monitor_bounds_to_logical(0, 0, 3840, 2160, 192), (0, 0, 1920, 1080));
        // Never collapse a scaled monitor to zero.
        assert_eq!(monitor_bounds_to_logical(0, 0, 1, 1, 192), (0, 0, 1, 1));
    }

    #[test]
    fn test_show_overlay_stores_fade_out_and_target_process() {
        init();

        let mut def = OverlayDefinition::default_new_overlay();
        def.id = "mgr_test_ov".to_string();
        def.target_process = Some("game.exe".to_string());
        def.close_with_target = true;
        def.fade_out_ms = Some(450);

        show_overlay(&def, false, None);

        let active_ids = get_active_overlay_ids();
        assert!(active_ids.contains("mgr_test_ov"));

        {
            let lock = active_map().lock().unwrap();
            let proc = lock.get("mgr_test_ov").unwrap();
            assert_eq!(proc.target_process, "game.exe");
            assert!(proc.close_with_target);
            assert_eq!(proc.fade_out_ms, Some(450));
        }

        hide_overlay("mgr_test_ov");
        let active_ids_after = get_active_overlay_ids();
        assert!(!active_ids_after.contains("mgr_test_ov"));
    }

    #[test]
    fn test_overlay_generation_only_hides_its_own_show() {
        init();

        let mut def = OverlayDefinition::default_new_overlay();
        def.id = "mgr_generation_ov".to_string();

        let first = show_overlay(&def, false, None).expect("first show");
        let second = show_overlay(&def, false, None).expect("second show");
        assert_ne!(first, second, "each show gets its own generation");

        // The first run's teardown must not hide the second run's show.
        assert!(!hide_overlay_if_generation("mgr_generation_ov", first));
        assert!(get_active_overlay_ids().contains("mgr_generation_ov"));

        assert!(hide_overlay_if_generation("mgr_generation_ov", second));
        assert!(!get_active_overlay_ids().contains("mgr_generation_ov"));
    }

    #[test]
    fn test_update_notification_and_progress_overlays() {
        init();

        show_update_notification("2.5.0");
        let active_ids = get_active_overlay_ids();
        assert!(active_ids.contains("overlay_update_notification"));

        hide_overlay("overlay_update_notification");
        assert!(!get_active_overlay_ids().contains("overlay_update_notification"));

        show_update_progress_overlay("2.5.0");
        assert!(get_active_overlay_ids().contains("overlay_update_progress"));

        hide_update_progress_overlay();
        assert!(!get_active_overlay_ids().contains("overlay_update_progress"));
    }

    #[test]
    fn test_dynamic_overlay_text_cache() {
        init();
        assert!(get_overlay_text("test_ov_cache").is_none());

        update_overlay_text("test_ov_cache", "Initial text");
        assert_eq!(get_overlay_text("test_ov_cache").as_deref(), Some("Initial text"));

        update_overlay_text("test_ov_cache", "Updated dynamic text");
        assert_eq!(get_overlay_text("test_ov_cache").as_deref(), Some("Updated dynamic text"));
    }

    #[test]
    fn test_update_overlay_text_publishes_live_text_without_restarting() {
        init();
        let id = "live_text_ov";
        let path = live_overlay_text_path(id);
        let _ = std::fs::remove_file(&path);
        assert!(live_overlay_text(id).is_none());

        update_overlay_text(id, "first");
        assert_eq!(live_overlay_text(id).as_deref(), Some("first"));
        assert_eq!(
            live_overlay_text_if_changed(id, Some("first")),
            None,
            "an unchanged label must not force a repaint"
        );
        assert_eq!(
            live_overlay_text_if_changed(id, Some("stale")).as_deref(),
            Some("first")
        );

        update_overlay_text(id, "second");
        assert_eq!(live_overlay_text(id).as_deref(), Some("second"));

        // Different overlay ids never share a live-text file.
        assert_ne!(live_overlay_text_path(id), live_overlay_text_path("live_text_ov_2"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_show_overlay_publishes_live_text_for_running_window() {
        init();
        let mut def = OverlayDefinition::default_new_overlay();
        def.id = "live_text_show_ov".to_string();
        show_overlay(&def, false, Some("shown".to_string()));
        assert_eq!(live_overlay_text(&def.id).as_deref(), Some("shown"));
        hide_overlay(&def.id);
        let _ = std::fs::remove_file(live_overlay_text_path(&def.id));
    }

    #[test]
    fn test_failed_spawn_is_reported_and_not_active() {
        init();
        // `show_overlay` short-circuits spawning under cfg(test), so exercise the
        // helper directly: a missing executable must yield None (logged) and no
        // active overlay, instead of a phantom entry.
        let command = Command::new("synector_definitely_missing_overlay_binary");
        let child = spawn_overlay_process("spawn_failure_ov", command);
        assert!(child.is_none(), "spawning a missing binary must fail");
        assert!(!get_active_overlay_ids().contains("spawn_failure_ov"));
    }
}
