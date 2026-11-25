use chrono::{DateTime, Local, TimeZone};

use super::serde_defaults::default_accent_color;
use super::*;

#[test]
fn test_generate_clone_name() {
    assert_eq!(generate_clone_name("Discord Mute"), "Discord Mute (Copy)");
    assert_eq!(generate_clone_name("Discord Mute (Copy)"), "Discord Mute (Copy 2)");
    assert_eq!(generate_clone_name("Discord Mute (Copy 2)"), "Discord Mute (Copy 3)");
    assert_eq!(generate_clone_name("Discord Mute (Copy 99)"), "Discord Mute (Copy 100)");
}

#[test]
fn test_rule_clone_with_new_id() {
    let original = Rule {
        id: "rule_orig_123".into(),
        name: "Game Optimizer".into(),
        description: "Some memo".into(),
        is_enabled: true,
        cooldown_seconds: 1.5,
        max_executions: Some(10),
        quota_period: Some(QuotaPeriod::Day),
        execution_count: 42,
        stop_on_error: true,
        trigger: Some(Trigger::Process {
            process_name: "game.exe".into(),
            match_mode: ProcessMatchMode::Exact,
            single_instance: true,
            event: crate::models::ProcessTriggerEvent::Started,
        }),
        actions: vec![Action::Notification {
            title: "Alert".into(),
            message: "Starting".into(),
        }],
        custom_icon_path: Some("C:\\game.exe".into()),
        auto_process_icon: true,
        conditions: vec![],
        last_executed_at: None,
        quota_window_start: Some("2026-11-19 08:00:00".into()),
        tags: vec![RuleTag::new("Gaming").with_color("#3B82F6").with_icon("🎮")],
    };

    let cloned = original.clone_with_new_id();
    assert_ne!(cloned.id, original.id);
    assert_eq!(cloned.name, "Game Optimizer (Copy)");
    assert_eq!(cloned.description, "Some memo");
    assert_eq!(cloned.tags, original.tags);
    assert!(!cloned.is_enabled); // Paused on clone
    assert_eq!(cloned.cooldown_seconds, 1.5);
    assert_eq!(cloned.max_executions, Some(10));
    assert_eq!(cloned.quota_period, Some(QuotaPeriod::Day));
    assert_eq!(cloned.trigger, original.trigger);
    assert_eq!(cloned.actions, original.actions);
    assert_eq!(cloned.execution_count, 0); // reset to 0
    assert!(cloned.quota_window_start.is_none()); // runtime anchor reset
    assert_eq!(cloned.custom_icon_path, Some("C:\\game.exe".into()));
    assert!(cloned.auto_process_icon);
    assert!(cloned.stop_on_error);
}

#[test]
fn test_action_duplicate_with_fresh_ids() {
    let overlay_act = Action::Overlay {
        overlay_id: "overlay_initial_123".to_string(),
        custom_overlay: Some(Box::new(OverlayDefinition {
            id: "overlay_initial_123".to_string(),
            ..Default::default()
        })),
        operation: OverlayOperation::Toggle,
        override_text: None,
        duration_ms: None,
        close_on_exit: false,
    };

    let duplicated = overlay_act.duplicate_with_fresh_ids();
    if let Action::Overlay { overlay_id, custom_overlay, .. } = duplicated {
        assert_ne!(overlay_id, "overlay_initial_123");
        assert_eq!(custom_overlay.as_ref().map(|o| o.id.as_str()), Some(overlay_id.as_str()));
    } else {
        panic!("Expected Action::Overlay");
    }
}

#[test]
fn test_new_actions_and_conditions_serde() {
    let rule = Rule {
        id: "rule_advanced".into(),
        name: "Industrial Workstation Setup".into(),
        description: "Applies windows, tuning and scripts".into(),
        is_enabled: true,
        cooldown_seconds: 1.0,
        max_executions: Some(5),
        quota_period: Some(QuotaPeriod::Week),
        execution_count: 2,
        stop_on_error: true,
        trigger: Some(Trigger::Webhook {
            endpoint_path: "studio_start".into(),
        }),
        actions: vec![
            Action::Window {
                target_process: Some("Code.exe".into()),
                window_action: WindowActionType::Maximize,
                x: None,
                y: None,
                width: None,
                height: None,
                transparency: Some(240),
            },
            Action::ProcessTuning {
                target_process: Some("Code.exe".into()),
                priority: Some(ProcessPriorityLevel::High),
                affinity_mask: Some(0xFF),
                trim_memory: true,
            },
            Action::Macro {
                preset_id: None,
                operation: MacroOperation::Run,
                events: vec![
                    MacroEvent::KeyDown { key: "Shift".into() },
                    MacroEvent::KeyPress { key: "P".into() },
                    MacroEvent::KeyUp { key: "Shift".into() },
                ],
                repeat: 1,
                speed: 1.0,
                infinite: false,
            },
            Action::PowerPlan {
                plan: PowerPlanType::HighPerformance,
            },
            Action::PlaySound {
                file_path: "C:\\Windows\\Media\\chimes.wav".into(),
                play_beep: false,
                beep_frequency: None,
                beep_duration_ms: None,
            },
            Action::ExecuteScript {
                interpreter: ScriptType::Lua,
                script: "syn.log('info', 'Workstation setup complete!')".into(),
                run_in_window: false,
            },
            Action::Firewall {
                process_name: "telemetry.exe".into(),
                action: FirewallAction::Block,
            },
            Action::Display {
                width: Some(1920),
                height: Some(1080),
                refresh_rate: Some(144),
                monitor_index: Some(1),
            },
            Action::DefaultDevice {
                target: AudioTarget::Playback,
                device_name: "Headphones".into(),
            },
            Action::DefaultDevice {
                target: AudioTarget::Microphone,
                device_name: "Microphone".into(),
            },
            Action::IfElse {
                conditions: vec![RuleConditionItem::new(RuleCondition::ProcessRunning {
                    process_name: "game.exe".into(),
                    match_mode: ProcessMatchMode::Exact,
                    must_be_running: true,
                })],
                then_actions: vec![Action::Wait {
                    mode: WaitMode::Timer,
                    seconds: 1.0,
                    condition: None,
                    timeout_sec: 30.0,
                    timeout_policy: WaitTimeoutPolicy::Abort,
                }],
                else_if_branches: vec![ElseIfBranch {
                    conditions: vec![RuleConditionItem::new(RuleCondition::Event {
                        event_type: SystemEventType::PowerSourceAc,
                        idle_duration_sec: 300,
                    })],
                    actions: vec![Action::Wait {
                        mode: WaitMode::Timer,
                        seconds: 2.0,
                        condition: None,
                        timeout_sec: 30.0,
                        timeout_policy: WaitTimeoutPolicy::Abort,
                    }],
                }],
                else_actions: vec![],
            },
        ],
        custom_icon_path: None,
        auto_process_icon: true,
        conditions: vec![
            RuleConditionItem::new(RuleCondition::WindowFocused {
                process_name: "game.exe".into(),
                match_mode: ProcessMatchMode::Exact,
                must_be_focused: true,
            }),
            RuleConditionItem::with_operator(
                ConditionOperator::Or,
                RuleCondition::FileWatcher {
                    path: "C:\\log.txt".into(),
                    filter_extension: None,
                    must_exist: true,
                },
            ),
        ],
        tags: vec![],
        last_executed_at: None,
        quota_window_start: None,
    };

    let json = serde_json::to_string(&rule).expect("Serialization must succeed");
    let decoded: Rule = serde_json::from_str(&json).expect("Deserialization must succeed");

    assert_eq!(decoded.id, rule.id);
    assert_eq!(decoded.max_executions, Some(5));
    assert_eq!(decoded.quota_period, Some(QuotaPeriod::Week));
    assert_eq!(decoded.execution_count, 2);
    assert!(decoded.stop_on_error);
    assert_eq!(decoded.actions.len(), 11);
    assert_eq!(decoded.conditions.len(), 2);
    assert_eq!(decoded, rule);
}

