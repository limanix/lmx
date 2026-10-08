//! Sockets.
//!
//! The kernel lists the sockets of each protocol in `/proc/net/{tcp,tcp6,udp,udp6}`, which every
//! user can read. A socket belongs to the processes with a descriptor of its inode in
//! `/proc/<pid>/fd`; only root can read the descriptors of other users' processes.

use std::{
    fs, io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    path::Path,
};

use lmx_model::Protocol;

use crate::FactError;

/// Root of the process file system inside a booted guest.
pub const PROC_PATH: &str = "/proc";

/// TCP state of a socket that accepts connections.
const TCP_LISTEN: u8 = 0x0A;

/// One socket that waits for connections or datagrams.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listener {
    /// Local address; `0.0.0.0` or `::` for every address.
    pub address: IpAddr,
    /// Local port.
    pub port: u16,
    /// User that created the socket.
    pub uid: u32,
    /// Inode that identifies the socket in `/proc/<pid>/fd`.
    pub inode: u64,
}

/// A process that holds a socket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Holder {
    /// Process ID.
    pub pid: u32,
    /// Command name, from `/proc/<pid>/comm`.
    pub command: String,
    /// Inode of the socket it holds.
    pub inode: u64,
}

/// Listeners of `protocol` on `port`, from the socket tables below `proc`.
///
/// A table that does not exist, such as `tcp6` without IPv6, has no listeners.
pub fn listeners(proc: &Path, protocol: Protocol, port: u16) -> Result<Vec<Listener>, FactError> {
    let tables = match protocol {
        Protocol::Tcp => ["tcp", "tcp6"],
        Protocol::Udp => ["udp", "udp6"],
    };
    let mut found = Vec::new();
    for name in tables {
        let table = match fs::read_to_string(proc.join("net").join(name)) {
            Ok(table) => table,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(source) => {
                return Err(FactError::Io {
                    what: "the socket table",
                    source,
                });
            }
        };
        found.extend(
            parse(&table, protocol)?
                .into_iter()
                .filter(|listener| listener.port == port),
        );
    }
    Ok(found)
}

/// Parses a socket table, keeping TCP sockets in `LISTEN` and UDP sockets without a peer.
pub fn parse(table: &str, protocol: Protocol) -> Result<Vec<Listener>, FactError> {
    let mut listeners = Vec::new();
    for line in table.lines().skip(1).filter(|line| !line.trim().is_empty()) {
        let malformed = || FactError::Parse {
            what: "socket table",
            detail: line.to_owned(),
        };
        let fields: Vec<&str> = line.split_whitespace().collect();
        let (Some(local), Some(remote), Some(state), Some(uid), Some(inode)) = (
            fields.get(1),
            fields.get(2),
            fields.get(3),
            fields.get(7),
            fields.get(9),
        ) else {
            return Err(malformed());
        };
        let state = u8::from_str_radix(state, 16).map_err(|_| malformed())?;
        let (_, remote_port) = endpoint(remote).ok_or_else(malformed)?;
        let waiting = match protocol {
            Protocol::Tcp => state == TCP_LISTEN,
            Protocol::Udp => remote_port == 0,
        };
        if !waiting {
            continue;
        }
        let (address, port) = endpoint(local).ok_or_else(malformed)?;
        listeners.push(Listener {
            address,
            port,
            uid: uid.parse().map_err(|_| malformed())?,
            inode: inode.parse().map_err(|_| malformed())?,
        });
    }
    Ok(listeners)
}

/// Decodes `ADDRESS:PORT` of a socket table.
fn endpoint(field: &str) -> Option<(IpAddr, u16)> {
    let (address, port) = field.split_once(':')?;
    let port = u16::from_str_radix(port, 16).ok()?;
    let mut bytes = Vec::with_capacity(16);
    for word in address.as_bytes().chunks(8) {
        let word = u32::from_str_radix(std::str::from_utf8(word).ok()?, 16).ok()?;
        bytes.extend_from_slice(&word.to_ne_bytes());
    }
    let address = match bytes.len() {
        4 => IpAddr::V4(Ipv4Addr::from(<[u8; 4]>::try_from(bytes).ok()?)),
        16 => IpAddr::V6(Ipv6Addr::from(<[u8; 16]>::try_from(bytes).ok()?)),
        _ => return None,
    };
    Some((address, port))
}

