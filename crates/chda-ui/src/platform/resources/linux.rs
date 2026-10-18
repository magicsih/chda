//! Match a process tree's socket descriptors to its network namespaces.

use std::collections::{HashMap, HashSet};
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use chda_core::resources::Listener;

pub(super) fn listeners(pids: &[u32]) -> Result<Vec<Listener>, String> {
    let mut owners: HashMap<u64, Vec<(u32, String)>> = HashMap::new();
    let mut namespaces = HashMap::new();
    for &pid in pids {
        let root = std::path::PathBuf::from(format!("/proc/{pid}"));
        let descriptors = match std::fs::read_dir(root.join("fd")) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("Cannot inspect process socket descriptors: {e}")),
        };
        let name = std::fs::read_to_string(root.join("comm"))
            .unwrap_or_default()
            .trim()
            .to_owned();
        for descriptor in descriptors {
            let descriptor = descriptor.map_err(|e| e.to_string())?;
            let target = match std::fs::read_link(descriptor.path()) {
                Ok(target) => target,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(format!("Cannot inspect process socket: {e}")),
            };
            if let Some(inode) = target
                .to_str()
                .and_then(|s| s.strip_prefix("socket:["))
                .and_then(|s| s.strip_suffix(']'))
                .and_then(|s| s.parse::<u64>().ok())
            {
                let entries = owners.entry(inode).or_default();
                if !entries.iter().any(|(owner, _)| *owner == pid) {
                    entries.push((pid, name.clone()));
                }
            }
        }
        match std::fs::read_link(root.join("ns/net")) {
            Ok(namespace) => {
                namespaces.entry(namespace).or_insert(root);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Cannot inspect the process network namespace: {e}")),
        }
    }
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for root in namespaces.values() {
        for (file, protocol) in [
            ("tcp", "TCP"),
            ("tcp6", "TCP"),
            ("udp", "UDP"),
            ("udp6", "UDP"),
        ] {
            let table = match std::fs::read_to_string(root.join("net").join(file)) {
                Ok(table) => table,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(format!("Cannot inspect listening ports: {e}")),
            };
            for (inode, address) in parse_table(&table, protocol) {
                for (pid, process) in owners.get(&inode).into_iter().flatten() {
                    if seen.insert((*pid, protocol, address.clone())) {
                        result.push(Listener {
                            pid: *pid,
                            process: process.clone(),
                            protocol: protocol.into(),
                            address: address.clone(),
                        });
                    }
                }
            }
        }
    }
    result.sort_by(|a, b| {
        a.pid
            .cmp(&b.pid)
            .then(a.protocol.cmp(&b.protocol))
            .then(a.address.cmp(&b.address))
    });
    Ok(result)
}

fn parse_table(table: &str, protocol: &str) -> Vec<(u64, String)> {
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<_> = line.split_ascii_whitespace().collect();
            let local = socket_address(fields.get(1)?)?;
            let remote = socket_address(fields.get(2)?)?;
            let state = *fields.get(3)?;
            if local.port() == 0
                || if protocol == "TCP" {
                    state != "0A"
                } else {
                    remote.port() != 0
                }
            {
                return None;
            }
            Some((fields.get(9)?.parse().ok()?, local.to_string()))
        })
        .collect()
}

fn socket_address(text: &str) -> Option<SocketAddr> {
    let (address, port) = text.split_once(':')?;
    let port = u16::from_str_radix(port, 16).ok()?;
    let ip = match address.len() {
        8 => IpAddr::V4(Ipv4Addr::from(
            u32::from_str_radix(address, 16).ok()?.to_ne_bytes(),
        )),
        32 => {
            let mut bytes = [0u8; 16];
            for (index, chunk) in address.as_bytes().as_chunks::<8>().0.iter().enumerate() {
                let word = u32::from_str_radix(std::str::from_utf8(chunk).ok()?, 16).ok()?;
                bytes[index * 4..index * 4 + 4].copy_from_slice(&word.to_ne_bytes());
            }
            IpAddr::V6(Ipv6Addr::from(bytes))
        }
        _ => return None,
    };
    Some(SocketAddr::new(ip, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_tables_keep_listeners_and_unconnected_udp_with_ipv6() {
        let table = "header\n0: 0100007F:1F90 00000000:0000 0A 0:0 0:0 0 1000 0 123\n1: 0100007F:1F91 0100007F:1234 01 0:0 0:0 0 1000 0 124\n2: 00000000000000000000000001000000:1F92 00000000000000000000000000000000:0000 07 0:0 0:0 0 1000 0 125\nmalformed\n";
        assert_eq!(parse_table(table, "TCP"), [(123, "127.0.0.1:8080".into())]);
        assert_eq!(
            parse_table(table, "UDP"),
            [(123, "127.0.0.1:8080".into()), (125, "[::1]:8082".into())]
        );
    }

    #[test]
    fn actual_process_sockets_are_found_and_closed_sockets_disappear() {
        let tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let tcp_address = tcp.local_addr().unwrap().to_string();
        let udp_address = udp.local_addr().unwrap().to_string();
        let pid = std::process::id();
        let ports = listeners(&[pid]).unwrap();
        assert!(
            ports
                .iter()
                .any(|p| p.pid == pid && p.protocol == "TCP" && p.address == tcp_address)
        );
        assert!(
            ports
                .iter()
                .any(|p| p.pid == pid && p.protocol == "UDP" && p.address == udp_address)
        );
        drop(tcp);
        assert!(
            !listeners(&[pid])
                .unwrap()
                .iter()
                .any(|p| p.protocol == "TCP" && p.address == tcp_address)
        );
    }
}