#[test]
fn test_unified_actions_serde() {
    let unified_actions = vec![
        Action::AudioControl {
            target: AudioTarget::Playback,
            process_name: None,
            action_type: AudioVolumeAction::SetVolume,
            volume_level: 75,
            left_volume: Some(70),
            right_volume: Some(80),
        },
        Action::AudioFormat {
            target: AudioTarget::Playback,
            device_name: None,
            sample_rate: 48000,
            bit_depth: 24,
            channels: 2,
        },
        Action::DefaultDevice {
            target: AudioTarget::Microphone,
            device_name: "USB Mic".into(),
        },
        Action::ExecuteScript {
            interpreter: ScriptType::PowerShell,
            script: "Get-Process".into(),
            run_in_window: true,
        },
        Action::Macro {
            preset_id: None,
            operation: MacroOperation::Run,
            events: vec![
                MacroEvent::KeyDown { key: "Shift".into() },
                MacroEvent::KeyPress { key: "A".into() },
                MacroEvent::KeyUp { key: "Shift".into() },
                MacroEvent::Text { text: "Hello".into(), delay_ms: 20, organic: false },
                MacroEvent::MouseMove {
                    x: 100,
                    y: 200,
                    smooth: true,
                    duration_ms: Some(150),
                    organic: true,
                    coord_mode: MouseCoordMode::Absolute,
                },
                MacroEvent::MouseClick {
                    button: MouseButton::Left,
                    clicks: 2,
                    x: None,
                    y: None,
                    coord_mode: MouseCoordMode::Absolute,
                    smooth: false,
                },
                MacroEvent::Delay {
                    ms: 250,
                    organic: true,
                    jitter_percent: 30,
                },
            ],
            repeat: 3,
            speed: 1.5,
            infinite: false,
        },
    ];

    let json = serde_json::to_string(&unified_actions).expect("Must serialize");
    let decoded: Vec<Action> = serde_json::from_str(&json).expect("Must deserialize");
    assert_eq!(decoded, unified_actions);
}

#[test]
fn test_new_v23_actions_serde() {
    let actions = vec![
        Action::TrimWorkingSet {
            target_process: Some("game.exe".to_string()),
            soft_limit_mb: Some(256),
            skip_if_focused: true,
        },
        Action::DiscordRpcSet {
            app_id: "123456789".to_string(),
            details: "Playing Game".to_string(),
            state: "In Mission".to_string(),
            large_image: Some("logo".to_string()),
            large_text: Some("Game Logo".to_string()),
            small_image: Some("status".to_string()),
            small_text: Some("Online".to_string()),
            show_elapsed: true,
            buttons: vec![DiscordRpcButton {
                label: "Join".to_string(),
                url: "https://example.com".to_string(),
            }],
            auto_clear_on_process_exit: true,
        },
        Action::DiscordRpcClear,
        Action::Archive {
            operation: ArchiveOperation::Pack,
            format: ArchiveFormat::Zip,
            source_path: "C:\\source".to_string(),
            destination_path: "C:\\out.zip".to_string(),
            compression_level: Some(6),
            overwrite: true,
        },
        Action::FileSystem {
            operation: FileSystemOperation::CreateFile,
            path: "C:\\file.txt".to_string(),
            target_path: None,
            content: Some("text".to_string()),
            create_parents: true,
            overwrite: true,
        },
    ];

    let json = serde_json::to_string(&actions).expect("Must serialize");
    let decoded: Vec<Action> = serde_json::from_str(&json).expect("Must deserialize");
    assert_eq!(decoded, actions);
}

