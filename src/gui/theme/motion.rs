//! Motion policy.
//!
//! A skin owns the intent (how long a drawer takes), the operating system owns the permission:
//! when Windows is configured not to animate client areas, every duration collapses to zero and
//! no transition schedules a repaint. The lookup is cached for the process lifetime.

fn animations_enabled_from_os() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };

    // `SPI_GETCLIENTAREAANIMATION` writes a Win32 `BOOL` (a 32-bit integer) into the buffer.
    let mut enabled: i32 = 0;
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut enabled as *mut i32 as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .is_ok();
    ok && enabled != 0
}

/// Whether the platform wants animated transitions.
pub fn animations_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(animations_enabled_from_os)
}

/// A skin duration as it must be used: unchanged when animation is allowed, zero otherwise.
pub fn duration(seconds: f32) -> f32 {
    if animations_enabled() {
        seconds.max(0.0)
    } else {
        0.0
    }
}

/// Slides a rectangle toward its target with a frame-rate independent ease. Used for the
/// active-tab pill and the segmented-cell indicator, so switching reads as movement instead of
/// a jump. With motion disabled by the platform the target is returned immediately.
pub fn slide_rect(ctx: &egui::Context, id: egui::Id, target: egui::Rect) -> egui::Rect {
    #[derive(Clone, Copy)]
    struct Slide {
        rect: egui::Rect,
    }

    impl Default for Slide {
        fn default() -> Self {
            Self {
                rect: egui::Rect::ZERO,
            }
        }
    }

    let mut slide = ctx
        .data(|data| data.get_temp::<Slide>(id))
        .unwrap_or(Slide { rect: target });

    if animations_enabled() {
        let dt = ctx.input(|input| input.stable_dt).clamp(0.001, 0.1);
        let k = (1.0 - (-dt / 0.055).exp()).clamp(0.0, 1.0);
        let lerp = |from: egui::Pos2, to: egui::Pos2| {
            egui::pos2(from.x + (to.x - from.x) * k, from.y + (to.y - from.y) * k)
        };
        slide.rect = egui::Rect::from_min_max(
            lerp(slide.rect.min, target.min),
            lerp(slide.rect.max, target.max),
        );
        if (slide.rect.min - target.min).length() > 0.6
            || (slide.rect.max - target.max).length() > 0.6
        {
            ctx.request_repaint();
        } else {
            slide.rect = target;
        }
    } else {
        slide.rect = target;
    }

    ctx.data_mut(|data| data.insert_temp(id, slide));
    slide.rect
}

/// Whether the content is scrolling right now. Hover transitions snap during a scroll, so a
/// highlight cannot appear to travel with the content.
pub fn scroll_in_progress(ctx: &egui::Context) -> bool {
    ctx.input(|input| input.smooth_scroll_delta != egui::Vec2::ZERO)
}

/// Animates a boolean for the active skin, honouring the platform motion permission.
///
/// With motion disabled the target value is returned immediately, so no transition state is
/// created and no extra repaint is scheduled.
pub fn animate_bool(ctx: &egui::Context, id: egui::Id, target: bool, seconds: f32) -> f32 {
    let seconds = duration(seconds);
    if seconds <= 0.0 {
        return if target { 1.0 } else { 0.0 };
    }
    ctx.animate_bool_with_time_and_easing(
        id,
        target,
        seconds,
        egui::emath::easing::cubic_out,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disabled_platform_collapses_every_duration() {
        let seconds = 0.24;
        let applied = duration(seconds);
        if animations_enabled() {
            assert!((applied - seconds).abs() < f32::EPSILON);
        } else {
            assert_eq!(applied, 0.0, "no animation means no transition at all");
        }
        assert_eq!(duration(-1.0), 0.0, "a negative duration is never animated");
    }
}
