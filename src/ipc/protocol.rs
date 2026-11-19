use serde::{Deserialize, Serialize};
use crate::config::{AppConfig, Settings};
use crate::models::{Action, AnalyticsSummary, LogItem, ProcessSnapshotItem, Rule};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum IpcRequest {
    Ping,
    GetState,
    SaveSettings(Settings),
    SaveRule(Rule),
    ReorderRules(Vec<String>),
    ResetRule(String),
    DeleteRule(String),
    ToggleRule { id: String, enabled: bool },
    TestRunRule {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rule: Option<Box<Rule>>,
    },
    StopRuleExecution(String),
    TestStep { action: Action, context: std::collections::HashMap<String, String> },
    GetProcesses,
    KillProcess { pid: u32, force: bool },
    SetAutorun(bool),
    UpdateWindowState(crate::config::WindowState),
    ClearLogs,
    ShutdownDaemon,
    ImportRules(Vec<Rule>),
    /// Apply a decoded share code: add its rules plus the library presets/overlays they
    /// reference. Imported rules are disabled unless `enable` is set.
    ImportShare {
        payload: Box<crate::sharing::SharePayload>,
        #[serde(default)]
        enable: bool,
    },
    /// Declarative replace of the whole rule set (CLI `--rules-sync`), applied and
    /// persisted in one step.
    SyncRules(Vec<Rule>),
    SaveOverlay(crate::models::OverlayDefinition),
    DeleteOverlay(String),
    SaveMacroPreset(crate::models::MacroPreset),
    DeleteMacroPreset(String),
    SetOverlayText { id: String, text: String },
    ShowOverlayPreview { overlay: crate::models::OverlayDefinition, calibration_mode: bool },
    HideOverlayPreview(String),
    GetOverlayPosition(String),
    UpdateOverlayPosition { id: String, x: f32, y: f32 },
    SetAvailableUpdate(Option<crate::updater::ReleaseInfo>),
    AcknowledgeOpenUpdateModal,
    TriggerUpdate,
    StartUpdate,
    AcknowledgeAutoStartUpdate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
#[allow(clippy::large_enum_variant)]
pub enum IpcResponse {
    Ok,
    Pong,
    State(DaemonState),
    Processes(Vec<ProcessSnapshotItem>),
    TestStepResult(Result<(), String>),
    OverlayPosition(Option<(f32, f32)>),
    ShareImportResult(crate::sharing::ShareImportSummary),
    Error(String),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct ActiveRuleExecutionState {
    pub active_count: usize,
    pub current_step: usize,
    pub total_steps: usize,
    pub is_exit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonState {
    pub autorun_enabled: bool,
    pub uptime_secs: u64,
    pub daemon_memory_bytes: u64,
    pub daemon_cpu_usage: f32,
    pub active_rules_count: usize,
    pub total_rules_count: usize,
    pub config: AppConfig,
    pub logs: Vec<LogItem>,
    #[serde(default)]
    pub active_rule_executions: std::collections::HashMap<String, ActiveRuleExecutionState>,
    #[serde(default)]
    pub active_overlay_ids: std::collections::HashSet<String>,
    #[serde(default)]
    pub active_test_rule_ids: std::collections::HashSet<String>,
    #[serde(default)]
    pub analytics: AnalyticsSummary,
    #[serde(default)]
    pub available_update: Option<crate::updater::ReleaseInfo>,
    #[serde(default)]
    pub update_progress: Option<f32>,
    #[serde(default)]
    pub update_status: Option<String>,
    #[serde(default)]
    pub pending_open_update_modal: bool,
    #[serde(default)]
    pub pending_auto_start_update: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_active_rule_execution_state_copy_and_serde() {
        let state = ActiveRuleExecutionState {
            active_count: 3,
            current_step: 1,
            total_steps: 4,
            is_exit: false,
        };
        let copied = state;
        assert_eq!(copied.active_count, 3);
        assert_eq!(copied.current_step, 1);
        assert_eq!(copied.total_steps, 4);
        assert!(!copied.is_exit);

        let json = serde_json::to_string(&state).unwrap();
        let decoded: ActiveRuleExecutionState = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, state);
    }

    #[test]
    fn test_ipc_overlay_position_serde() {
        let req = IpcRequest::GetOverlayPosition("test_ov".into());
        let json_req = serde_json::to_string(&req).unwrap();
        let dec_req: IpcRequest = serde_json::from_str(&json_req).unwrap();
        match dec_req {
            IpcRequest::GetOverlayPosition(id) => assert_eq!(id, "test_ov"),
            _ => panic!("wrong req variant"),
        }

        let resp = IpcResponse::OverlayPosition(Some((100.0, 200.0)));
        let json_resp = serde_json::to_string(&resp).unwrap();
        let dec_resp: IpcResponse = serde_json::from_str(&json_resp).unwrap();
        match dec_resp {
            IpcResponse::OverlayPosition(Some((x, y))) => {
                assert_eq!(x, 100.0);
                assert_eq!(y, 200.0);
            }
            _ => panic!("wrong resp variant"),
        }
    }

    #[test]
    fn test_ipc_import_rules_request_serde() {
        let req = IpcRequest::ImportRules(vec![Rule {
            id: "r_test".into(),
            name: "Imported Rule".into(),
            description: "".into(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            trigger: None,
            actions: vec![],
            on_exit_actions: vec![],
            custom_icon_path: Some("C:\\test.ico".into()),
            auto_process_icon: true,
            ..Default::default()
        }]);

        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("ImportRules"));
        assert!(json.contains("Imported Rule"));
    }

    #[test]
    fn test_ipc_import_share_request_serde() {
        let payload = crate::sharing::SharePayload::rules_only(vec![Rule {
            id: "share_rule".into(),
            name: "Shared Rule".into(),
            ..Default::default()
        }]);
        let req = IpcRequest::ImportShare {
            payload: Box::new(payload),
            enable: true,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("ImportShare"));
        assert!(json.contains("Shared Rule"));
        match serde_json::from_str::<IpcRequest>(&json).unwrap() {
            IpcRequest::ImportShare { payload, enable } => {
                assert!(enable);
                assert_eq!(payload.rules[0].id, "share_rule");
            }
            other => panic!("wrong req variant: {other:?}"),
        }
    }

    #[test]
    fn test_ipc_share_import_result_serde() {
        let response = IpcResponse::ShareImportResult(crate::sharing::ShareImportSummary {
            imported: 2,
            skipped_duplicates: 1,
            overlays: 1,
            macro_presets: 0,
            enabled: false,
        });
        let json = serde_json::to_string(&response).unwrap();
        match serde_json::from_str::<IpcResponse>(&json).unwrap() {
            IpcResponse::ShareImportResult(summary) => {
                assert_eq!(summary.imported, 2);
                assert_eq!(summary.skipped_duplicates, 1);
            }
            other => panic!("wrong response variant: {other:?}"),
        }
    }

    #[test]
    fn test_ipc_sync_rules_request_serde() {
        let req = IpcRequest::SyncRules(vec![Rule {
            id: "r_sync".into(),
            name: "Synced Rule".into(),
            ..Default::default()
        }]);
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("SyncRules"));
        let decoded: IpcRequest = serde_json::from_str(&json).unwrap();
        match decoded {
            IpcRequest::SyncRules(rules) => {
                assert_eq!(rules.len(), 1);
                assert_eq!(rules[0].id, "r_sync");
            }
            _ => panic!("wrong req variant"),
        }
    }

    #[test]
    fn test_ipc_test_run_and_stop_serde() {
        let req = IpcRequest::TestRunRule {
            id: "rule_1".into(),
            rule: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        let dec: IpcRequest = serde_json::from_str(&json).unwrap();
        match dec {
            IpcRequest::TestRunRule { id, rule } => {
                assert_eq!(id, "rule_1");
                assert!(rule.is_none());
            }
            _ => panic!("wrong variant"),
        }

        let stop_req = IpcRequest::StopRuleExecution("rule_1".into());
        let json_stop = serde_json::to_string(&stop_req).unwrap();
        let dec_stop: IpcRequest = serde_json::from_str(&json_stop).unwrap();
        match dec_stop {
            IpcRequest::StopRuleExecution(id) => assert_eq!(id, "rule_1"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_ipc_update_requests_and_state_serde() {
        let release = crate::updater::ReleaseInfo {
            version: "2.0.0".to_string(),
            tag_name: "v2.0.0".to_string(),
            name: "Synector 2.0".to_string(),
            body: "Big update".to_string(),
            html_url: "https://github.com".to_string(),
            portable: Some(crate::updater::UpdateAsset {
                name: "Synector-v2.0.0-Windows-x64.zip".to_string(),
                url: "https://github.com/asset.zip".to_string(),
                size: 1024,
                sha256: Some("a".repeat(64)),
            }),
            installer: Some(crate::updater::UpdateAsset {
                name: "Synector-Setup-2.0.0-Windows-x64.exe".to_string(),
                url: "https://github.com/setup.exe".to_string(),
                size: 2048,
                sha256: Some("b".repeat(64)),
            }),
        };

        let req = IpcRequest::SetAvailableUpdate(Some(release.clone()));
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("SetAvailableUpdate"));
        let dec: IpcRequest = serde_json::from_str(&json).unwrap();
        match dec {
            IpcRequest::SetAvailableUpdate(Some(r)) => assert_eq!(r.version, "2.0.0"),
            _ => panic!("Expected SetAvailableUpdate"),
        }

        let ack_req = IpcRequest::AcknowledgeOpenUpdateModal;
        let json_ack = serde_json::to_string(&ack_req).unwrap();
        assert!(json_ack.contains("AcknowledgeOpenUpdateModal"));

        let trig_req = IpcRequest::TriggerUpdate;
        let json_trig = serde_json::to_string(&trig_req).unwrap();
        assert!(json_trig.contains("TriggerUpdate"));

        let start_req = IpcRequest::StartUpdate;
        let json_start = serde_json::to_string(&start_req).unwrap();
        assert!(json_start.contains("StartUpdate"));

        let ack_auto = IpcRequest::AcknowledgeAutoStartUpdate;
        let json_auto = serde_json::to_string(&ack_auto).unwrap();
        assert!(json_auto.contains("AcknowledgeAutoStartUpdate"));

        let state = DaemonState {
            autorun_enabled: false,
            uptime_secs: 42,
            daemon_memory_bytes: 5000,
            daemon_cpu_usage: 0.1,
            active_rules_count: 0,
            total_rules_count: 0,
            config: AppConfig::default(),
            logs: vec![],
            active_rule_executions: std::collections::HashMap::new(),
            active_overlay_ids: std::collections::HashSet::new(),
            active_test_rule_ids: std::collections::HashSet::new(),
            analytics: AnalyticsSummary::default(),
            available_update: Some(release),
            update_progress: Some(0.75),
            update_status: Some("75%".to_string()),
            pending_open_update_modal: true,
            pending_auto_start_update: true,
        };

        let state_json = serde_json::to_string(&state).unwrap();
        let decoded_state: DaemonState = serde_json::from_str(&state_json).unwrap();
        assert_eq!(decoded_state.available_update.unwrap().version, "2.0.0");
        assert_eq!(decoded_state.update_progress, Some(0.75));
        assert!(decoded_state.pending_open_update_modal);
        assert!(decoded_state.pending_auto_start_update);
    }
}
