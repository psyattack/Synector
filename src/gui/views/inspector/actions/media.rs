//! `media` action field editors.

use super::*;

pub(crate) fn render_play_sound_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    _resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let idx = ctx.idx;
    let is_exit_pipeline = ctx.is_exit_pipeline;
    let Action::PlaySound { file_path, play_beep, beep_frequency, beep_duration_ms } = action else { return; };
    let mode_options = [
        (false, Cell::new(loc.inspector_sound_mode_file, "")),
        (true, Cell::new(loc.inspector_sound_mode_beep, "")),
    ];
    widgets::cells(ui, play_beep, &mode_options);
    ui.add_space(4.0);

    if *play_beep {
        let mut is_custom = beep_frequency.is_some();
        let tone_options = [
            (false, Cell::new(loc.inspector_beep_alert, loc.inspector_beep_alert_tip)),
            (true, Cell::new(loc.inspector_beep_custom, loc.inspector_beep_custom_tip)),
        ];
        if widgets::cells(ui, &mut is_custom, &tone_options) {
            if is_custom {
                *beep_frequency = Some(750);
                *beep_duration_ms = Some(200);
            } else {
                *beep_frequency = None;
                *beep_duration_ms = None;
            }
        }

        if is_custom {
            ui.add_space(4.0);
            let mut freq = beep_frequency.unwrap_or(750);
            let mut dur = beep_duration_ms.unwrap_or(200);

            widgets::hud(ui, loc.inspector_sound_beep_frequency, t.muted);
            if ui.add(egui::Slider::new(&mut freq, 100..=5000).suffix(" Hz")).changed() {
                *beep_frequency = Some(freq);
            }

            ui.add_space(4.0);
            widgets::hud(ui, loc.inspector_sound_beep_duration, t.muted);
            if widgets::duration_millis_u32(ui, loc, ("beep_duration", idx, is_exit_pipeline), &mut dur, 50.0..=3000.0).changed() {
                *beep_duration_ms = Some(dur);
            }
        }
    } else {
        widgets::hud(ui, loc.inspector_field_audio_file_path, t.muted);
        ui.horizontal(|ui| {
            ui.set_width(ui.available_width());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let btn_w = widgets::button_width(ui, loc.settings_btn_browse, 0.0);
                if widgets::button(ui, loc.settings_btn_browse, Kind::Secondary, btn_w).clicked() {
                    if let Some(file) = rfd::FileDialog::new()
                        .add_filter(loc.inspector_dialog_filter_audio, &["wav", "mp3", "wma", "m4a", "ogg"])
                        .pick_file()
                    {
                        *file_path = file.to_string_lossy().to_string();
                    }
                }
                ui.add(
                    egui::TextEdit::singleline(file_path)
                        .hint_text(loc.inspector_hint_sound_file)
                        .desired_width(ui.available_width())
                        .min_size(vec2(0.0, theme::CONTROL_H))
                        .align(egui::Align2::LEFT_CENTER)
                        .margin(vec2(10.0, 8.0)),
                );
            });
        });
    }
}

pub(crate) fn render_audio_control_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let default_picker_target = ctx.default_picker_target;
    let Action::AudioControl { target, process_name, action_type, volume_level, left_volume, right_volume, } = action else { return; };
    widgets::hud(ui, loc.inspector_audio_target_label, t.muted);
    let target_options = [
        (crate::models::AudioTarget::Playback, Cell::new(
            loc.inspector_audio_target_playback,
            loc.inspector_audio_target_playback_tip,
        )),
        (crate::models::AudioTarget::Microphone, Cell::new(
            loc.inspector_audio_target_mic,
            loc.inspector_audio_target_mic_tip,
        )),
        (crate::models::AudioTarget::Process, Cell::new(
            loc.inspector_audio_target_process,
            loc.inspector_audio_target_process_tip,
        )),
    ];
    widgets::cells(ui, target, &target_options);

    if *target == crate::models::AudioTarget::Process {
        ui.add_space(4.0);
        widgets::hud(ui, loc.inspector_field_target_process, t.muted);
        if render_optional_process_name_row(ui, loc, process_name, loc.inspector_hint_action_process) {
            resp.pick_target = Some(default_picker_target);
        }
    }

    ui.add_space(4.0);
    widgets::hud(ui, loc.inspector_field_audio_action_type, t.muted);
    let audio_options = [
        (crate::models::AudioVolumeAction::Mute, Cell::new(
            loc.inspector_cell_mute,
            loc.inspector_audio_mute,
        )),
        (crate::models::AudioVolumeAction::Unmute, Cell::new(
            loc.inspector_cell_unmute,
            loc.inspector_audio_unmute,
        )),
        (crate::models::AudioVolumeAction::ToggleMute, Cell::new(
            loc.inspector_cell_toggle,
            loc.inspector_audio_toggle_mute,
        )),
        (crate::models::AudioVolumeAction::SetVolume, Cell::new(
            loc.inspector_cell_volume,
            loc.inspector_audio_set_volume,
        )),
    ];
    widgets::cells(ui, action_type, &audio_options);

    if *action_type == crate::models::AudioVolumeAction::SetVolume {
        ui.add_space(4.0);
        if *target == crate::models::AudioTarget::Playback {
            let mut separate = left_volume.is_some() || right_volume.is_some();
            if ui.checkbox(&mut separate, loc.inspector_audio_separate_channels).changed() {
                if separate {
                    *left_volume = Some(*volume_level);
                    *right_volume = Some(*volume_level);
                } else {
                    *left_volume = None;
                    *right_volume = None;
                }
            }

            if separate {
                let mut l = left_volume.unwrap_or(*volume_level);
                let mut r = right_volume.unwrap_or(*volume_level);
                ui.add_space(2.0);
                widgets::hud(ui, loc.inspector_audio_left_channel, t.muted);
                if ui.add(egui::Slider::new(&mut l, 0..=100).suffix("%")).changed() {
                    *left_volume = Some(l);
                }
                ui.add_space(2.0);
                widgets::hud(ui, loc.inspector_audio_right_channel, t.muted);
                if ui.add(egui::Slider::new(&mut r, 0..=100).suffix("%")).changed() {
                    *right_volume = Some(r);
                }
            } else {
                widgets::hud(ui, loc.inspector_field_volume_level, t.muted);
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(volume_level, 0..=100).suffix("%"));
                });
            }
        } else {
            widgets::hud(ui, loc.inspector_field_volume_level, t.muted);
            ui.horizontal(|ui| {
                ui.add(egui::Slider::new(volume_level, 0..=100).suffix("%"));
            });
        }
    }
}

