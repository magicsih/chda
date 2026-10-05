//! macOS sampling, run only on a worker. Missing processes are not zero usage.
use chda_core::resources::{Listener, Resources};
use std::{collections::HashSet, process::Command, time::Duration};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

pub struct Sampler {
    system: System,
    root: Option<(u32, u64)>,
    ports: Option<(u64, Vec<Listener>, Option<String>)>,
}
impl Default for Sampler {
    fn default() -> Self {
        Self {
            system: System::new(),
            root: None,
            ports: None,
        }
    }
}
impl Sampler {
    pub fn sample(&mut self, root: u32, now: u64, ports_due: bool) -> Result<Resources, String> {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            ProcessRefreshKind::new().with_cpu().with_memory(),
        );
        let process = self
            .system
            .process(Pid::from_u32(root))
            .ok_or("PTY process exited or cannot be inspected")?;
        let identity = (root, process.start_time());
        let warmed = self.root == Some(identity);
        if self
            .root
            .is_some_and(|old| old.0 == root && old.1 != identity.1)
        {
            self.root = None;
            self.ports = None;
            return Err("PTY PID has been reused".into());
        }
        if !warmed {
            self.ports = None;
        }
        self.root = Some(identity);
        let mut pids = HashSet::from([Pid::from_u32(root)]);
        loop {
            let before = pids.len();
            for (pid, child) in self.system.processes() {
                if child.parent().is_some_and(|p| pids.contains(&p)) {
                    pids.insert(*pid);
                }
            }
            if pids.len() == before {
                break;
            }
        }
        let mut result = Resources {
            root_pid: root,
            root_started_at: identity.1,
            cpu_percent: warmed.then_some(0.0),
            uptime_seconds: now / 1000 - process.start_time().min(now / 1000),
            observed_at: now,
            ..Default::default()
        };
        for pid in &pids {
            if let Some(p) = self.system.process(*pid) {
                result.rss_bytes = result.rss_bytes.saturating_add(p.memory());
                if let Some(cpu) = &mut result.cpu_percent {
                    *cpu += p.cpu_usage();
                }
                result.process_count += 1;
            }
        }
        if ports_due || self.ports.is_none() {
            let mut ids: Vec<_> = pids.iter().map(|p| p.as_u32()).collect();
            ids.sort();
            let ids = ids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
            let mut command = Command::new("/usr/sbin/lsof");
            command.args(["-nP", "-a", "-p", &ids, "-FpcfPnT"]);
            match chda_core::agents::ipc::capture_command(
                &mut command,
                None,
                Duration::from_secs(1),
                1_048_576,
            ) {
                Ok(output) => {
                    self.ports = Some((now, parse_ports(&String::from_utf8_lossy(&output)), None))
                }
                // Listing all descriptors makes a successful empty network
                // subset distinct from an execution or inspection failure.
                Err(e) => self.ports = Some((now, Vec::new(), Some(e.to_string()))),
            }
        }
        if let Some((time, ports, error)) = &self.ports {
            result.ports_observed_at = error.is_none().then_some(*time);
            result.listeners = ports.clone();
            result.ports_error = error.clone();
        }
        Ok(result)
    }
}

fn parse_ports(output: &str) -> Vec<Listener> {
    let mut ports = Vec::new();
    let mut pid = 0;
    let mut process = String::new();
    let mut protocol = String::new();
    let mut address = String::new();
    let mut listening = false;
    let flush = |ports: &mut Vec<Listener>,
                 pid,
                 process: &str,
                 protocol: &str,
                 address: &str,
                 listening| {
        if pid > 0
            && !address.is_empty()
            && ((protocol == "TCP" && listening) || (protocol == "UDP" && !address.contains("->")))
        {
            let listener = Listener {
                pid,
                process: process.into(),
                protocol: protocol.into(),
                address: address.into(),
            };
            if !ports.contains(&listener) {
                ports.push(listener);
            }
        }
    };
    for line in output.lines() {
        let Some((tag, value)) = line.split_at_checked(1) else {
            continue;
        };
        match tag {
            "p" | "f" => {
                flush(&mut ports, pid, &process, &protocol, &address, listening);
                protocol.clear();
                address.clear();
                listening = false;
                if tag == "p" {
                    pid = value.parse().unwrap_or(0);
                    process.clear();
                }
            }
            "c" => process = value.into(),
            "P" => protocol = value.into(),
            "n" => address = value.into(),
            "T" if value == "ST=LISTEN" => listening = true,
            _ => {}
        }
    }
    flush(&mut ports, pid, &process, &protocol, &address, listening);
    ports
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn listeners_exclude_outbound_connections_and_keep_owner() {
        let ports = parse_ports(
            "p42\ncnode\nf1\nPTCP\nn127.0.0.1:3000\nTST=LISTEN\nf2\nPTCP\nn127.0.0.1:4000->1.2.3.4:443\nTST=ESTABLISHED\nf3\nPUDP\nn*:5000\nf4\nPUDP\nn127.0.0.1:4000->1.2.3.4:53\n",
        );
        assert_eq!(ports.len(), 2);
        assert_eq!(ports[0].pid, 42);
        assert_eq!(ports[0].address, "127.0.0.1:3000");
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    #[test]
    fn own_listener_is_attributed_and_first_cpu_sample_is_unknown() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let mut sampler = Sampler::default();
        let now = crate::terminal_view::now_ms();
        let report = sampler.sample(std::process::id(), now, true).unwrap();
        assert_eq!(report.cpu_percent, None);
        assert!(report.rss_bytes > 0);
        assert!(report.ports_error.is_none(), "{:?}", report.ports_error);
        assert!(
            report.listeners.iter().any(|l| l.pid == std::process::id()
                && l.protocol == "TCP"
                && l.address == address),
            "{:?}",
            report.listeners
        );
        drop(listener);
        let report = sampler
            .sample(std::process::id(), crate::terminal_view::now_ms(), true)
            .unwrap();
        assert!(!report.listeners.iter().any(|l| l.address == address));
        assert!(
            sampler.sample(u32::MAX, now, false).is_err(),
            "an exited process is never zero CPU"
        );
    }
}
