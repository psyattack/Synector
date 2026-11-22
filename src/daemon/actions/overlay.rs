use std::collections::HashMap;

use super::ActionRunner;
use crate::models::Action;

pub(super) fn execute(action: &Action, context: &HashMap<String, String>) -> Result<(), String> {
    match action {
        Action::Overlay {
            overlay_id,
            custom_overlay,
            operation,
            override_text,
            duration_ms,
            ..
        } => {
            let id = ActionRunner::interpolate_tokens(overlay_id, context);
            let text = override_text.as_ref().map(|t| ActionRunner::interpolate_tokens(t, context));

            let config = crate::config::Library::shared();
            let mut found_overlay = custom_overlay.as_deref().cloned().or_else(|| {
                config.find_overlay(&id).cloned()
            });

            if let Some(ref mut def) = found_overlay {
                if !id.is_empty() && def.id != id && custom_overlay.is_some() {
                    def.id = id.clone();
                }
            }

            let target_id = found_overlay.as_ref().map(|o| o.id.as_str()).unwrap_or(&id);

            match operation {
                crate::models::OverlayOperation::Show => {
                    if let Some(mut overlay) = found_overlay {
                        if let Some(custom_t) = text.clone() {
                            overlay.name = custom_t;
                        }
                        crate::daemon::overlay_manager::show_overlay(&overlay, false, text);
                    } else {
                        let def = match id.as_str() {
                            "overlay_crosshair" => crate::models::OverlayDefinition::default_crosshair(),
                            "overlay_telemetry" => crate::models::OverlayDefinition::default_telemetry(),
                            "overlay_macro_badge" => crate::models::OverlayDefinition::default_macro_badge(),
                            _ => crate::models::OverlayDefinition::default_new_overlay(),
                        };
                        crate::daemon::overlay_manager::show_overlay(&def, false, text);
                    }
                }
                crate::models::OverlayOperation::Hide => {
                    crate::daemon::overlay_manager::hide_overlay(target_id);
                }
                crate::models::OverlayOperation::Toggle => {
                    let overlay = found_overlay.unwrap_or_else(|| match id.as_str() {
                        "overlay_crosshair" => crate::models::OverlayDefinition::default_crosshair(),
                        "overlay_telemetry" => crate::models::OverlayDefinition::default_telemetry(),
                        "overlay_macro_badge" => crate::models::OverlayDefinition::default_macro_badge(),
                        _ => crate::models::OverlayDefinition::default_new_overlay(),
                    });
                    crate::daemon::overlay_manager::toggle_overlay(&overlay, text);
                }
                crate::models::OverlayOperation::Flash => {
                    let dur = duration_ms.unwrap_or(3000);
                    let overlay = found_overlay.unwrap_or_else(|| match id.as_str() {
                        "overlay_crosshair" => crate::models::OverlayDefinition::default_crosshair(),
                        "overlay_telemetry" => crate::models::OverlayDefinition::default_telemetry(),
                        "overlay_macro_badge" => crate::models::OverlayDefinition::default_macro_badge(),
                        _ => crate::models::OverlayDefinition::default_new_overlay(),
                    });
                    crate::daemon::overlay_manager::flash_overlay(&overlay, dur, text);
                }
                crate::models::OverlayOperation::SetText => {
                    let t_str = text.unwrap_or_default();
                    crate::daemon::overlay_manager::update_overlay_text(target_id, &t_str);
                }
            }
            Ok(())
        }
        _ => unreachable!("overlay handler received a non-overlay action"),
    }
}
