mod audio;
mod engine;
mod graph;
mod keys;
mod media;
mod process;
mod system;

pub use self::audio::simulate_media_key;
pub use self::engine::{get_engine_telemetry, update_engine_stats};
pub use self::graph::{get_graph_current_label, get_graph_samples};
pub use self::keys::{get_active_or_recent_keys, is_vk_pressed};
pub use self::media::get_system_media_status;
pub use self::process::{daemon_memory_bytes, sample_daemon_usage};
pub use self::system::format_telemetry_text;
pub use self::system::get_cpu_ram_metrics;

#[cfg(test)]
use std::time::{Duration, Instant};

#[cfg(test)]
use crate::models::GraphMetric;

#[cfg(test)]
use self::{
    audio::media_key_is_extended,
    graph::METRIC_HISTORY,
    process::process_cpu_percent,
    system::metrics_snapshot_is_fresh,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_cpu_percent_math() {
        // One fully busy core for one second is 100 % of a single-core machine.
        let one_core = process_cpu_percent(10_000_000, Duration::from_secs(1), 1).unwrap();
        assert!((one_core - 100.0).abs() < 0.01);

        // The same CPU time spread over four cores is 25 % of total capacity.
        let four_cores = process_cpu_percent(10_000_000, Duration::from_secs(1), 4).unwrap();
        assert!((four_cores - 25.0).abs() < 0.01);

        assert_eq!(process_cpu_percent(10, Duration::ZERO, 4), None);
        assert_eq!(process_cpu_percent(10, Duration::from_secs(1), 0), None);
    }

    #[test]
    fn test_metric_history_advances_per_metric() {
        if let Ok(mut lock) = METRIC_HISTORY.lock() {
            *lock = None;
        }

        let cpu_first = get_graph_samples(GraphMetric::SystemCpu, 8, None);
        let ram_first = get_graph_samples(GraphMetric::SystemRam, 8, None);
        assert_eq!(cpu_first.len(), 1);
        assert_eq!(ram_first.len(), 1);

        std::thread::sleep(Duration::from_millis(410));

        let cpu_second = get_graph_samples(GraphMetric::SystemCpu, 8, None);
        let ram_second = get_graph_samples(GraphMetric::SystemRam, 8, None);
        assert_eq!(cpu_second.len(), 2, "the CPU buffer must advance");
        assert_eq!(
            ram_second.len(),
            2,
            "a second graph must advance even when another metric sampled in the same frame"
        );
        assert!(cpu_second.iter().all(|value| value.is_finite()));
        assert!(ram_second.iter().all(|value| value.is_finite()));
    }

    #[test]
    fn test_unsupported_graph_metrics_return_no_samples() {
        assert!(get_graph_samples(GraphMetric::Fps, 16, None).is_empty());
        assert!(get_graph_samples(GraphMetric::ProcessCpu, 16, None).is_empty());
        assert!(get_graph_samples(GraphMetric::ProcessRam, 16, None).is_empty());

        assert_eq!(get_graph_current_label(GraphMetric::Fps, None), "FPS: n/a");
        assert_eq!(
            get_graph_current_label(GraphMetric::ProcessCpu, None),
            "P-CPU: n/a"
        );
        assert_eq!(
            get_graph_current_label(GraphMetric::ProcessRam, None),
            "P-RAM: n/a"
        );
    }

    #[test]
    fn test_media_key_extended_flags() {
        assert!(media_key_is_extended(0xAD)); // VK_VOLUME_MUTE
        assert!(media_key_is_extended(0xAF)); // VK_VOLUME_UP
        assert!(media_key_is_extended(0xB3)); // VK_MEDIA_PLAY_PAUSE
        assert!(!media_key_is_extended(b'A'));
    }

    #[test]
    fn test_metrics_snapshot_cache_freshness() {
        assert!(metrics_snapshot_is_fresh(Instant::now(), true));
        assert!(!metrics_snapshot_is_fresh(Instant::now(), false));
        assert!(!metrics_snapshot_is_fresh(
            Instant::now() - Duration::from_millis(800),
            true
        ));
    }
}
