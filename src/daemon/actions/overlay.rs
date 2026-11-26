use std::collections::HashMap;

use super::ActionRunner;
use crate::models::Action;

/// Executes one overlay step and returns the `(overlay id, show generation)` of an overlay it
/// actually put on screen, so the pipeline can own that exact show.
pub(super) fn execute(
    action: &Action,
    context: &HashMap<String, String>,
) -> Result<Option<(String, u64)>, String> {
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

            let target_id = found_overlay
                .as_ref()
                .map(|o| o.id.clone())
                .unwrap_or_else(|| id.clone());

            let shown = match operation {
                crate::models::OverlayOperation::Show => {
                    let generation = if let Some(mut overlay) = found_overlay {
                        if let Some(custom_t) = text.clone() {
                            overlay.name = custom_t;
                        }
                        crate::daemon::overlay_manager::show_overlay(&overlay, false, text)
                    } else {
                        let mut def = match id.as_str() {
                            "overlay_crosshair" => crate::models::OverlayDefinition::default_crosshair(),
                            "overlay_telemetry" => crate::models::OverlayDefinition::default_telemetry(),
                            "overlay_macro_badge" => crate::models::OverlayDefinition::default_macro_badge(),
                            _ => crate::models::OverlayDefinition::default_new_overlay(),
                        };
                        def.id = target_id.clone();
                        crate::daemon::overlay_manager::show_overlay(&def, false, text)
                    };
                    generation.map(|generation| (target_id.clone(), generation))
                }
                crate::models::OverlayOperation::Hide => {
                    crate::daemon::overlay_manager::hide_overlay(&target_id);
                    None
                }
                crate::models::OverlayOperation::Toggle => {
                    let mut overlay = found_overlay.unwrap_or_else(|| match id.as_str() {
                        "overlay_crosshair" => crate::models::OverlayDefinition::default_crosshair(),
                        "overlay_telemetry" => crate::models::OverlayDefinition::default_telemetry(),
                        "overlay_macro_badge" => crate::models::OverlayDefinition::default_macro_badge(),
                        _ => crate::models::OverlayDefinition::default_new_overlay(),
                    });
                    overlay.id = target_id.clone();
                    crate::daemon::overlay_manager::toggle_overlay(&overlay, text)
                        .map(|generation| (target_id.clone(), generation))
                }
                crate::models::OverlayOperation::Flash => {
                    let dur = duration_ms.unwrap_or(3000);
                    let mut overlay = found_overlay.unwrap_or_else(|| match id.as_str() {
                        "overlay_crosshair" => crate::models::OverlayDefinition::default_crosshair(),
                        "overlay_telemetry" => crate::models::OverlayDefinition::default_telemetry(),
                        "overlay_macro_badge" => crate::models::OverlayDefinition::default_macro_badge(),
                        _ => crate::models::OverlayDefinition::default_new_overlay(),
                    });
                    overlay.id = target_id.clone();
                    crate::daemon::overlay_manager::flash_overlay(&overlay, dur, text)
                        .map(|generation| (target_id.clone(), generation))
                }
                crate::models::OverlayOperation::SetText => {
                    let t_str = text.unwrap_or_default();
                    crate::daemon::overlay_manager::update_overlay_text(&target_id, &t_str);
                    None
                }
            };
            Ok(shown)
        }
        _ => unreachable!("overlay handler received a non-overlay action"),
    }
}
