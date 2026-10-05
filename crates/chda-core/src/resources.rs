//! Session-resource snapshots. RSS totals may include shared pages repeatedly.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Resources {
    pub root_pid: u32,
    pub root_started_at: u64,
    pub cpu_percent: Option<f32>,
    pub rss_bytes: u64,
    pub process_count: usize,
    pub uptime_seconds: u64,
    pub observed_at: u64,
    pub listeners: Vec<Listener>,
    pub ports_observed_at: Option<u64>,
    pub ports_error: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listener {
    pub pid: u32,
    pub process: String,
    pub protocol: String,
    pub address: String,
}
impl Resources {
    pub fn details(&self) -> String {
        let mut lines = vec![format!(
            "Focused pane · PTY {} and {} process(es)\nRSS sum: {:.1} MiB · shared pages can be counted more than once\nCPU: sum across descendants; 100% is one CPU core\nProcess uptime: {}s",
            self.root_pid,
            self.process_count,
            self.rss_bytes as f64 / 1_048_576.0,
            self.uptime_seconds
        )];
        if let Some(error) = &self.ports_error {
            lines.push(format!("Ports unavailable: {error}"));
        }
        for port in &self.listeners {
            lines.push(format!(
                "{} {} · {} PID {}",
                port.protocol, port.address, port.process, port.pid
            ));
        }
        if self.ports_observed_at.is_some() && self.listeners.is_empty() {
            lines.push("No listening TCP or bound unconnected UDP ports".into());
        }
        lines.join("\n")
    }
}
