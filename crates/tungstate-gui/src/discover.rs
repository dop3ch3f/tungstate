//! File servers on this network, for the add-connection form to start from.
//!
//! Two ways of looking, run side by side:
//! - **mDNS**: servers that announce themselves (`_smb._tcp`, `_ftp._tcp`, …)
//!   with their name. Most NAS boxes do.
//! - **A port knock** on the local /24 for the ones that don't: a TCP connect
//!   to 445 (SMB), 21 (FTP) and 22 (SFTP), nothing sent. HTTP ports are left
//!   out on purpose: every router and printer answers on 80, and calling them
//!   `WebDAV` servers would be wrong more often than right.
//!
//! Finds are merged by address, so one box offering SMB and FTP is one row.
//! Only private IPv4 networks are knocked on, and only the /24 around this
//! device's own address, so the work is bounded whatever the netmask says.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

/// How long mDNS is listened to. Announcing servers answer within a second;
/// the rest is for a slow Wi-Fi.
const LISTEN: Duration = Duration::from_secs(4);
/// One knock. A LAN host answers in milliseconds; a missing one never does.
const KNOCK: Duration = Duration::from_millis(400);
/// Knocks in flight at once: 254 hosts × 3 ports in about five seconds.
const KNOCKERS: usize = 64;

/// What one service type means as a connection kind.
const SERVICES: &[(&str, &str)] = &[
    ("_smb._tcp.local.", "smb"),
    ("_ftp._tcp.local.", "ftp"),
    ("_sftp-ssh._tcp.local.", "sftp"),
    ("_ssh._tcp.local.", "sftp"),
    ("_webdav._tcp.local.", "webdav"),
    ("_webdavs._tcp.local.", "webdavs"),
];

/// The ports knocked on, and the kind each means.
const PORTS: &[(u16, &str)] = &[(445, "smb"), (21, "ftp"), (22, "sftp")];

/// One thing a server offers: a kind of connection, on a port.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Offer {
    /// `smb`, `ftp`, `sftp`, `webdav` or `webdavs` (`WebDAV` over HTTPS).
    pub kind: String,
    pub port: u16,
}

/// One server, by address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Found {
    pub address: String,
    /// What it calls itself over mDNS, when it said.
    pub name: Option<String>,
    /// Its `.local` host name, when it said.
    pub host: Option<String>,
    /// Sorted, one per kind.
    pub offers: Vec<Offer>,
}

/// One sighting, before merging.
#[derive(Debug, Clone)]
pub struct Hit {
    pub ip: Ipv4Addr,
    pub name: Option<String>,
    pub host: Option<String>,
    pub kind: &'static str,
    pub port: u16,
}

/// Folds one sighting into what is known. Returns the server's row when it
/// changed, so only real news reaches the window.
///
/// mDNS wins over a knock for the port: a server may announce FTP on 2121,
/// and the knock only ever tries 21.
pub fn merge(known: &mut BTreeMap<Ipv4Addr, Found>, hit: Hit) -> Option<Found> {
    let row = known.entry(hit.ip).or_insert_with(|| Found {
        address: hit.ip.to_string(),
        name: None,
        host: None,
        offers: Vec::new(),
    });
    let before = row.clone();
    let announced = hit.name.is_some();
    if row.name.is_none() {
        row.name = hit.name;
    }
    if row.host.is_none() {
        row.host = hit.host.map(|h| h.trim_end_matches('.').to_owned());
    }
    match row.offers.iter_mut().find(|o| o.kind == hit.kind) {
        Some(offer) if announced => offer.port = hit.port,
        Some(_) => {}
        None => row.offers.push(Offer {
            kind: hit.kind.to_owned(),
            port: hit.port,
        }),
    }
    row.offers.sort();
    (*row != before).then(|| row.clone())
}

/// The addresses to knock on: the /24 around each private IPv4 address this
/// device has, less the device itself.
#[must_use]
pub fn neighbours(own: &[Ipv4Addr]) -> Vec<Ipv4Addr> {
    let mut out: Vec<Ipv4Addr> = own
        .iter()
        .filter(|ip| ip.is_private())
        .flat_map(|ip| {
            let [a, b, c, _] = ip.octets();
            (1..=254).map(move |d| Ipv4Addr::new(a, b, c, d))
        })
        .filter(|ip| !own.contains(ip))
        .collect();
    out.sort();
    out.dedup();
    out
}

fn own_addresses() -> Vec<Ipv4Addr> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback())
        .filter_map(|i| match i.ip() {
            IpAddr::V4(ip) => Some(ip),
            IpAddr::V6(_) => None,
        })
        .collect()
}

/// Looks for servers for a few seconds, calling `seen` with each row as it
/// appears or gains an offer, and returns everything found.
///
/// Blocking: run it off the window's thread.
pub fn look(seen: impl Fn(&Found)) -> Vec<Found> {
    let (tx, rx) = mpsc::channel::<Hit>();

    let announced = {
        let tx = tx.clone();
        std::thread::spawn(move || listen(&tx))
    };
    let knocked = std::thread::spawn(move || knock(&neighbours(&own_addresses()), &tx));

    let mut known = BTreeMap::new();
    // Every sender is dropped once both lookers finish, which ends this loop.
    for hit in rx {
        if let Some(row) = merge(&mut known, hit) {
            seen(&row);
        }
    }
    let _ = announced.join();
    let _ = knocked.join();
    known.into_values().collect()
}

