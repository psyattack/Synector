use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use super::ActionRunner;
use crate::models::Action;

#[cfg(windows)]
use super::input::{mouse_control, parse_virtual_key, send_key_event, simulate_text_typing};
#[cfg(windows)]
use crate::models::{MouseActionType, MouseButton, MouseCoordMode};

static ACTIVE_MACRO_CANCELLATIONS: OnceLock<Mutex<HashMap<String, Vec<Arc<AtomicBool>>>>> = OnceLock::new();

pub(super) fn macro_cancellations() -> &'static Mutex<HashMap<String, Vec<Arc<AtomicBool>>>> {
    ACTIVE_MACRO_CANCELLATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn stop_macro_by_id_or_name(id_or_name: &str) {
    let mut lock = macro_cancellations().lock().unwrap();
    let config = crate::config::Library::shared();
    let resolved_id = config.find_macro(id_or_name).map(|p| p.id.as_str()).unwrap_or(id_or_name);

    if let Some(tokens) = lock.get_mut(resolved_id) {
        for token in tokens.drain(..) {
            token.store(true, Ordering::SeqCst);
        }
    }
    if let Some(tokens) = lock.get_mut(id_or_name) {
        for token in tokens.drain(..) {
            token.store(true, Ordering::SeqCst);
        }
    }
}

pub fn stop_all_active_macros() {
    let mut lock = macro_cancellations().lock().unwrap();
    for (_, tokens) in lock.drain() {
        for token in tokens {
            token.store(true, Ordering::SeqCst);
        }
    }
}

pub fn is_macro_running(id_or_name: &str) -> bool {
    let mut lock = macro_cancellations().lock().unwrap();
    let config = crate::config::Library::shared();
    let resolved_id = config.find_macro(id_or_name).map(|p| p.id.as_str()).unwrap_or(id_or_name);

    if let Some(tokens) = lock.get_mut(resolved_id) {
        tokens.retain(|t| !t.load(Ordering::Relaxed));
        if !tokens.is_empty() {
            return true;
        }
    }
    if let Some(tokens) = lock.get_mut(id_or_name) {
        tokens.retain(|t| !t.load(Ordering::Relaxed));
        if !tokens.is_empty() {
            return true;
        }
    }
    false
}

pub fn has_any_active_macro() -> bool {
    let mut lock = macro_cancellations().lock().unwrap();
    for tokens in lock.values_mut() {
        tokens.retain(|t| !t.load(Ordering::Relaxed));
        if !tokens.is_empty() {
            return true;
        }
    }
    false
}

pub(super) async fn execute(action: &Action, context: &HashMap<String, String>) -> Result<(), String> {
    match action {
        Action::Macro {
            preset_id,
            operation,
            events,
            repeat,
            speed,
            infinite,
        } => {
            let config = crate::config::Library::shared();
            let interp_preset_id = preset_id.as_ref().map(|p| ActionRunner::interpolate_tokens(p, context));
            let found_preset = interp_preset_id.as_deref().and_then(|pid| config.find_macro(pid));

            match operation {
                crate::models::MacroOperation::Stop => {
                    if let Some(ref pid) = interp_preset_id {
                        super::stop_macro_by_id_or_name(pid);
                    } else {
                        super::stop_all_active_macros();
                    }
                    Ok(())
                }
                crate::models::MacroOperation::Toggle => {
                    let is_running = if let Some(ref pid) = interp_preset_id {
                        super::is_macro_running(pid)
                    } else {
                        super::has_any_active_macro()
                    };

                    if is_running {
                        if let Some(ref pid) = interp_preset_id {
                            super::stop_macro_by_id_or_name(pid);
                        } else {
                            super::stop_all_active_macros();
                        }
                        Ok(())
                    } else {
                        let (macro_events, macro_repeat, macro_speed, macro_infinite) =
                            resolve_macro_plan(events, *repeat, *speed, *infinite, found_preset);
                        let key_to_register = interp_preset_id
                            .clone()
                            .or_else(|| found_preset.map(|p| p.id.clone()));
                        run_macro(
                            macro_events,
                            macro_repeat,
                            macro_speed,
                            macro_infinite,
                            key_to_register,
                            context,
                        )
                        .await
                    }
                }
                crate::models::MacroOperation::Run => {
                    let (macro_events, macro_repeat, macro_speed, macro_infinite) =
                        resolve_macro_plan(events, *repeat, *speed, *infinite, found_preset);
                    let key_to_register = interp_preset_id
                        .clone()
                        .or_else(|| found_preset.map(|p| p.id.clone()));
                    run_macro(
                        macro_events,
                        macro_repeat,
                        macro_speed,
                        macro_infinite,
                        key_to_register,
                        context,
                    )
                    .await
                }
            }
        }
        _ => unreachable!("macro handler received a non-macro action"),
    }
}