pub(crate) fn render_audio_format_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    _resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let Action::AudioFormat { target, device_name: _, sample_rate, bit_depth, channels, } = action else { return; };
    widgets::hud(ui, loc.inspector_audio_target_label, t.muted);
    let target_options = [
        (crate::models::AudioTarget::Playback, Cell::new(loc.inspector_audio_target_playback, loc.inspector_audio_target_playback_tip)),
        (crate::models::AudioTarget::Microphone, Cell::new(loc.inspector_audio_target_mic, loc.inspector_audio_target_mic_tip)),
    ];
    widgets::cells(ui, target, &target_options);

    ui.add_space(4.0);
    widgets::hud(ui, loc.inspector_audio_format_sample_rate, t.muted);
    let rate_options = [
        (44100, Cell::new(loc.inspector_audio_rate_44100, loc.inspector_audio_rate_44100_tip)),
        (48000, Cell::new(loc.inspector_audio_rate_48000, loc.inspector_audio_rate_48000_tip)),
        (96000, Cell::new(loc.inspector_audio_rate_96000, loc.inspector_audio_rate_96000_tip)),
        (192000, Cell::new(loc.inspector_audio_rate_192000, loc.inspector_audio_rate_192000_tip)),
    ];
    widgets::cells(ui, sample_rate, &rate_options);

    ui.add_space(4.0);
    widgets::hud(ui, loc.inspector_audio_format_bit_depth, t.muted);
    let depth_options = [
        (16, Cell::new(loc.inspector_audio_depth_16, loc.inspector_audio_depth_16_tip)),
        (24, Cell::new(loc.inspector_audio_depth_24, loc.inspector_audio_depth_24_tip)),
        (32, Cell::new(loc.inspector_audio_depth_32, loc.inspector_audio_depth_32_tip)),
    ];
    widgets::cells(ui, bit_depth, &depth_options);

    ui.add_space(4.0);
    widgets::hud(ui, loc.inspector_audio_format_channels, t.muted);
    let ch_options = [
        (2, Cell::new(loc.inspector_audio_channels_stereo, loc.inspector_audio_channels_stereo_tip)),
        (1, Cell::new(loc.inspector_audio_channels_mono, loc.inspector_audio_channels_mono_tip)),
    ];
    widgets::cells(ui, channels, &ch_options);
}

pub(crate) fn render_default_device_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    _resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let Action::DefaultDevice { target, device_name } = action else { return; };
    widgets::hud(ui, loc.inspector_audio_target_label, t.muted);
    let dev_target_options = [
        (crate::models::AudioTarget::Playback, Cell::new(
            loc.inspector_audio_dev_playback,
            loc.inspector_audio_dev_playback_tip,
        )),
        (crate::models::AudioTarget::Microphone, Cell::new(
            loc.inspector_audio_dev_mic,
            loc.inspector_audio_dev_mic_tip,
        )),
    ];
    widgets::cells(ui, target, &dev_target_options);

    ui.add_space(4.0);
    let dev_hint = match target {
        crate::models::AudioTarget::Playback => loc.inspector_hint_device_playback,
        crate::models::AudioTarget::Microphone => loc.inspector_hint_device_mic,
        crate::models::AudioTarget::Process => loc.inspector_hint_device_process,
    };
    widgets::hud(ui, loc.inspector_field_device_name, t.muted);
    ui.add(
        egui::TextEdit::singleline(device_name)
            .hint_text(dev_hint)
            .desired_width(ui.available_width())
            .min_size(vec2(0.0, theme::CONTROL_H))
            .align(egui::Align2::LEFT_CENTER)
            .margin(vec2(10.0, 8.0)),
    );
}
