//! Network interfaces and their global IPv4 addresses.
//!
//! The host selects the address it shows in `limanix list` by matching hardware addresses with the
//! VM's shared network. The guest therefore reports every interface with its hardware address and
//! leaves the choice to the host.

use std::{net::Ipv4Addr, path::Path};

use lmx_model::Interface;
use serde::Deserialize;

use crate::{FactError, command};

/// Reads interfaces with `ip -j address show`.
pub fn interfaces(ip: &Path) -> Result<Vec<Interface>, FactError> {
    let output = command::output(ip, &["-j", "address", "show"])?;
    parse(&output)
}

/// One link in `ip -j address show` output.
#[derive(Deserialize)]
struct Link {
    /// Kernel interface name.
    ifname: String,
    /// Hardware address, or the endpoint of an IP tunnel; absent for some virtual links.
    #[serde(default)]
    address: Option<String>,
    /// Protocol addresses.
    #[serde(default)]
    addr_info: Vec<Address>,
}

/// One protocol address of a link.
#[derive(Deserialize)]
struct Address {
    /// `inet` or `inet6`.
    family: String,
    /// `global`, `link`, `host` and so on.
    #[serde(default)]
    scope: String,
    /// Address without prefix length.
    #[serde(default)]
    local: String,
}

/// Parses `ip -j address show` output, keeping global-scope IPv4 addresses only.
pub fn parse(output: &[u8]) -> Result<Vec<Interface>, FactError> {
    let links: Vec<Link> = serde_json::from_slice(output).map_err(|error| FactError::Parse {
        what: "ip address",
        detail: error.to_string(),
    })?;

    Ok(links
        .into_iter()
        .map(|link| Interface {
            name: link.ifname,
            mac: link
                .address
                .filter(|address| is_hardware_address(address))
                .map(|mac| mac.to_ascii_lowercase()),
            ipv4: link
                .addr_info
                .into_iter()
                .filter(|address| address.family == "inet" && address.scope == "global")
                .filter_map(|address| address.local.parse::<Ipv4Addr>().ok())
                .map(|address| address.to_string())
                .collect(),
        })
        .collect())
}

/// Whether `address` is a hardware address: two-digit hexadecimal octets separated by colons.
///
/// IP tunnels such as `ipip` and `sit` report their local endpoint in the same field, such as
/// `0.0.0.0`.
fn is_hardware_address(address: &str) -> bool {
    address
        .split(':')
        .all(|octet| octet.len() == 2 && octet.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_global_ipv4_with_hardware_addresses() {
        let output = br#"[
            {"ifname": "lo", "address": "00:00:00:00:00:00",
             "addr_info": [{"family": "inet", "scope": "host", "local": "127.0.0.1"}]},
            {"ifname": "enp0s1", "address": "52:55:55:AA:BB:CC",
             "addr_info": [
                {"family": "inet", "scope": "global", "local": "192.0.2.10"},
                {"family": "inet6", "scope": "global", "local": "2001:db8::10"},
                {"family": "inet", "scope": "link", "local": "169.254.1.2"}
             ]},
            {"ifname": "wg0", "addr_info": [{"family": "inet", "scope": "global", "local": "10.0.0.2"}]}
        ]"#;

        let interfaces = parse(output).expect("valid ip output");
        assert_eq!(
            interfaces,
            vec![
                Interface {
                    name: "lo".into(),
                    mac: Some("00:00:00:00:00:00".into()),
                    ipv4: vec![]
                },
                Interface {
                    name: "enp0s1".into(),
                    mac: Some("52:55:55:aa:bb:cc".into()),
                    ipv4: vec!["192.0.2.10".into()],
                },
                Interface {
                    name: "wg0".into(),
                    mac: None,
                    ipv4: vec!["10.0.0.2".into()]
                },
            ]
        );
    }

    #[test]
    fn reports_no_hardware_address_for_ip_tunnels() {
        let output = br#"[
            {"ifname": "tunl0", "link_type": "ipip", "address": "0.0.0.0"},
            {"ifname": "ip6tnl0", "link_type": "tunnel6", "address": "::"}
        ]"#;

        let interfaces = parse(output).expect("valid ip output");
        assert_eq!(interfaces.len(), 2);
        assert!(
            interfaces.iter().all(|interface| interface.mac.is_none()),
            "{interfaces:?}"
        );
    }

    #[test]
    fn rejects_other_output() {
        let error = parse(b"Device \"eth0\" does not exist.").expect_err("not JSON");
        assert!(
            error
                .to_string()
                .starts_with("unexpected ip address output"),
            "{error}"
        );
    }
}