pub(super) fn random_range(min: u32, max: u32) -> u32 {
    if min >= max {
        return min;
    }
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEED: AtomicU64 = AtomicU64::new(0x853c49e6748fea9b);
    let mut s = SEED.fetch_add(0x9e3779b97f4a7c15, Ordering::Relaxed);
    s ^= s >> 30;
    s = s.wrapping_mul(0xbf58476d1ce4e5b9);
    s ^= s >> 27;
    s = s.wrapping_mul(0x94d049bb133111eb);
    s ^= s >> 31;
    let range = (max - min + 1) as u64;
    min + (s % range) as u32
}

pub(crate) fn random_gaussian(mean: f64, std_dev: f64) -> f64 {
    let u1 = ((random_range(1, 10000) as f64) / 10000.0).max(1e-6);
    let u2 = ((random_range(1, 10000) as f64) / 10000.0).max(1e-6);
    let z0 = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
    mean + z0 * std_dev
}

pub(crate) fn calculate_sleep_ms(base_ms: u32, organic: bool, jitter_percent: u32, ch_opt: Option<char>) -> u64 {
    if !organic {
        return base_ms as u64;
    }
    let base = if base_ms == 0 { 25 } else { base_ms } as f64;
    let jitter = (jitter_percent.clamp(5, 95) as f64) / 100.0;
    let std_dev = base * (jitter / 2.3);
    let sample = super::random_gaussian(base, std_dev);
    let mut chosen = sample.round().max(5.0) as u64;

    // Rare biological hesitation / micro-pause: ~1.2% chance of 120-260ms delay
    if random_range(1, 1000) <= 12 {
        chosen += random_range(120, 260) as u64;
    }

    if let Some(ch) = ch_opt {
        if ch == ' ' || ch == ',' || ch == '.' || ch == '!' || ch == '?' || ch == '\n' {
            let pause = super::random_gaussian(45.0, 15.0).round().max(10.0) as u64;
            chosen += pause;
        }
    }
    chosen
}

/// Merges an inline macro plan with a saved preset. Explicit action fields win, except
/// that the GUI defaults (one repeat, normal speed) let a preset supply its own values.
pub(super) fn resolve_macro_plan(
    events: &[crate::models::MacroEvent],
    repeat: u32,
    speed: f32,
    infinite: bool,
    preset: Option<&crate::models::MacroPreset>,
) -> (Vec<crate::models::MacroEvent>, u32, f32, bool) {
    if let Some(preset) = preset {
        let events = if events.is_empty() {
            preset.events.clone()
        } else {
            events.to_vec()
        };
        let repeat = if repeat == 1 && preset.repeat > 1 {
            preset.repeat
        } else {
            repeat
        };
        let speed = if (speed - 1.0).abs() < f32::EPSILON
            && (preset.speed - 1.0).abs() > f32::EPSILON
        {
            preset.speed
        } else {
            speed
        };
        (events, repeat, speed, infinite || preset.infinite)
    } else {
        (events.to_vec(), repeat, speed, infinite)
    }
}

/// Replays one macro plan: registers the cancellation token for Toggle/Run bookkeeping,
/// repeats until done or cancelled, and always unregisters the token afterwards.
pub(super) async fn run_macro(
    events: Vec<crate::models::MacroEvent>,
    repeat: u32,
    speed: f32,
    infinite: bool,
    key: Option<String>,
    context: &HashMap<String, String>,
) -> Result<(), String> {
    let cancel_flag = Arc::new(AtomicBool::new(false));
    if let Some(key) = key.as_ref() {
        let mut lock = macro_cancellations().lock().unwrap();
        lock.entry(key.clone())
            .or_default()
            .push(Arc::clone(&cancel_flag));
    }

    let repeats = repeat.max(1);
    let mut count = 0;
    let mut result = Ok(());
    while (infinite || count < repeats) && !cancel_flag.load(Ordering::Relaxed) {
        for event in &events {
            if cancel_flag.load(Ordering::Relaxed) {
                break;
            }
            if let Err(err) = execute_macro_event(event, speed, context).await {
                result = Err(err);
                break;
            }
        }
        if result.is_err() {
            break;
        }
        count += 1;
    }

    if let Some(key) = key.as_ref() {
        let mut lock = macro_cancellations().lock().unwrap();
        if let Some(tokens) = lock.get_mut(key) {
            tokens.retain(|flag| !Arc::ptr_eq(flag, &cancel_flag));
        }
    }
    result
}

