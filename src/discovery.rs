//! Finding other cnverc PCs on the local link (SPEC §9), so the peer panel
//! can offer a pick-list instead of making someone type an address.
//!
//! Convenience only. Managed switches and VLANs drop broadcast, so typing the
//! address must always work and nothing depends on this. It uses a UDP
//! broadcast on port 47801: every couple of seconds each cnverc announces its
//! name and port to every local network it is on, and listens for the others.
//! Broadcast never leaves the local link, needs no router, no DHCP and no DNS,
//! and works on a bare cable.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::wire::{MAX_NAME, PROTO};

/// The discovery port (SPEC §9).
pub const PORT: u16 = 47801;

/// How often this PC announces itself.
const ANNOUNCE_EVERY: Duration = Duration::from_secs(2);

/// A PC not heard from for this long is taken off the list.
const FORGET_AFTER: Duration = Duration::from_secs(7);

/// The largest announcement accepted. Real ones are under 150 bytes.
const MAX_PACKET: usize = 512;

/// Another cnverc, heard on the local network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub name: String,
    /// Where to connect: the address it was heard from, and the port it
    /// listens on.
    pub addr: SocketAddr,
}

/// What each PC broadcasts.
#[derive(Debug, Serialize, Deserialize)]
struct Announce {
    t: String,
    name: String,
    port: u16,
    proto: u32,
    /// Random per run, so a PC can recognise and ignore its own announcements.
    id: u64,
}

/// Start announcing and listening. `report` hears the list of other PCs
/// whenever it changes.
pub fn start(
    name: &str,
    port: u16,
    stop: Arc<AtomicBool>,
    report: Box<dyn Fn(Vec<Found>) + Send>,
) -> Result<Vec<JoinHandle<()>>> {
    let socket = UdpSocket::bind(("0.0.0.0", PORT))
        .with_context(|| format!("cannot open UDP port {PORT} for discovery"))?;
    socket
        .set_broadcast(true)
        .context("cannot enable broadcast")?;
    socket
        .set_read_timeout(Some(Duration::from_millis(250)))
        .context("cannot set a read timeout")?;

    let id = run_id();
    let announce = serde_json::to_vec(&Announce {
        t: "Announce".to_string(),
        name: name.to_string(),
        port,
        proto: PROTO,
        id,
    })
    .context("cannot build the announcement")?;
    info!("discovery: announcing \"{name}\" on UDP port {PORT}");

    let thread = std::thread::Builder::new()
        .name("cnverc-discovery".to_string())
        .spawn(move || {
            let mut seen: BTreeMap<SocketAddr, (String, Instant)> = BTreeMap::new();
            let mut last_announce: Option<Instant> = None;
            let mut buffer = [0u8; MAX_PACKET];
            let mut reported: Vec<Found> = Vec::new();

            while !stop.load(Ordering::Relaxed) {
                if last_announce.is_none_or(|t| t.elapsed() >= ANNOUNCE_EVERY) {
                    last_announce = Some(Instant::now());
                    for target in broadcast_targets() {
                        // A network that refuses broadcast is simply not
                        // searched; typing the address still works there.
                        if let Err(e) = socket.send_to(&announce, target) {
                            debug!("discovery: cannot announce to {target}: {e}");
                        }
                    }
                }

                if let Ok((len, from)) = socket.recv_from(&mut buffer) {
                    if let Some((their_name, their_port)) = parse(&buffer[..len], id) {
                        let addr = SocketAddr::new(from.ip(), their_port);
                        seen.insert(addr, (their_name, Instant::now()));
                    }
                }

                seen.retain(|_, (_, at)| at.elapsed() < FORGET_AFTER);
                let now: Vec<Found> = seen
                    .iter()
                    .map(|(addr, (name, _))| Found {
                        name: name.clone(),
                        addr: *addr,
                    })
                    .collect();
                if now != reported {
                    reported = now.clone();
                    report(now);
                }
            }
        })
        .context("cannot spawn the discovery thread")?;
    Ok(vec![thread])
}

/// Read an announcement. `None` for this PC's own, another version's, or
/// anything that is not an announcement at all.
fn parse(packet: &[u8], own_id: u64) -> Option<(String, u16)> {
    let announce: Announce = serde_json::from_slice(packet).ok()?;
    if announce.t != "Announce" || announce.proto != PROTO || announce.id == own_id {
        return None;
    }
    let name: String = announce
        .name
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_NAME)
        .collect();
    (announce.port != 0).then(|| (name.trim().to_string(), announce.port))
}

/// Every local IPv4 network's broadcast address, and the all-networks one.
///
/// Windows sends 255.255.255.255 out of one interface only, so each network's
/// own broadcast address is used as well; that is what reaches a second
/// network card with a cable to the other laptop.
fn broadcast_targets() -> Vec<SocketAddr> {
    let mut targets: Vec<SocketAddr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback())
        .filter_map(|i| match i.addr {
            if_addrs::IfAddr::V4(v4) => v4.broadcast,
            if_addrs::IfAddr::V6(_) => None,
        })
        .map(|ip| SocketAddr::new(IpAddr::V4(ip), PORT))
        .collect();
    targets.push(SocketAddr::new(IpAddr::V4(Ipv4Addr::BROADCAST), PORT));
    targets.sort();
    targets.dedup();
    targets
}

/// A number that differs between runs, without a random-number crate.
fn run_id() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or_default();
    nanos ^ (u64::from(std::process::id()) << 32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(name: &str, port: u16, proto: u32, id: u64) -> Vec<u8> {
        serde_json::to_vec(&Announce {
            t: "Announce".into(),
            name: name.into(),
            port,
            proto,
            id,
        })
        .expect("json")
    }

    #[test]
    fn another_pc_is_found_and_this_one_is_not() {
        assert_eq!(
            parse(&packet("laptop-b", 47800, PROTO, 2), 1),
            Some(("laptop-b".to_string(), 47800))
        );
        assert_eq!(parse(&packet("me", 47800, PROTO, 1), 1), None, "our own");
        assert_eq!(parse(&packet("new", 47800, PROTO + 1, 2), 1), None);
        assert_eq!(parse(b"hello", 1), None);
    }

    #[test]
    fn a_name_cannot_carry_control_characters() {
        let found = parse(&packet("evil\n\u{7}pc", 47800, PROTO, 2), 1).expect("parsed");
        assert!(!found.0.chars().any(char::is_control));
    }
}