#[test]
fn test_trim_working_set_serde_options_and_backcompat() {
    // Backwards compatibility: legacy JSON without soft_limit_mb and skip_if_focused
    let legacy_json = r#"{"$action_type":"trim_working_set","target_process":"notepad.exe"}"#;
    let action: Action = serde_json::from_str(legacy_json).expect("Must deserialize legacy format");
    assert_eq!(
        action,
        Action::TrimWorkingSet {
            target_process: Some("notepad.exe".to_string()),
            soft_limit_mb: None,
            skip_if_focused: false,
        }
    );

    // Serialization with None soft_limit_mb omits it
    let serialized = serde_json::to_string(&action).expect("Must serialize");
    assert!(!serialized.contains("soft_limit_mb"));
    assert!(serialized.contains(r#""skip_if_focused":false"#));

    // Full roundtrip with soft_limit_mb and skip_if_focused: true
    let full_action = Action::TrimWorkingSet {
        target_process: Some("game.exe".to_string()),
        soft_limit_mb: Some(512),
        skip_if_focused: true,
    };
    let full_json = serde_json::to_string(&full_action).expect("Must serialize");
    assert!(full_json.contains(r#""soft_limit_mb":512"#));
    assert!(full_json.contains(r#""skip_if_focused":true"#));
    let full_decoded: Action = serde_json::from_str(&full_json).expect("Must deserialize");
    assert_eq!(full_decoded, full_action);
}

#[test]
fn test_manual_trigger_serde() {
    let trigger = Trigger::Manual;
    let serialized = serde_json::to_string(&trigger).unwrap();
    assert_eq!(serialized, r#"{"$trigger_type":"manual"}"#);
    let deserialized: Trigger = serde_json::from_str(&serialized).unwrap();
    assert_eq!(deserialized, Trigger::Manual);
}

#[test]
fn test_global_hotkey_hold_mode_and_repeat_serde() {
    // Test optional field default: json without repeat_while_held defaults to false
    let json = r#"{"$trigger_type":"hotkey","key_combination":"Ctrl+Shift+H","hold_mode":true}"#;
    let trigger: Trigger = serde_json::from_str(json).unwrap();
    assert_eq!(
        trigger,
        Trigger::GlobalHotkey {
            key_combination: "Ctrl+Shift+H".to_string(),
            hold_mode: true,
            repeat_while_held: false,
        }
    );

    // Test roundtrip with repeat_while_held: true
    let trigger_loop = Trigger::GlobalHotkey {
        key_combination: "M4".to_string(),
        hold_mode: true,
        repeat_while_held: true,
    };
    let serialized = serde_json::to_string(&trigger_loop).unwrap();
    let deserialized: Trigger = serde_json::from_str(&serialized).unwrap();
    assert_eq!(trigger_loop, deserialized);
}

#[test]
fn test_precondition_resource_and_event_serde() {
    // 1. ResourceThreshold roundtrip
    let res_cond = RuleCondition::ResourceThreshold {
        process_name: "game.exe".to_string(),
        cpu_percent: Some(75.5),
        ram_mb: Some(2048),
    };
    let serialized = serde_json::to_string(&res_cond).unwrap();
    let deserialized: RuleCondition = serde_json::from_str(&serialized).unwrap();
    assert_eq!(res_cond, deserialized);

    // 2. Event condition roundtrip
    let event_cond = RuleCondition::Event {
        event_type: SystemEventType::SessionLock,
        idle_duration_sec: 300,
    };
    let serialized = serde_json::to_string(&event_cond).unwrap();
    let deserialized: RuleCondition = serde_json::from_str(&serialized).unwrap();
    assert_eq!(event_cond, deserialized);

    // 3. FileWatcher condition roundtrip
    let file_cond = RuleCondition::FileWatcher {
        path: "C:\\test.log".to_string(),
        filter_extension: Some("log".to_string()),
        must_exist: true,
    };
    let serialized = serde_json::to_string(&file_cond).unwrap();
    let deserialized: RuleCondition = serde_json::from_str(&serialized).unwrap();
    assert_eq!(file_cond, deserialized);
}

#[test]
fn test_rule_activity_condition_and_wait_serde() {
    let condition = RuleCondition::RuleRunning {
        rule_id: "rule_game_mode".to_string(),
        must_be_running: false,
    };
    let json = serde_json::to_string(&condition).unwrap();
    assert!(json.contains("\"$condition_type\":\"rule_running\""), "{json}");
    let decoded: RuleCondition = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, condition);

    // Omitted must_be_running defaults to true, matching the other state conditions.
    let defaulted: RuleCondition =
        serde_json::from_str(r#"{"$condition_type":"rule_running","rule_id":"rule_a"}"#).unwrap();
    assert_eq!(
        defaulted,
        RuleCondition::RuleRunning {
            rule_id: "rule_a".to_string(),
            must_be_running: true,
        }
    );

    for wait in [
        WaitCondition::RuleStarted {
            rule_id: "rule_game_mode".into(),
        },
        WaitCondition::RuleStopped {
            rule_id: "rule_game_mode".into(),
        },
    ] {
        let action = Action::Wait {
            mode: WaitMode::UntilCondition,
            seconds: 0.0,
            condition: Some(wait),
            timeout_sec: 30.0,
            timeout_policy: WaitTimeoutPolicy::Abort,
        };
        let json = serde_json::to_string(&action).unwrap();
        let decoded: Action = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, action);
    }
}

#[test]
fn test_overlay_models_and_action_serde() {
    let action = Action::Overlay {
        overlay_id: "overlay_crosshair".to_string(),
        custom_overlay: None,
        operation: OverlayOperation::Toggle,
        override_text: Some("Aim Active".to_string()),
        duration_ms: Some(3000),
        close_on_exit: true,
    };
    let json = serde_json::to_string(&action).expect("Serialize Action::Overlay");
    let decoded: Action = serde_json::from_str(&json).expect("Deserialize Action::Overlay");
    assert_eq!(decoded, action);

    // Test with custom embedded overlay
    let custom_action = Action::Overlay {
        overlay_id: "custom_rule_hud".to_string(),
        custom_overlay: Some(Box::new(OverlayDefinition::default_crosshair())),
        operation: OverlayOperation::Show,
        override_text: None,
        duration_ms: None,
        close_on_exit: false,
    };
    let custom_json = serde_json::to_string(&custom_action).unwrap();
    let custom_decoded: Action = serde_json::from_str(&custom_json).unwrap();
    assert_eq!(custom_decoded, custom_action);

    let mut overlay = OverlayDefinition::default_crosshair();
    overlay.monitor_index = Some(1);
    overlay.close_with_target = true;
    let json_ov = serde_json::to_string(&overlay).expect("Serialize OverlayDefinition");
    let decoded_ov: OverlayDefinition = serde_json::from_str(&json_ov).expect("Deserialize OverlayDefinition");
    assert_eq!(decoded_ov, overlay);
    assert_eq!(decoded_ov.monitor_index, Some(1));
    assert!(decoded_ov.close_with_target);

    // Test MonitorInfo orientation
    let mon_landscape = MonitorInfo {
        index: 0,
        name: "Display 1".to_string(),
        x: 0,
        y: 0,
        width: 2560,
        height: 1440,
        is_primary: true,
    };
    assert_eq!(mon_landscape.orientation(), MonitorOrientation::Landscape);
    assert!(!mon_landscape.is_portrait());

    let mon_portrait = MonitorInfo {
        index: 1,
        name: "Display 2".to_string(),
        x: -1080,
        y: 0,
        width: 1080,
        height: 1920,
        is_primary: false,
    };
    assert_eq!(mon_portrait.orientation(), MonitorOrientation::Portrait);
    assert!(mon_portrait.is_portrait());

    let macro_ov = OverlayDefinition::default_macro_badge();
    let json_macro = serde_json::to_string(&macro_ov).unwrap();
    let decoded_macro: OverlayDefinition = serde_json::from_str(&json_macro).unwrap();
    assert_eq!(decoded_macro, macro_ov);

    // Test Image element defaults: lock_aspect defaults to true
    let raw_img_json = r#"{"type": "image", "path": "test.png"}"#;
    let img_elem: OverlayElement = serde_json::from_str(raw_img_json).unwrap();
    if let OverlayElement::Image { lock_aspect, .. } = img_elem {
        assert!(lock_aspect, "lock_aspect must default to true");
    } else {
        panic!("Expected Image element");
    }

    // Test fade_in_ms and fade_out_ms serde
    let mut fade_overlay = OverlayDefinition::default_crosshair();
    fade_overlay.fade_in_ms = Some(250);
    fade_overlay.fade_out_ms = Some(350);
    let fade_json = serde_json::to_string(&fade_overlay).unwrap();
    assert!(fade_json.contains("\"fade_in_ms\":250"));
    assert!(fade_json.contains("\"fade_out_ms\":350"));
    let decoded_fade: OverlayDefinition = serde_json::from_str(&fade_json).unwrap();
    assert_eq!(decoded_fade.fade_in_ms, Some(250));
    assert_eq!(decoded_fade.fade_out_ms, Some(350));

    // When omitted, both must default to None and click_through to false
    let raw_no_fade = r#"{"id":"test","name":"Test"}"#;
    let decoded_no_fade: OverlayDefinition = serde_json::from_str(raw_no_fade).unwrap();
    assert_eq!(decoded_no_fade.fade_in_ms, None);
    assert_eq!(decoded_no_fade.fade_out_ms, None);
    assert!(!decoded_no_fade.click_through, "click_through must default to false");
    assert!(!OverlayDefinition::default_new_overlay().click_through, "new overlay click_through must be false by default");
}

#[test]
fn test_overlay_interactive_and_telemetry_elements() {
    let elements = vec![
        OverlayElement::Graph {
            id: "g1".to_string(),
            metric: GraphMetric::SystemCpu,
            style: GraphStyle::AreaFilled,
            sample_count: 50,
            color: [0, 255, 128, 255],
            fill_color: [0, 255, 128, 60],
            bg_color: [20, 20, 30, 200],
            show_current_val: true,
            x: 10.0,
            y: 20.0,
            width: 140.0,
            height: 60.0,
            rotation: 0.0,
        },
        OverlayElement::ClockTimer {
            id: "c1".to_string(),
            mode: TimerMode::Countdown,
            format_str: "%H:%M".to_string(),
            countdown_total_ms: 30000,
            countdown_remaining_ms: 25000,
            stopwatch_elapsed_ms: 0,
            is_running: true,
            font_size: 16.0,
            color: [255, 255, 255, 255],
            bg_color: [0, 0, 0, 180],
            bold: true,
            font_family: "Segoe UI".to_string(),
            x: 15.0,
            y: 25.0,
            width: 100.0,
            height: 30.0,
            rotation: 45.0,
            on_finish: Some(OverlayClickAction::CloseOverlay),
        },
        OverlayElement::InteractiveControl {
            id: "ctrl1".to_string(),
            control: InteractiveControlKind::Button {
                label: "Mute".to_string(),
                icon: None,
                action: Some(OverlayClickAction::ToggleMute),
            },
            color: [255, 255, 255, 255],
            bg_color: [40, 45, 55, 220],
            active_color: [0, 200, 100, 255],
            x: 5.0,
            y: 5.0,
            width: 80.0,
            height: 28.0,
            rotation: 0.0,
        },
        OverlayElement::SystemTelemetry {
            id: "tel1".to_string(),
            kind: TelemetryKind::Battery,
            style: TelemetryStyle::Badge,
            color: [0, 220, 130, 255],
            bg_color: [25, 30, 40, 220],
            x: 50.0,
            y: 60.0,
            width: 90.0,
            height: 24.0,
            rotation: 0.0,
            on_click: None,
        },
        OverlayElement::MediaWidget {
            id: "m1".to_string(),
            show_controls: true,
            compact: false,
            color: [240, 240, 250, 255],
            bg_color: [20, 24, 32, 200],
            x: 0.0,
            y: 0.0,
            width: 180.0,
            height: 52.0,
            rotation: 0.0,
        },
        OverlayElement::KeyVisualizer {
            id: "k1".to_string(),
            layout: KeyVisualizerLayout::WasdMouse,
            max_keys: 5,
            decay_seconds: 2.0,
            show_modifiers_only: true,
            color: [255, 255, 255, 255],
            bg_color: [35, 40, 50, 220],
            active_color: [0, 180, 255, 255],
            x: 100.0,
            y: 100.0,
            width: 170.0,
            height: 105.0,
            rotation: 0.0,
        },
    ];

    for mut elem in elements {
        assert!(!elem.id().is_empty());
        assert_eq!(elem.x(), elem.x());
        assert_eq!(elem.y(), elem.y());
        *elem.id_mut() = format!("{}_cloned", elem.id());
        assert!(elem.id().ends_with("_cloned"));

        let json = serde_json::to_string(&elem).expect("Serialize element");
        let decoded: OverlayElement = serde_json::from_str(&json).expect("Deserialize element");
        assert_eq!(decoded.id(), elem.id());
        assert_eq!(decoded.x(), elem.x());
        assert_eq!(decoded.y(), elem.y());
        assert_eq!(decoded.rotation(), elem.rotation());
    }
}

#[test]
fn test_key_visualizer_layouts_and_backcompat() {
    let layouts = [
        KeyVisualizerLayout::WasdMouse,
        KeyVisualizerLayout::WasdOnly,
        KeyVisualizerLayout::ActionRpg,
        KeyVisualizerLayout::MouseOnly,
        KeyVisualizerLayout::TextStream,
    ];
    for layout in layouts {
        let elem = OverlayElement::KeyVisualizer {
            id: "k".into(),
            layout,
            max_keys: 4,
            decay_seconds: 1.5,
            show_modifiers_only: false,
            color: [255, 255, 255, 255],
            bg_color: [0, 0, 0, 200],
            active_color: [0, 200, 255, 255],
            x: 10.0,
            y: 10.0,
            width: 170.0,
            height: 105.0,
            rotation: 0.0,
        };
        let json = serde_json::to_string(&elem).unwrap();
        let decoded: OverlayElement = serde_json::from_str(&json).unwrap();
        if let OverlayElement::KeyVisualizer { layout: dec_layout, .. } = decoded {
            assert_eq!(dec_layout, layout);
        } else {
            panic!("Expected KeyVisualizer");
        }
    }

    // Backward compatibility: legacy JSON without layout and active_color
    let legacy_json = r#"{"type":"key_visualizer","id":"legacy","max_keys":3,"decay_seconds":1.0,"show_modifiers_only":false,"color":[255,255,255,255],"bg_color":[0,0,0,200],"x":0.0,"y":0.0,"width":160.0,"height":32.0,"rotation":0.0}"#;
    let decoded: OverlayElement = serde_json::from_str(legacy_json).unwrap();
    if let OverlayElement::KeyVisualizer { layout, active_color, .. } = decoded {
        assert_eq!(layout, KeyVisualizerLayout::WasdMouse);
        assert_eq!(active_color, default_accent_color());
    } else {
        panic!("Expected KeyVisualizer");
    }
}

#[test]
fn test_wait_check_toggle_serde() {
    let wait_timer = Action::Wait {
        mode: WaitMode::Timer,
        seconds: 3.5,
        condition: None,
        timeout_sec: 10.0,
        timeout_policy: WaitTimeoutPolicy::Continue,
    };
    let json_timer = serde_json::to_string(&wait_timer).unwrap();
    let dec_timer: Action = serde_json::from_str(&json_timer).unwrap();
    assert_eq!(dec_timer, wait_timer);

    let wait_until = Action::Wait {
        mode: WaitMode::UntilCondition,
        seconds: 0.0,
        condition: Some(WaitCondition::WindowOpened {
            process_name: "notepad.exe".into(),
            match_mode: ProcessMatchMode::Exact,
        }),
        timeout_sec: 45.0,
        timeout_policy: WaitTimeoutPolicy::Abort,
    };
    let json_until = serde_json::to_string(&wait_until).unwrap();
    let dec_until: Action = serde_json::from_str(&json_until).unwrap();
    assert_eq!(dec_until, wait_until);

    let check_act = Action::Check {
        condition: RuleCondition::ProcessRunning {
            process_name: "game.exe".into(),
            match_mode: ProcessMatchMode::Exact,
            must_be_running: true,
        },
    };
    let json_check = serde_json::to_string(&check_act).unwrap();
    let dec_check: Action = serde_json::from_str(&json_check).unwrap();
    assert_eq!(dec_check, check_act);

    let fw_toggle = Action::Firewall {
        process_name: "app.exe".into(),
        action: FirewallAction::Toggle,
    };
    let json_fw = serde_json::to_string(&fw_toggle).unwrap();
    let dec_fw: Action = serde_json::from_str(&json_fw).unwrap();
    assert_eq!(dec_fw, fw_toggle);

    let win_min_restore = Action::Window {
        target_process: Some("app.exe".into()),
        window_action: WindowActionType::ToggleMinimizeRestore,
        x: None,
        y: None,
        width: None,
        height: None,
        transparency: None,
    };
    let json_wmr = serde_json::to_string(&win_min_restore).unwrap();
    let dec_wmr: Action = serde_json::from_str(&json_wmr).unwrap();
    assert_eq!(dec_wmr, win_min_restore);

    let win_show_hide = Action::Window {
        target_process: Some("app.exe".into()),
        window_action: WindowActionType::ToggleShowHide,
        x: None,
        y: None,
        width: None,
        height: None,
        transparency: None,
    };
    let json_wsh = serde_json::to_string(&win_show_hide).unwrap();
    let dec_wsh: Action = serde_json::from_str(&json_wsh).unwrap();
    assert_eq!(dec_wsh, win_show_hide);
}

#[test]
fn test_macro_presets_and_operations_serde() {
    let preset = MacroPreset {
        id: "preset_1".into(),
        name: "Test Macro".into(),
        events: vec![
            MacroEvent::KeyDown { key: "A".into() },
            MacroEvent::KeyUp { key: "A".into() },
        ],
        repeat: 3,
        speed: 1.5,
        infinite: false,
    };
    let preset_json = serde_json::to_string(&preset).unwrap();
    let dec_preset: MacroPreset = serde_json::from_str(&preset_json).unwrap();
    assert_eq!(dec_preset.id, "preset_1");
    assert_eq!(dec_preset.name, "Test Macro");
    assert_eq!(dec_preset.events.len(), 2);
    assert_eq!(dec_preset.repeat, 3);
    assert_eq!(dec_preset.speed, 1.5);
    assert!(!dec_preset.infinite);

    let macro_run_action = Action::Macro {
        events: vec![],
        repeat: 1,
        speed: 1.0,
        infinite: false,
        preset_id: Some("preset_1".into()),
        operation: MacroOperation::Run,
    };
    let json_run = serde_json::to_string(&macro_run_action).unwrap();
    let dec_run: Action = serde_json::from_str(&json_run).unwrap();
    assert_eq!(dec_run, macro_run_action);

    let macro_stop_action = Action::Macro {
        events: vec![],
        repeat: 1,
        speed: 1.0,
        infinite: false,
        preset_id: Some("preset_1".into()),
        operation: MacroOperation::Stop,
    };
    let json_stop = serde_json::to_string(&macro_stop_action).unwrap();
    let dec_stop: Action = serde_json::from_str(&json_stop).unwrap();
    assert_eq!(dec_stop, macro_stop_action);

    let macro_toggle_action = Action::Macro {
        events: vec![],
        repeat: 1,
        speed: 1.0,
        infinite: false,
        preset_id: Some("preset_1".into()),
        operation: MacroOperation::Toggle,
    };
    let json_toggle = serde_json::to_string(&macro_toggle_action).unwrap();
    let dec_toggle: Action = serde_json::from_str(&json_toggle).unwrap();
    assert_eq!(dec_toggle, macro_toggle_action);

    let overlay_set_text = Action::Overlay {
        overlay_id: "overlay_1".into(),
        operation: OverlayOperation::SetText,
        custom_overlay: None,
        override_text: Some("Dynamic notification text".into()),
        duration_ms: None,
        close_on_exit: false,
    };
    let json_set_text = serde_json::to_string(&overlay_set_text).unwrap();
    let dec_set_text: Action = serde_json::from_str(&json_set_text).unwrap();
    assert_eq!(dec_set_text, overlay_set_text);

    // Verify refresh_overlay_ids preserves preset reference (custom_overlay == None)
    let mut cloned_action = overlay_set_text.clone();
    cloned_action.refresh_overlay_ids();
    assert_eq!(cloned_action, overlay_set_text);
}

fn local_at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Local> {
    Local
        .with_ymd_and_hms(year, month, day, hour, minute, 0)
        .single()
        .expect("valid local time")
}

#[test]
fn test_calendar_quota_periods_roll_over() {
    // Minute: 14:59:xx belongs to the previous bucket, 15:00 starts a new one.
    let mut minutely = Rule {
        max_executions: Some(2),
        quota_period: Some(QuotaPeriod::Minute),
        execution_count: 2,
        last_executed_at: Some("2026-11-19 14:59:10".into()),
        ..Default::default()
    };
    assert!(minutely.quota_exhausted(local_at(2026, 11, 19, 14, 59)));
    assert_eq!(
        minutely.quota_next_reset(local_at(2026, 11, 19, 14, 59)),
        Some(local_at(2026, 11, 19, 15, 0))
    );
    assert!(!minutely.quota_exhausted(local_at(2026, 11, 19, 15, 0)));
    assert!(minutely.normalize_quota(local_at(2026, 11, 19, 15, 0)));
    assert_eq!(minutely.execution_count, 0);

    // Hour: 14:59 belongs to the previous bucket, 15:00 starts a new one.
    let mut hourly = Rule {
        max_executions: Some(2),
        quota_period: Some(QuotaPeriod::Hour),
        execution_count: 2,
        last_executed_at: Some("2026-11-19 14:10:00".into()),
        ..Default::default()
    };
    assert!(hourly.quota_exhausted(local_at(2026, 11, 19, 14, 59)));
    assert!(!hourly.quota_exhausted(local_at(2026, 11, 19, 15, 0)));
    assert!(hourly.normalize_quota(local_at(2026, 11, 19, 15, 0)));
    assert_eq!(hourly.execution_count, 0);

    // Day: local midnight is the boundary.
    let mut daily = Rule {
        max_executions: Some(3),
        quota_period: Some(QuotaPeriod::Day),
        execution_count: 3,
        last_executed_at: Some("2026-11-18 22:00:00".into()),
        ..Default::default()
    };
    assert!(daily.quota_exhausted(local_at(2026, 11, 18, 23, 59)));
    assert_eq!(daily.quota_used(local_at(2026, 11, 19, 0, 1)), 0);
    assert!(daily.normalize_quota(local_at(2026, 11, 19, 0, 1)));
    // The last-run stamp stays: daily and monthly schedules deduplicate on it.
    assert_eq!(daily.last_executed_at.as_deref(), Some("2026-11-18 22:00:00"));

    // Week: Monday 00:00 starts a new ISO week (2026-11-16 is a Monday).
    let weekly = Rule {
        max_executions: Some(1),
        quota_period: Some(QuotaPeriod::Week),
        execution_count: 1,
        last_executed_at: Some("2026-11-16 09:00:00".into()),
        ..Default::default()
    };
    assert!(weekly.quota_exhausted(local_at(2026, 11, 22, 23, 0)));
    assert!(!weekly.quota_exhausted(local_at(2026, 11, 23, 0, 1)));

    // Month: the 1st at 00:00 is the boundary.
    let monthly = Rule {
        max_executions: Some(1),
        quota_period: Some(QuotaPeriod::Month),
        execution_count: 1,
        last_executed_at: Some("2026-11-30 23:00:00".into()),
        ..Default::default()
    };
    assert!(monthly.quota_exhausted(local_at(2026, 11, 30, 23, 59)));
    assert!(!monthly.quota_exhausted(local_at(2026, 12, 1, 0, 0)));
}

#[test]
fn test_custom_quota_window_anchors_at_first_execution() {
    let mut rule = Rule {
        max_executions: Some(2),
        quota_period: Some(QuotaPeriod::Custom { value: 90, unit: QuotaUnit::Minutes }),
        ..Default::default()
    };
    let start = local_at(2026, 11, 19, 10, 0);
    assert!(rule.begin_quota_window(start));
    assert_eq!(rule.quota_window_start.as_deref(), Some("2026-11-19 10:00:00"));
    rule.execution_count = 2;
    rule.last_executed_at = Some("2026-11-19 10:30:00".into());

    assert!(rule.quota_exhausted(local_at(2026, 11, 19, 11, 0)));
    assert_eq!(
        rule.quota_next_reset(local_at(2026, 11, 19, 11, 0)),
        Some(local_at(2026, 11, 19, 11, 30))
    );
    assert!(rule.quota_exhausted(local_at(2026, 11, 19, 11, 29)));
    // The window lapses exactly on its end, and the counter rolls over lazily.
    assert!(!rule.quota_exhausted(local_at(2026, 11, 19, 11, 30)));
    assert!(rule.normalize_quota(local_at(2026, 11, 19, 11, 30)));
    assert_eq!(rule.execution_count, 0);
    assert!(rule.quota_window_start.is_none());

    // The next execution opens a fresh window instead of inheriting the old one.
    let next = local_at(2026, 11, 19, 13, 0);
    assert!(rule.begin_quota_window(next));
    assert_eq!(rule.quota_window_start.as_deref(), Some("2026-11-19 13:00:00"));
    assert_eq!(rule.quota_remaining(next), Some(2));
}

#[test]
fn test_custom_quota_window_supports_calendar_months() {
    let mut rule = Rule {
        max_executions: Some(1),
        quota_period: Some(QuotaPeriod::Custom { value: 1, unit: QuotaUnit::Months }),
        ..Default::default()
    };
    let start = local_at(2026, 11, 19, 10, 0);
    assert!(rule.begin_quota_window(start));
    rule.execution_count = 1;
    rule.last_executed_at = Some("2026-11-19 10:05:00".into());

    // One calendar month from the window start, not a fixed 30-day span.
    assert_eq!(
        rule.quota_next_reset(local_at(2026, 12, 1, 0, 0)),
        Some(local_at(2026, 12, 19, 10, 0))
    );
    assert!(rule.quota_exhausted(local_at(2026, 12, 18, 23, 0)));
    assert!(!rule.quota_exhausted(local_at(2026, 12, 19, 10, 0)));

    // Month-end clamping: Jan 31 + 1 month lands on the last day of February.
    let mut clamped = Rule {
        quota_period: Some(QuotaPeriod::Custom { value: 1, unit: QuotaUnit::Months }),
        ..Default::default()
    };
    assert!(clamped.begin_quota_window(local_at(2027, 1, 31, 9, 0)));
    assert_eq!(
        clamped.quota_next_reset(local_at(2027, 2, 1, 0, 0)),
        Some(local_at(2027, 2, 28, 9, 0))
    );
}

#[test]
fn test_lifetime_quota_never_resets() {
    let mut rule = Rule {
        max_executions: Some(1),
        quota_period: None,
        execution_count: 1,
        last_executed_at: Some("2020-01-01 00:00:00".into()),
        ..Default::default()
    };
    assert!(rule.quota_exhausted(local_at(2026, 11, 19, 12, 0)));
    assert!(!rule.normalize_quota(local_at(2026, 11, 19, 12, 0)));
    assert_eq!(rule.execution_count, 1);
    assert_eq!(rule.quota_remaining(local_at(2026, 11, 19, 12, 0)), Some(0));
}

#[test]
fn test_quota_period_serde() {
    // Rules without the field keep the historical lifetime quota.
    let legacy: Rule = serde_json::from_str(r#"{"id":"r","name":"R","max_executions":5}"#).unwrap();
    assert_eq!(legacy.quota_period, None);
    assert_eq!(legacy.quota_window_start, None);

    let custom = Rule {
        id: "r".into(),
        name: "R".into(),
        max_executions: Some(2),
        quota_period: Some(QuotaPeriod::Custom { value: 2, unit: QuotaUnit::Days }),
        ..Default::default()
    };
    let json = serde_json::to_string(&custom).unwrap();
    assert!(
        json.contains(r#""quota_period":{"custom":{"value":2,"unit":"days"}}"#),
        "{json}"
    );
    // The custom-window anchor is runtime state, never part of the definition.
    assert!(!json.contains("quota_window_start"));
    let decoded: Rule = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.quota_period, custom.quota_period);

    // Calendar variants serialize as bare snake_case strings.
    let daily: Rule = serde_json::from_str(r#"{"id":"r","name":"R","quota_period":"day"}"#).unwrap();
    assert_eq!(daily.quota_period, Some(QuotaPeriod::Day));
    // A custom window without a unit defaults to minutes.
    let minutes: Rule =
        serde_json::from_str(r#"{"id":"r","name":"R","quota_period":{"custom":{"value":30}}}"#).unwrap();
    assert_eq!(
        minutes.quota_period,
        Some(QuotaPeriod::Custom { value: 30, unit: QuotaUnit::Minutes })
    );
}

#[test]
fn test_duration_from_secs_rejects_invalid_values() {
    assert_eq!(
        duration_from_secs(1.5).expect("valid").as_millis(),
        1_500
    );
    for invalid in [-1.0, f64::NAN, f64::INFINITY, MAX_WAIT_SECONDS + 1.0, 1e300] {
        assert!(
            duration_from_secs(invalid).is_err(),
            "{invalid} must be rejected"
        );
    }
}

#[test]
fn test_custom_quota_window_rejects_invalid_and_lapses_on_overflow() {
    for value in [0, QuotaPeriod::MAX_CUSTOM_WINDOW_UNITS + 1] {
        assert!(
            QuotaPeriod::Custom { value, unit: QuotaUnit::Minutes }
                .validation_error()
                .is_some(),
            "value {value} must be rejected"
        );
    }
    assert!(QuotaPeriod::Custom { value: 1, unit: QuotaUnit::Minutes }
        .validation_error()
        .is_none());
    assert!(QuotaPeriod::Day.validation_error().is_none());

    // An unrepresentable window end must read as a lapsed window, never as "still counting".
    let extreme = Local
        .with_ymd_and_hms(262_140, 1, 1, 0, 0, 0)
        .single()
        .expect("chrono supports this year");
    assert!(QuotaPeriod::Custom {
        value: QuotaPeriod::MAX_CUSTOM_WINDOW_UNITS,
        unit: QuotaUnit::Months,
    }
    .custom_window_end(extreme)
    .is_none());

    // A corrupt anchor is likewise not a current window.
    let mut rule = Rule {
        max_executions: Some(1),
        quota_period: Some(QuotaPeriod::Custom {
            value: 1,
            unit: QuotaUnit::Days,
        }),
        execution_count: 1,
        quota_window_start: Some("not a timestamp".into()),
        ..Default::default()
    };
    let now = local_at(2026, 11, 19, 12, 0);
    assert_eq!(rule.quota_used(now), 0);
    assert!(!rule.quota_exhausted(now));
    assert!(rule.normalize_quota(now));
}

#[test]
fn test_parse_hex_color_rejects_non_ascii_tag_colour() {
    assert_eq!(parse_hex_color("#10B981"), Some([0x10, 0xB9, 0x81]));
    assert_eq!(parse_hex_color("10b981"), Some([0x10, 0xB9, 0x81]));
    for invalid in ["a\u{e9}\u{20ac}", "12345", "#12345G", "1234567"] {
        assert_eq!(parse_hex_color(invalid), None, "{invalid}");
    }

    let broken = RuleTag::new("Broken").with_color("a\u{e9}\u{20ac}");
    let mut rule = Rule {
        tags: vec![broken],
        ..Default::default()
    };
    assert!(rule.tag_color_error().is_some());
    rule.tags = vec![RuleTag::new("Good").with_color("#10B981")];
    assert!(rule.tag_color_error().is_none());
}

#[test]
fn test_quota_redefinition_helpers() {
    let lifetime = Rule::default();
    let mut defined = lifetime.clone();
    defined.max_executions = Some(5);
    assert!(defined.quota_redefined_from(&lifetime));

    // Changing only the numeric limit keeps the running counter.
    let mut same = defined.clone();
    same.max_executions = Some(9);
    assert!(!same.quota_redefined_from(&defined));

    let mut periodic = same.clone();
    periodic.quota_period = Some(QuotaPeriod::Day);
    assert!(periodic.quota_redefined_from(&same));

    let mut progress = defined.clone();
    progress.execution_count = 4;
    progress.last_executed_at = Some("2026-11-19 10:00:00".into());
    progress.quota_window_start = Some("2026-11-19 09:00:00".into());
    progress.reset_quota_progress();
    assert_eq!(progress.execution_count, 0);
    assert!(progress.quota_window_start.is_none());
    // Schedules deduplicate on this stamp, so it must survive a quota reset.
    assert_eq!(
        progress.last_executed_at.as_deref(),
        Some("2026-11-19 10:00:00")
    );
}

#[test]
fn test_overlay_ids_normalize_across_load_and_nesting() {
    let mut action = Action::Overlay {
        overlay_id: "action_id".into(),
        custom_overlay: Some(Box::new(OverlayDefinition {
            id: "definition_id".into(),
            ..Default::default()
        })),
        operation: OverlayOperation::Show,
        override_text: None,
        duration_ms: None,
        close_on_exit: true,
    };
    action.normalize_overlay_ids();
    if let Action::Overlay {
        overlay_id,
        custom_overlay,
        ..
    } = &action
    {
        assert_eq!(overlay_id, "action_id");
        assert_eq!(
            custom_overlay.as_ref().map(|definition| definition.id.as_str()),
            Some("action_id")
        );
    } else {
        panic!("expected an overlay action");
    }

    // An empty action id adopts the embedded definition id instead.
    let mut empty = Action::Overlay {
        overlay_id: String::new(),
        custom_overlay: Some(Box::new(OverlayDefinition {
            id: "embedded".into(),
            ..Default::default()
        })),
        operation: OverlayOperation::Show,
        override_text: None,
        duration_ms: None,
        close_on_exit: false,
    };
    empty.normalize_overlay_ids();
    if let Action::Overlay { overlay_id, .. } = &empty {
        assert_eq!(overlay_id, "embedded");
    }

    let mut nested = Rule {
        actions: vec![Action::IfElse {
            conditions: Vec::new(),
            then_actions: vec![action],
            else_if_branches: Vec::new(),
            else_actions: Vec::new(),
        }],
        ..Default::default()
    };
    nested.normalize();
    if let Action::IfElse { then_actions, .. } = &nested.actions[0] {
        if let Action::Overlay { custom_overlay, .. } = &then_actions[0] {
            assert_eq!(
                custom_overlay.as_ref().map(|definition| definition.id.as_str()),
                Some("action_id")
            );
        } else {
            panic!("expected a nested overlay action");
        }
    } else {
        panic!("expected an if_else action");
    }
}

#[test]
fn test_validate_rule_keys_rejects_typos() {
    let rule = Rule {
        id: "r".into(),
        name: "R".into(),
        actions: vec![Action::StartProcess {
            file_path: "app.exe".into(),
            arguments: String::new(),
            working_directory: None,
            run_as_admin: false,
            only_if_not_running: false,
            run_hidden: false,
            close_on_exit: false,
        }],
        tags: vec![RuleTag::new("tag")],
        ..Default::default()
    };
    let mut value = serde_json::to_value(&rule).expect("serialize");
    assert!(validate_rule_keys(&value).is_ok());

    value["max_execution"] = serde_json::json!(5);
    value["actions"][0]["max_execution"] = serde_json::json!(5);
    value["tags"][0]["colour"] = serde_json::json!("#fff");
    let errors = validate_rule_keys(&value).expect_err("typos must be reported");
    let rendered = errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ");
    assert!(rendered.contains("max_execution"), "{rendered}");
    assert!(rendered.contains("actions[0]"), "{rendered}");
    assert!(rendered.contains("tags[0]"), "{rendered}");
}

#[test]
fn test_schedule_per_type_validation() {
    let schedule = |schedule_type: ScheduleType,
                    interval_sec: u64,
                    daily_time: Option<&str>,
                    days_of_week: Vec<u8>,
                    day_of_month: Option<u8>,
                    target_datetime: Option<&str>| Trigger::Schedule {
        schedule_type,
        interval_sec,
        daily_time: daily_time.map(str::to_string),
        days_of_week,
        day_of_month,
        target_datetime: target_datetime.map(str::to_string),
    };

    assert!(schedule(ScheduleType::Daily, 0, None, vec![], None, None)
        .schedule_errors()
        .iter()
        .any(|error| error.contains("daily_time")));
    assert!(!schedule(ScheduleType::Daily, 0, Some("9:00"), vec![], None, None)
        .schedule_errors()
        .is_empty());
    assert!(schedule(ScheduleType::Daily, 0, Some("09:00"), vec![], None, None)
        .schedule_errors()
        .is_empty());
    assert!(schedule(ScheduleType::Daily, 0, Some("09:00:00"), vec![], None, None)
        .schedule_errors()
        .is_empty());
    assert!(!schedule(ScheduleType::Interval, 0, None, vec![], None, None)
        .schedule_errors()
        .is_empty());
    assert!(!schedule(ScheduleType::OneShot, 0, None, vec![], None, None)
        .schedule_errors()
        .is_empty());
    assert!(schedule(
        ScheduleType::OneShot,
        0,
        None,
        vec![],
        None,
        Some("2026-12-31 23:59")
    )
    .schedule_errors()
    .is_empty());
    assert!(!schedule(ScheduleType::Monthly, 0, Some("12:00:00"), vec![], None, None)
        .schedule_errors()
        .is_empty());
    assert!(schedule(
        ScheduleType::Monthly,
        0,
        Some("12:00:00"),
        vec![],
        Some(1),
        None
    )
    .schedule_errors()
    .is_empty());
    assert!(!schedule(ScheduleType::Daily, 0, Some("09:00"), vec![0], None, None)
        .schedule_errors()
        .is_empty());
}

#[test]
fn test_time_of_day_and_day_validators() {
    assert!(is_valid_hh_mm("00:00"));
    assert!(is_valid_hh_mm("23:59"));
    assert!(!is_valid_hh_mm("9:00"));
    assert!(!is_valid_hh_mm("24:00"));
    assert!(!is_valid_hh_mm("23:60"));
    assert!(!is_valid_hh_mm("23:5"));
    assert!(is_valid_hh_mm_ss("23:59:59"));
    assert!(!is_valid_hh_mm_ss("23:59:60"));
    assert!(days_of_week_are_valid(&[1, 7]));
    assert!(!days_of_week_are_valid(&[0]));
    assert!(!days_of_week_are_valid(&[8]));
    assert!(is_valid_day_of_month(31));
    assert!(!is_valid_day_of_month(0));
    assert!(!is_valid_day_of_month(32));
}

#[test]
fn test_resource_threshold_and_wait_validation() {
    assert_eq!(resource_threshold_errors("", None, None).len(), 2);
    assert_eq!(resource_threshold_errors("game.exe", None, None).len(), 1);
    assert!(resource_threshold_errors("game.exe", Some(80.0), None).is_empty());

    let missing_condition = Action::Wait {
        mode: WaitMode::UntilCondition,
        seconds: 0.0,
        condition: None,
        timeout_sec: 30.0,
        timeout_policy: WaitTimeoutPolicy::Abort,
    };
    assert!(missing_condition
        .validation_errors()
        .iter()
        .any(|error| error.contains("needs a condition")));

    let negative_timer = Action::Wait {
        mode: WaitMode::Timer,
        seconds: -5.0,
        condition: None,
        timeout_sec: 30.0,
        timeout_policy: WaitTimeoutPolicy::Abort,
    };
    assert!(!negative_timer.validation_errors().is_empty());

    let empty_wait = WaitCondition::ProcessStarted {
        process_name: String::new(),
        match_mode: ProcessMatchMode::Exact,
    };
    assert!(!empty_wait.validation_errors().is_empty());

    let bad_range = RuleCondition::TimeRange {
        start_time: "09:00".into(),
        end_time: "25:00".into(),
        days_of_week: vec![1, 2],
    };
    assert!(!bad_range.validation_errors().is_empty());
}