pub async fn execute_macro_event(
    event: &crate::models::MacroEvent,
    speed: f32,
    context: &HashMap<String, String>,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        match event {
            crate::models::MacroEvent::KeyDown { key } => {
                if let Some(vk) = parse_virtual_key(key) {
                    send_key_event(vk, false);
                }
            }
            crate::models::MacroEvent::KeyUp { key } => {
                if let Some(vk) = parse_virtual_key(key) {
                    send_key_event(vk, true);
                }
            }
            crate::models::MacroEvent::KeyPress { key } => {
                if let Some(vk) = parse_virtual_key(key) {
                    send_key_event(vk, false);
                    let hold_ms = random_gaussian(55.0, 12.0).round().clamp(35.0, 110.0) as u64;
                    tokio::time::sleep(std::time::Duration::from_millis(hold_ms)).await;
                    send_key_event(vk, true);
                }
            }
            crate::models::MacroEvent::Text { text, delay_ms, organic } => {
                let interpolated = ActionRunner::interpolate_tokens(text, context);
                simulate_text_typing(&interpolated, *delay_ms, *organic, 30).await?;
            }
            crate::models::MacroEvent::MouseMove { x, y, coord_mode, smooth, duration_ms, organic } => {
                let default_dur = if *smooth { 250 } else { 16 };
                let base_dur = duration_ms.unwrap_or(default_dur);
                let scaled_duration = Some((base_dur as f32 / speed.max(0.01)).round().max(1.0) as u32);
                mouse_control(
                    MouseActionType::Move,
                    MouseButton::Left,
                    *coord_mode,
                    Some(*x),
                    Some(*y),
                    *smooth,
                    scaled_duration,
                    *organic,
                    None,
                    None,
                    None,
                    false,
                )
                .await?;
            }
            crate::models::MacroEvent::MouseDown { button } => {
                mouse_control(
                    MouseActionType::Down,
                    *button,
                    MouseCoordMode::Current,
                    None, None, false, None, false, None, None, None, false,
                )
                .await?;
            }
            crate::models::MacroEvent::MouseUp { button } => {
                mouse_control(
                    MouseActionType::Up,
                    *button,
                    MouseCoordMode::Current,
                    None, None, false, None, false, None, None, None, false,
                )
                .await?;
            }
            crate::models::MacroEvent::MouseClick { button, clicks, x, y, coord_mode, smooth } => {
                let cm = if x.is_some() || y.is_some() { *coord_mode } else { MouseCoordMode::Current };
                let (disp_x, disp_y) = if let (Some(px), Some(py)) = (x, y) {
                    let jx = random_gaussian(0.0, 1.2).round() as i32;
                    let jy = random_gaussian(0.0, 1.2).round() as i32;
                    (Some(px + jx), Some(py + jy))
                } else {
                    (*x, *y)
                };
                let click_hold = random_gaussian(45.0, 10.0).round().clamp(30.0, 85.0) as u32;
                mouse_control(
                    MouseActionType::Click,
                    *button,
                    cm,
                    disp_x,
                    disp_y,
                    *smooth,
                    Some(200),
                    false,
                    None,
                    Some(*clicks),
                    Some(click_hold),
                    true,
                )
                .await?;
            }
            crate::models::MacroEvent::MouseWheel { delta } => {
                mouse_control(
                    crate::models::MouseActionType::Scroll,
                    crate::models::MouseButton::Left,
                    crate::models::MouseCoordMode::Current,
                    None, None, false, None, false,
                    Some(*delta),
                    None, None, false,
                )
                .await?;
            }
            crate::models::MacroEvent::Delay { ms, organic, jitter_percent } => {
                let eff_ms = (*ms as f32 / speed.max(0.01)).round() as u32;
                let sleep_ms = calculate_sleep_ms(eff_ms, *organic, *jitter_percent, None);
                if sleep_ms > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(sleep_ms)).await;
                }
            }
        }
    }
    Ok(())
}
