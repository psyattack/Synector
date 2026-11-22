use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::models::GraphMetric;

use super::system::{get_network_speed_bytes_per_sec, get_system_metrics};

// ---------------- METRIC HISTORY RING BUFFERS ----------------

struct MetricBuffer {
    last_push: Instant,
    samples: VecDeque<f32>,
}

pub(super) struct MetricHistoryTracker {
    buffers: HashMap<GraphMetric, MetricBuffer>,
}

const METRIC_PUSH_INTERVAL: Duration = Duration::from_millis(400);

pub(super) static METRIC_HISTORY: Mutex<Option<MetricHistoryTracker>> = Mutex::new(None);

pub fn get_graph_samples(metric: GraphMetric, sample_count: usize, target_process: Option<&str>) -> Vec<f32> {
    let count = sample_count.clamp(8, 100);
    let metrics = get_system_metrics(target_process);

    // Metrics without a real source produce no history instead of a fabricated line.
    let current_val = match metric {
        GraphMetric::SystemCpu => Some(metrics.cpu_usage_pct / 100.0),
        GraphMetric::ProcessCpu => metrics.target_process_cpu_pct.map(|cpu| cpu / 100.0),
        GraphMetric::SystemRam => Some(metrics.ram_usage_pct / 100.0),
        GraphMetric::ProcessRam => metrics
            .target_process_ram_mb
            .map(|mb| (mb as f32 / metrics.ram_total_mb.max(1) as f32).clamp(0.0, 1.0)),
        // No frame-rate source is available in the daemon; the label reports n/a.
        GraphMetric::Fps => None,
        GraphMetric::Battery => metrics.battery_pct.map(|pct| pct as f32 / 100.0),
        GraphMetric::Network => {
            let speed = get_network_speed_bytes_per_sec();
            Some(if speed <= 10.0 {
                0.03
            } else {
                ((speed.log10() - 1.0) / 6.0).clamp(0.04, 0.96)
            })
        }
    };
    let Some(current_val) = current_val else {
        return Vec::new();
    };

    let mut lock = match METRIC_HISTORY.lock() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };

    let now = Instant::now();
    let tracker = lock.get_or_insert_with(|| MetricHistoryTracker {
        buffers: HashMap::new(),
    });

    // Each metric has its own sample latch: two graphs in one frame must both advance.
    let buffer = tracker
        .buffers
        .entry(metric)
        .or_insert_with(|| MetricBuffer {
            last_push: now,
            samples: VecDeque::new(),
        });

    if buffer.samples.is_empty() || now.duration_since(buffer.last_push) >= METRIC_PUSH_INTERVAL {
        buffer.last_push = now;
        buffer.samples.push_back(current_val.clamp(0.02, 0.98));
        while buffer.samples.len() > 100 {
            buffer.samples.pop_front();
        }
    }

    // A short history is rendered short rather than padded with generated samples.
    buffer.samples.iter().rev().take(count).rev().copied().collect()
}

pub fn get_graph_current_label(metric: GraphMetric, target_process: Option<&str>) -> String {
    let metrics = get_system_metrics(target_process);
    match metric {
        GraphMetric::SystemCpu => format!("CPU: {:.0}%", metrics.cpu_usage_pct),
        GraphMetric::ProcessCpu => match metrics.target_process_cpu_pct {
            Some(cpu) => format!("P-CPU: {:.0}%", cpu),
            None => "P-CPU: n/a".to_string(),
        },
        GraphMetric::SystemRam => format!("RAM: {:.0}% ({:.1}/{:.1} GB)", metrics.ram_usage_pct, metrics.ram_used_mb as f32 / 1024.0, metrics.ram_total_mb as f32 / 1024.0),
        GraphMetric::ProcessRam => match metrics.target_process_ram_mb {
            Some(mb) => format!("P-RAM: {} MB", mb),
            None => "P-RAM: n/a".to_string(),
        },
        GraphMetric::Fps => "FPS: n/a".to_string(),
        GraphMetric::Battery => {
            if let Some(bat) = metrics.battery_pct {
                format!("BAT: {}%", bat)
            } else {
                "PWR: AC".to_string()
            }
        }
        GraphMetric::Network => {
            let speed = get_network_speed_bytes_per_sec();
            if speed >= 1024.0 * 1024.0 {
                format!("NET: {:.1} MB/s", speed / (1024.0 * 1024.0))
            } else if speed >= 1024.0 {
                format!("NET: {:.0} KB/s", speed / 1024.0)
            } else {
                format!("NET: {:.0} B/s", speed)
            }
        }
    }
}
