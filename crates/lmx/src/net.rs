//! `lmx net check PORT`: why a port of the VM may be unreachable from the Mac.
//!
//! It automates the guest side of the guide "Check a connection in order": the firewall rule and
//! its protocol, the listener and its address, and the process behind it. The VM's address and a
//! connection attempt from the Mac belong to `limanix net check` on the host.

use std::{collections::BTreeSet, fs, io, net::IpAddr, process::ExitCode};

use lmx_facts::{
    FactError,
    sockets::{self, Holder, Listener},
};
use lmx_model::{Check, CheckStatus, Envelope, NetCheck, Ports, Protocol};

use crate::{
    findings, output,
    palette::{Paint, Palette},
    system::System,
};

/// Command name of Docker's proxy, which holds the sockets of ports Docker publishes.
const DOCKER_PROXY: &str = "docker-proxy";

/// Runs `lmx net check PORT`.
pub(crate) fn check(
    system: &System,
    port: u16,
    protocol: Protocol,
    json: bool,
) -> io::Result<ExitCode> {
    let checks = checks(system, port, protocol);
    if json {
        output::write_json(&Envelope::success(NetCheck {
            port,
            protocol,
            checks: checks.clone(),
        }))?;
        return Ok(findings::status(&checks));
    }
    findings::write(&checks, Paint::detect(Palette::of_config(&system.config)))
}

/// The checks of `port`, in the order of the guide.
fn checks(system: &System, port: u16, protocol: Protocol) -> Vec<Check> {
    let ports = system
        .config
        .as_ref()
        .map(|config| &config.network.ports)
        .map_err(Clone::clone);
    let listeners = sockets::listeners(&system.proc(), protocol, port);
    let held = match &listeners {
        Ok(listeners) if !listeners.is_empty() => Some(holders(system, listeners)),
        _ => None,
    };
    let docker = held.as_ref().is_some_and(|(holders, _)| {
        holders
            .iter()
            .any(|(holder, _)| holder.command == DOCKER_PROXY)
    });
    let mut checks = vec![
        firewall(ports, port, protocol, docker),
        listener(&listeners, port, protocol),
    ];
    if let Some((holders, complete)) = held {
        checks.push(process(&holders, complete, port, protocol, |uid| {
            user_name(system, uid)
        }));
    }
    checks
}

/// Whether the guest firewall opens `port` for `protocol`, from the declared ports; a port that
/// Docker publishes passes Docker's own rules instead.
fn firewall(ports: Result<&Ports, String>, port: u16, protocol: Protocol, docker: bool) -> Check {
    let name = protocol.name();
    if docker {
        return Check::new(
            "firewall",
            CheckStatus::Ok,
            format!(
                "Docker publishes {name} {port} with its own rules, outside the guest firewall."
            ),
        );
    }
    let ports = match ports {
        Ok(ports) => ports,
        Err(error) => {
            return Check::new(
                "firewall",
                CheckStatus::Failed,
                format!("The declared ports cannot be read: {error}"),
            )
            .hint("Run lmx doctor.");
        }
    };
    let (open, other, other_name) = match protocol {
        Protocol::Tcp => (&ports.tcp, &ports.udp, Protocol::Udp.name()),
        Protocol::Udp => (&ports.udp, &ports.tcp, Protocol::Tcp.name()),
    };
    let declare = format!(
        "Add {port} to network.ports.{} in limanix.toml, then run limanix update.",
        name.to_lowercase()
    );
    if protocol == Protocol::Tcp && port == 22 {
        Check::new(
            "firewall",
            CheckStatus::Ok,
            "TCP 22 is always open for SSH.",
        )
    } else if open.contains(&port) {
        Check::new(
            "firewall",
            CheckStatus::Ok,
            format!("{name} {port} is open in the guest firewall."),
        )
    } else if other.contains(&port) {
        Check::new(
            "firewall",
            CheckStatus::Failed,
            format!("Only {other_name} {port} is open in the guest firewall, not {name}."),
        )
        .hint(declare)
    } else {
        Check::new(
            "firewall",
            CheckStatus::Failed,
            format!("{name} {port} is not open in the guest firewall."),
        )
        .hint(declare)
    }
}

/// Whether something listens on `port`, and on which addresses.
fn listener(listeners: &Result<Vec<Listener>, FactError>, port: u16, protocol: Protocol) -> Check {
    let name = protocol.name();
    let listeners = match listeners {
        Ok(listeners) => listeners,
        Err(error) => {
            return Check::new("listener", CheckStatus::Failed, error.to_string());
        }
    };
    if listeners.is_empty() {
        return Check::new(
            "listener",
            CheckStatus::Failed,
            format!("Nothing listens on {name} {port} in the guest."),
        )
        .hint("Start the application, and check the port it uses.");
    }
    let addresses: BTreeSet<IpAddr> = listeners
        .iter()
        .map(|listener| listener.address.to_canonical())
        .collect();
    let shown = addresses
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" and ");
    if addresses.iter().all(IpAddr::is_loopback) {
        Check::new(
            "listener",
            CheckStatus::Failed,
            format!(
                "{name} {port} listens on {shown} only, so it is reachable only inside the guest."
            ),
        )
        .hint("Make the application listen on 0.0.0.0 or the guest address.")
    } else {
        Check::new(
            "listener",
            CheckStatus::Ok,
            format!("{name} {port} listens on {shown}."),
        )
    }
}