fn listen(tx: &mpsc::Sender<Hit>) {
    let Ok(daemon) = mdns_sd::ServiceDaemon::new() else {
        tracing::warn!("mDNS is not available here; only knocking");
        return;
    };
    let browsers: Vec<_> = SERVICES
        .iter()
        .filter_map(|(ty, kind)| daemon.browse(ty).ok().map(|rx| (rx, *kind)))
        .collect();
    let until = Instant::now() + LISTEN;
    while Instant::now() < until {
        for (rx, kind) in &browsers {
            while let Ok(event) = rx.try_recv() {
                let mdns_sd::ServiceEvent::ServiceResolved(service) = event else {
                    continue;
                };
                let name = mdns_sd::instance_name_unescaped(service.get_fullname());
                for ip in service.get_addresses() {
                    if let IpAddr::V4(ip) = ip.to_ip_addr() {
                        let _ = tx.send(Hit {
                            ip,
                            name: Some(name.clone()),
                            host: Some(service.get_hostname().to_owned()),
                            kind,
                            port: service.get_port(),
                        });
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = daemon.shutdown();
}

fn knock(hosts: &[Ipv4Addr], tx: &mpsc::Sender<Hit>) {
    let work: Vec<(Ipv4Addr, u16, &'static str)> = hosts
        .iter()
        .flat_map(|ip| PORTS.iter().map(move |(port, kind)| (*ip, *port, *kind)))
        .collect();
    let queue = Arc::new(Mutex::new(work.into_iter()));
    let workers: Vec<_> = (0..KNOCKERS)
        .map(|_| {
            let queue = Arc::clone(&queue);
            let tx = tx.clone();
            std::thread::spawn(move || {
                loop {
                    // Held only to take the next address, never across a knock.
                    let next = queue.lock().ok().and_then(|mut q| q.next());
                    let Some((ip, port, kind)) = next else { break };
                    let at = SocketAddr::new(IpAddr::V4(ip), port);
                    if TcpStream::connect_timeout(&at, KNOCK).is_ok() {
                        let _ = tx.send(Hit {
                            ip,
                            name: None,
                            host: None,
                            kind,
                            port,
                        });
                    }
                }
            })
        })
        .collect();
    for w in workers {
        let _ = w.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(ip: [u8; 4], name: Option<&str>, kind: &'static str, port: u16) -> Hit {
        Hit {
            ip: Ipv4Addr::from(ip),
            name: name.map(str::to_owned),
            host: name.map(|n| format!("{n}.local.")),
            kind,
            port,
        }
    }

    #[test]
    fn one_server_offering_two_kinds_is_one_row() {
        let mut known = BTreeMap::new();
        assert!(
            merge(
                &mut known,
                hit([192, 168, 31, 71], Some("Area51"), "smb", 445)
            )
            .is_some()
        );
        let row = merge(
            &mut known,
            hit([192, 168, 31, 71], Some("Area51"), "ftp", 21),
        )
        .unwrap();
        assert_eq!(known.len(), 1);
        assert_eq!(row.name.as_deref(), Some("Area51"));
        assert_eq!(row.host.as_deref(), Some("Area51.local"));
        let kinds: Vec<_> = row.offers.iter().map(|o| o.kind.as_str()).collect();
        assert_eq!(kinds, ["ftp", "smb"]);
    }

    #[test]
    fn a_knock_after_an_announcement_is_not_news() {
        let mut known = BTreeMap::new();
        merge(&mut known, hit([10, 0, 0, 5], Some("nas"), "smb", 445));
        assert!(merge(&mut known, hit([10, 0, 0, 5], None, "smb", 445)).is_none());
    }

    #[test]
    fn an_announced_port_replaces_the_knocked_default() {
        let mut known = BTreeMap::new();
        merge(&mut known, hit([10, 0, 0, 5], None, "ftp", 21));
        let row = merge(&mut known, hit([10, 0, 0, 5], Some("nas"), "ftp", 2121)).unwrap();
        assert_eq!(
            row.offers,
            [Offer {
                kind: "ftp".into(),
                port: 2121
            }]
        );
        assert_eq!(row.name.as_deref(), Some("nas"));
    }

    #[test]
    fn a_nameless_server_keeps_its_address() {
        let mut known = BTreeMap::new();
        let row = merge(&mut known, hit([192, 168, 1, 40], None, "smb", 445)).unwrap();
        assert_eq!(row.address, "192.168.1.40");
        assert!(row.name.is_none());
    }

    #[test]
    fn neighbours_are_the_private_slash_24_without_ourselves() {
        let around = neighbours(&[Ipv4Addr::new(192, 168, 31, 28)]);
        assert_eq!(around.len(), 253);
        assert!(!around.contains(&Ipv4Addr::new(192, 168, 31, 28)));
        assert!(around.contains(&Ipv4Addr::new(192, 168, 31, 71)));
    }

    #[test]
    fn public_addresses_are_never_knocked_on() {
        let around = neighbours(&[Ipv4Addr::new(8, 8, 8, 8), Ipv4Addr::new(100, 64, 0, 1)]);
        assert_eq!(around, Vec::<Ipv4Addr>::new());
    }

    /// Looks at whatever network this machine is on and prints what it
    /// finds. By hand only: CI has no NAS, and its network is not ours.
    #[test]
    #[ignore = "looks at the real network; run with --ignored --nocapture"]
    fn looks_at_this_network() {
        let found = look(|row| eprintln!("seen {row:?}"));
        eprintln!("{} found", found.len());
    }

    #[test]
    fn two_interfaces_on_one_network_knock_once() {
        let around = neighbours(&[Ipv4Addr::new(10, 1, 2, 3), Ipv4Addr::new(10, 1, 2, 4)]);
        assert_eq!(around.len(), 252);
    }
}
