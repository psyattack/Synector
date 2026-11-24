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