/// The processes that hold any of `listeners`, each with the user that created its socket, and
/// whether every process could be searched.
fn holders(system: &System, listeners: &[Listener]) -> (Vec<(Holder, u32)>, bool) {
    let inodes: Vec<u64> = listeners.iter().map(|listener| listener.inode).collect();
    let Ok((holders, complete)) = sockets::holders(&system.proc(), &inodes) else {
        return (Vec::new(), false);
    };
    let mut found: Vec<(Holder, u32)> = holders
        .into_iter()
        .map(|holder| {
            let uid = listeners
                .iter()
                .find(|listener| listener.inode == holder.inode)
                .map_or(0, |listener| listener.uid);
            (holder, uid)
        })
        .collect();
    found.dedup_by_key(|(holder, _)| holder.pid);
    (found, complete)
}

/// Which process holds the socket; Docker's proxy means Docker publishes the port.
fn process(
    holders: &[(Holder, u32)],
    complete: bool,
    port: u16,
    protocol: Protocol,
    user_name: impl Fn(u32) -> String,
) -> Check {
    let Some((first, uid)) = holders.first() else {
        if complete {
            return Check::new(
                "process",
                CheckStatus::Ok,
                "No process holds the socket; the kernel does.",
            );
        }
        let udp = if protocol == Protocol::Udp {
            " --udp"
        } else {
            ""
        };
        return Check::new(
            "process",
            CheckStatus::Unknown,
            "No visible process holds the socket.",
        )
        .hint(format!(
            "Run sudo lmx net check {port}{udp} to see processes of other users."
        ));
    };
    let more = match holders.len() {
        1 => String::new(),
        count => format!(" and {} more", count - 1),
    };
    let owner = format!("{} (pid {}){more}", first.command, first.pid);
    let socket = format!("the socket of user {}", user_name(*uid));
    if holders
        .iter()
        .any(|(holder, _)| holder.command == DOCKER_PROXY)
    {
        Check::new(
            "process",
            CheckStatus::Warning,
            format!("Docker publishes {} {port}: {owner}.", protocol.name()),
        )
        .hint(
            "network.ports does not control ports that Docker publishes, and removing the port \
             from it does not close them.",
        )
    } else {
        Check::new(
            "process",
            CheckStatus::Ok,
            format!("{owner} holds {socket}."),
        )
    }
}

/// Name of the user `uid` from the guest's `/etc/passwd`, or `uid N`.
fn user_name(system: &System, uid: u32) -> String {
    fs::read_to_string(system.passwd())
        .ok()
        .and_then(|passwd| {
            passwd.lines().find_map(|line| {
                let mut fields = line.split(':');
                let name = fields.next()?;
                (fields.nth(1)?.parse::<u32>().ok()? == uid).then(|| name.to_owned())
            })
        })
        .unwrap_or_else(|| format!("uid {uid}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Declared ports.
    fn ports(tcp: &[u16], udp: &[u16]) -> Ports {
        Ports {
            tcp: tcp.to_vec(),
            udp: udp.to_vec(),
        }
    }

    /// A listener on `address`.
    fn on(address: [u8; 4]) -> Listener {
        Listener {
            address: IpAddr::from(address),
            port: 8080,
            uid: 1000,
            inode: 1,
        }
    }

    #[test]
    fn the_firewall_opens_a_declared_port_for_its_protocol_only() {
        let declared = ports(&[8080], &[5353]);
        let status = |port, protocol| firewall(Ok(&declared), port, protocol, false).status;
        assert_eq!(status(8080, Protocol::Tcp), CheckStatus::Ok);
        assert_eq!(status(22, Protocol::Tcp), CheckStatus::Ok);
        assert_eq!(status(5353, Protocol::Tcp), CheckStatus::Failed);
        assert_eq!(
            firewall(Ok(&declared), 5353, Protocol::Tcp, false).message,
            "Only UDP 5353 is open in the guest firewall, not TCP."
        );
        assert_eq!(status(9000, Protocol::Udp), CheckStatus::Failed);
        let docker = firewall(Ok(&declared), 9000, Protocol::Tcp, true);
        assert_eq!(docker.status, CheckStatus::Ok, "Docker's rules open it");
    }

    #[test]
    fn a_loopback_listener_is_reachable_only_inside_the_guest() {
        let status = |listeners: Vec<Listener>| listener(&Ok(listeners), 8080, Protocol::Tcp);
        assert_eq!(status(vec![]).status, CheckStatus::Failed);
        let loopback = status(vec![on([127, 0, 0, 1])]);
        assert_eq!(loopback.status, CheckStatus::Failed);
        assert_eq!(
            loopback.message,
            "TCP 8080 listens on 127.0.0.1 only, so it is reachable only inside the guest."
        );
        assert_eq!(
            status(vec![on([127, 0, 0, 1]), on([0, 0, 0, 0])]).status,
            CheckStatus::Ok
        );
    }

    #[test]
    fn names_the_process_and_flags_docker() {
        let holder = |pid, command: &str| {
            (
                Holder {
                    pid,
                    command: command.into(),
                    inode: 1,
                },
                1000,
            )
        };
        let name = |_uid| "dev".to_owned();
        let check = process(&[holder(42, "python3")], true, 8080, Protocol::Tcp, name);
        assert_eq!(
            check.message,
            "python3 (pid 42) holds the socket of user dev."
        );
        let check = process(
            &[holder(7, "docker-proxy")],
            true,
            8080,
            Protocol::Tcp,
            name,
        );
        assert_eq!(check.status, CheckStatus::Warning);
        let kernel = process(&[], true, 8080, Protocol::Tcp, name);
        assert_eq!(kernel.status, CheckStatus::Ok, "the kernel holds it");
        let check = process(&[], false, 8080, Protocol::Tcp, name);
        assert_eq!(check.status, CheckStatus::Unknown);
        assert_eq!(
            check.hint.as_deref(),
            Some("Run sudo lmx net check 8080 to see processes of other users.")
        );
    }
}