/// The processes below `proc` that hold any of the sockets `inodes`, in one search, and whether
/// every process could be searched.
pub fn holders(proc: &Path, inodes: &[u64]) -> Result<(Vec<Holder>, bool), FactError> {
    let targets: Vec<(String, u64)> = inodes
        .iter()
        .map(|inode| (format!("socket:[{inode}]"), *inode))
        .collect();
    let entries = fs::read_dir(proc).map_err(|source| FactError::Io {
        what: "the process list",
        source,
    })?;
    let mut holders = Vec::new();
    let mut complete = true;
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let descriptors = match fs::read_dir(entry.path().join("fd")) {
            Ok(descriptors) => descriptors,
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                complete = false;
                continue;
            }
            Err(_) => continue,
        };
        let mut held: Vec<u64> = descriptors
            .flatten()
            .filter_map(|descriptor| fs::read_link(descriptor.path()).ok())
            .filter_map(|link| {
                targets
                    .iter()
                    .find(|(target, _)| link.as_os_str() == target.as_str())
                    .map(|(_, inode)| *inode)
            })
            .collect();
        held.sort_unstable();
        held.dedup();
        if held.is_empty() {
            continue;
        }
        let command = fs::read_to_string(entry.path().join("comm"))
            .map(|command| command.trim_end().to_owned())
            .unwrap_or_default();
        holders.extend(held.into_iter().map(|inode| Holder {
            pid,
            command: command.clone(),
            inode,
        }));
    }
    holders.sort_by_key(|holder| (holder.pid, holder.inode));
    Ok((holders, complete))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    const TCP: &str = "\
  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 4242 1 0 100 0 0 10 0
   1: 00000000:0016 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 11870 1 0 100 0 0 10 0
   2: 0A00020F:0016 0100020A:C350 01 00000000:00000000 02:00000AD7 00000000     0        0 12345 4 0 20 4 30 10 -1
";

    const TCP6: &str = "\
  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000001000000:1F90 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 4243 1 0 100 0 0 10 0
   1: 00000000000000000000000000000000:0016 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 11872 1 0 100 0 0 10 0
";

    #[test]
    fn keeps_listening_sockets_with_their_addresses() {
        let listeners = parse(TCP, Protocol::Tcp).expect("valid table");
        assert_eq!(
            listeners,
            [
                Listener {
                    address: IpAddr::from([127, 0, 0, 1]),
                    port: 8080,
                    uid: 1000,
                    inode: 4242,
                },
                Listener {
                    address: IpAddr::from([0, 0, 0, 0]),
                    port: 22,
                    uid: 0,
                    inode: 11870,
                },
            ]
        );
        let listeners = parse(TCP6, Protocol::Tcp).expect("valid table");
        assert_eq!(listeners[0].address, IpAddr::V6(Ipv6Addr::LOCALHOST));
        assert_eq!(listeners[1].address, IpAddr::V6(Ipv6Addr::UNSPECIFIED));
    }

    #[test]
    fn a_udp_socket_without_a_peer_waits_for_datagrams() {
        let table = "\
  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode ref pointer drops
  1: 00000000:0044 00000000:0000 07 00000000:00000000 00:00000000 00000000     0        0 9861 2 0 0
  2: 0100007F:0035 0100007F:D431 01 00000000:00000000 00:00000000 00000000     0        0 9862 2 0 0
";
        let listeners = parse(table, Protocol::Udp).expect("valid table");
        assert_eq!(listeners.len(), 1);
        assert_eq!(listeners[0].port, 68);
    }

    #[test]
    fn finds_the_processes_that_hold_a_socket() {
        let proc = tempfile::tempdir().expect("temporary /proc");
        for (pid, command, socket) in [("4242", "python3", 4242), ("7", "sshd", 11870)] {
            let fd = proc.path().join(pid).join("fd");
            fs::create_dir_all(&fd).expect("create fd");
            symlink(format!("socket:[{socket}]"), fd.join("3")).expect("link the socket");
            symlink("/dev/null", fd.join("0")).expect("link stdin");
            fs::write(proc.path().join(pid).join("comm"), format!("{command}\n")).expect("comm");
        }
        fs::create_dir_all(proc.path().join("net")).expect("create net");
        fs::write(proc.path().join("net/tcp"), TCP).expect("write the table");

        let listeners = listeners(proc.path(), Protocol::Tcp, 8080).expect("readable");
        assert_eq!(listeners.len(), 1);
        let (holders, complete) = holders(proc.path(), &[listeners[0].inode]).expect("readable");
        assert!(complete);
        assert_eq!(
            holders,
            [Holder {
                pid: 4242,
                command: "python3".into(),
                inode: 4242,
            }]
        );
    }
}
