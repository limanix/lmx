//! `lmx welcome`: the summary an interactive shell shows when it starts.
//!
//! Below the logo comes the state of the guest: one line when nothing needs attention, or one line
//! for each thing that does, with the command that helps. Then the VM, resources, disk, network,
//! modules and shared folders, a tip when nothing needs attention, and the next commands. Labels
//! take 11 columns after a two-column indent, and values wrap so no line is wider than 80 columns.
//! Facts are read first, and rendering is a pure function of them.

use std::{
    io,
    process::ExitCode,
    time::{Duration, SystemTime},
};

use lmx_facts::{
    disk, generations, machine,
    mounts::{self, Mount},
    network, units,
};
use lmx_model::{
    CONVERGED, DEGRADED, DISK_LOW, DiskUsage, FINALIZE_FAILED, Interface, OUT_OF_DATE, Owner,
    Ports, RESTART_REQUIRED,
};

use crate::{
    format::gibibytes,
    layout::{columns, wrap},
    output, owner,
    palette::{Color, Paint, Palette},
    system::System,
};

/// Columns of the label after the indent.
const LABEL: usize = 11;
/// Columns left for a value: 80 minus the indent and the label.
const VALUE: usize = 67;
/// Widest shared-folder target before it wraps, leaving room for the mode.
const TARGET: usize = 63;
/// Columns of each logo line drawn in blue; the rest is mauve.
const LOGO_SPLIT: usize = 36;
/// Warning threshold when the configuration, and with it the platform policy, is unreadable.
const DEFAULT_MINIMUM_PERCENT: u8 = 10;
/// How long the welcome waits for `lmxd`; a shell must start quickly.
const OWNER_TIMEOUT: Duration = Duration::from_millis(500);
/// Cells of the disk bar.
const BAR: usize = 20;
/// Characters of a generation identifier that the welcome shows.
const GENERATION: usize = 7;
/// Name prefixes of container and virtual interfaces, whose addresses the Mac does not use.
const VIRTUAL: [&str; 11] = [
    "docker", "br-", "veth", "virbr", "cni", "flannel", "cali", "vxlan", "kube", "lxc", "podman",
];
/// Tips that hold in every guest: a command, then what it does.
const TIPS: [(&str, &str); 3] = [
    ("pbcopy < FILE", " copies a file to your Mac clipboard."),
    (
        "lmx net check PORT",
        " explains why the Mac cannot reach a port.",
    ),
    (
        "lmx doctor",
        " finds what is wrong with the guest and what to do.",
    ),
];

/// The LimaNix logo.
const LOGO: [&str; 8] = [
    "888      d8b                        888b    888 d8b",
    "888      Y8P                        8888b   888 Y8P",
    "888                                 88888b  888",
    "888      888 88888b.d88b.   8888b.  888Y88b 888 888 888  888",
    r#"888      888 888 "888 "88b     "88b 888 Y88b888 888 `Y8bd8P'"#,
    "888      888 888  888  888 .d888888 888  Y88888 888   X88K",
    r#"888      888 888  888  888 888  888 888   Y8888 888 .d8""8b."#,
    r#"88888888 888 888  888  888 "Y888888 888    Y888 888 888  888"#,
];

/// Everything the welcome shows.
#[derive(Debug)]
struct Facts {
    /// VM name.
    name: String,
    /// Operating system and architecture, such as `NixOS 26.05, arm64`.
    system: String,
    /// Selected catalog modules, or `none`.
    modules: String,
    /// Free share below which the disk warning appears, in percent.
    minimum_percent: u8,
    /// Generation of the running configuration, when readable.
    generation: Option<String>,
    /// Generation built into the system profile, from its marker.
    built: Option<String>,
    /// Time since the system profile last moved to another generation.
    updated: Option<Duration>,
    /// Time since boot, when readable.
    uptime: Option<Duration>,
    /// Processors, when readable.
    cpus: Option<usize>,
    /// Total memory in bytes, when readable.
    memory: Option<u64>,
    /// Guest disk usage, when readable.
    disk: Option<DiskUsage>,
    /// Global IPv4 addresses of the guest's own interfaces, or `None` when they cannot be listed.
    addresses: Option<Vec<String>>,
    /// Ports the guest firewall opens, when the configuration is readable.
    ports: Option<Ports>,
    /// Shared folders, or `None` when the mount table is unreadable.
    mounts: Option<Vec<Mount>>,
    /// Failed units; empty when there are none or they cannot be listed.
    failed_units: Vec<String>,
    /// State of `lmxd`, or `None` when it does not answer in time.
    owner: Option<Owner>,
    /// Tip for when nothing needs attention: a command, then what it does.
    tip: Option<(String, String)>,
}

/// One thing that needs attention, and the command that helps.
#[derive(Debug, PartialEq, Eq)]
struct Attention {
    /// Short name of the problem, at most nine columns.
    word: &'static str,
    /// What is wrong.
    text: String,
    /// The command to run.
    command: String,
    /// Words after the command, such as where to run it.
    after: &'static str,
}

impl Attention {
    /// The value of the line: what is wrong, then the command.
    fn value(&self) -> String {
        format!("{}; run {}{}", self.text, self.command, self.after)
    }
}

/// Runs `lmx welcome`; fails when the shared folders cannot be listed, as the shell welcome did.
pub(crate) fn run(system: &System) -> io::Result<ExitCode> {
    let facts = collect(system);
    output::write_text(&render(
        &facts,
        Paint::detect(Palette::of_config(&system.config)),
    ))?;
    Ok(output::page_status(facts.mounts.is_some()))
}

/// Reads the facts the welcome shows; each one is best effort.
fn collect(system: &System) -> Facts {
    let (name, os, modules, minimum_percent, generation, ports) = match &system.config {
        Ok(config) => (
            config.vm.name.clone(),
            format!("{}, {}", config.vm.system, config.vm.arch),
            selected(&config.modules),
            config.disk.minimum_percent,
            Some(config.generation.clone()),
            Some(config.network.ports.clone()),
        ),
        Err(_) => (
            "unknown".to_owned(),
            "workspace metadata cannot be read".to_owned(),
            "unknown".to_owned(),
            DEFAULT_MINIMUM_PERCENT,
            None,
            None,
        ),
    };
    let now = SystemTime::now();
    let (markers, _) = generations::read(&system.generation_paths());
    Facts {
        name,
        system: os,
        modules,
        minimum_percent,
        generation,
        built: markers.built,
        updated: generations::profile_changed(&system.system_profile())
            .ok()
            .and_then(|changed| now.duration_since(changed).ok()),
        uptime: machine::uptime(&system.uptime()).ok(),
        cpus: machine::cpus().ok(),
        memory: machine::memory(&system.meminfo()).ok(),
        disk: disk::usage(&system.store()).ok(),
        addresses: network::interfaces(&system.ip())
            .ok()
            .map(|interfaces| addresses(&interfaces)),
        ports,
        mounts: mounts::shared(&system.mountinfo()).ok(),
        failed_units: units::failed(&system.systemctl()).unwrap_or_default(),
        owner: owner::status_within(&system.owner_socket(), OWNER_TIMEOUT).ok(),
        tip: tip(system, now),
    }
}

/// The selected modules without the catalog's `lmx:` prefix, or `none`.
fn selected(modules: &[String]) -> String {
    if modules.is_empty() {
        return "none".to_owned();
    }
    modules
        .iter()
        .map(|module| module.strip_prefix("lmx:").unwrap_or(module))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Addresses of the guest's own interfaces, leaving out container and virtual networks.
fn addresses(interfaces: &[Interface]) -> Vec<String> {
    interfaces
        .iter()
        .filter(|interface| {
            !VIRTUAL
                .iter()
                .any(|prefix| interface.name.starts_with(prefix))
        })
        .flat_map(|interface| interface.ipv4.iter().cloned())
        .collect()
}

/// A tip that changes from shell to shell; every other one points to a selected module's card.
fn tip(system: &System, now: SystemTime) -> Option<(String, String)> {
    let seed = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| {
            usize::try_from(since.subsec_nanos()).unwrap_or(0)
        });
    let module = system.help().ok().and_then(|help| {
        let modules = system
            .config
            .as_ref()
            .map_or(&[][..], |config| config.modules.as_slice());
        modules
            .iter()
            .find_map(|module| help.find(module).map(|(topic, _)| topic.to_owned()))
            .or_else(|| help.topics.keys().next().cloned())
    });
    if let Some(topic) = module
        && seed.is_multiple_of(2)
    {
        return Some((
            format!("lmx help {topic}"),
            format!(" shows what the {topic} module gives you."),
        ));
    }
    let (command, text) = TIPS[seed / 2 % TIPS.len()];
    Some((command.to_owned(), text.to_owned()))
}

/// Renders the welcome.
fn render(facts: &Facts, paint: Paint) -> String {
    let palette = paint.palette();
    let mut text = String::from("\n");
    for line in LOGO {
        let (left, right) = line.split_at(LOGO_SPLIT.min(line.len()));
        text.push_str(&format!(
            "  {}{}\n",
            paint.color(palette.blue, left),
            paint.color(palette.mauve, right)
        ));
    }
    text.push('\n');

    let attention = attention(facts);
    if attention.is_empty() {
        let (word, value, color) = settled(facts, paint);
        state(&mut text, paint, ("●", word, color), &value, None);
    }
    for item in &attention {
        state(
            &mut text,
            paint,
            ("▲", item.word, palette.yellow),
            &item.value(),
            Some(&item.command),
        );
    }
    text.push('\n');

    vm(&mut text, facts, paint);
    let resources = resources(facts);
    if !resources.is_empty() {
        row(&mut text, paint, "Resources", &resources, None);
    }
    if let Some(usage) = facts.disk {
        disk(&mut text, facts, usage, paint);
    }
    if let Some(network) = network(facts) {
        row(&mut text, paint, "Network", &network, None);
    }
    row(&mut text, paint, "Modules", &facts.modules, None);
    shared(&mut text, facts, paint);
    text.push('\n');

    if attention.is_empty()
        && let Some((command, rest)) = &facts.tip
    {
        text.push_str(&format!(
            "  {}  {}{rest}\n",
            paint.color(palette.muted, "tip"),
            paint.color(palette.blue, command)
        ));
    }
    text.push_str(&format!(
        "  {}{}{}{}{}{}\n\n",
        paint.color(palette.blue, "lmx help"),
        paint.color(palette.muted, " for commands, "),
        paint.color(palette.blue, "lmx status"),
        paint.color(palette.muted, " for details, "),
        paint.color(palette.blue, "exit"),
        paint.color(palette.muted, " to return to the Mac.")
    ));
    text
}

/// What needs attention, most urgent first.
fn attention(facts: &Facts) -> Vec<Attention> {
    let holds = |kind: &str| {
        facts.owner.as_ref().is_some_and(|owner| {
            owner
                .conditions
                .iter()
                .any(|condition| condition.kind == kind)
        })
    };
    let item = |word, text: String, command: String, after| Attention {
        word,
        text,
        command,
        after,
    };
    let mut items = Vec::new();
    if facts.owner.is_none() {
        items.push(item(
            "owner",
            "lmxd does not answer".into(),
            "lmx doctor".into(),
            "",
        ));
    }
    if holds(DEGRADED) {
        items.push(item(
            "degraded",
            "the health check failed".into(),
            "lmx doctor".into(),
            "",
        ));
    }
    match facts.failed_units.as_slice() {
        [] => {}
        [unit] => {
            // A command that wraps cannot be copied in one go; the list of failed units can.
            let status = format!("systemctl status {unit}");
            let command = if columns(&format!("{unit}; run {status}")) <= VALUE {
                status
            } else {
                "systemctl --failed".to_owned()
            };
            items.push(item("failed", unit.clone(), command, ""));
        }
        units => items.push(item(
            "failed",
            listing(units),
            "systemctl --failed".into(),
            "",
        )),
    }
    let short = disk_short(facts);
    if !short.is_empty() || holds(DISK_LOW) {
        let text = if short.is_empty() {
            "the guest disk is nearly full".to_owned()
        } else {
            format!("{} free on the guest disk", short.join(" and "))
        };
        items.push(item("disk low", text, "sudo lmx store reserve".into(), ""));
    }
    if holds(RESTART_REQUIRED) {
        let text = facts.built.as_deref().map_or_else(
            || "a new generation is built".to_owned(),
            |built| format!("generation {} is built", short_generation(built)),
        );
        items.push(item(
            "restart",
            text,
            "limanix update".into(),
            " on the Mac",
        ));
    }
    if holds(OUT_OF_DATE) && !facts.owner.as_ref().is_some_and(applying) {
        items.push(item(
            "update",
            "a new generation is waiting".into(),
            "limanix update".into(),
            " on the Mac",
        ));
    }
    if holds(FINALIZE_FAILED) {
        items.push(item(
            "cleanup",
            "removing older generations failed".into(),
            "sudo lmx logs finalize".into(),
            "",
        ));
    }
    items
}

/// The state line when nothing needs attention: its word, value and color.
fn settled(facts: &Facts, paint: Paint) -> (&'static str, String, Color) {
    let palette = paint.palette();
    let generation = facts
        .generation
        .as_deref()
        .map_or("unknown", short_generation);
    let converged = facts.owner.as_ref().is_some_and(|owner| {
        owner
            .conditions
            .iter()
            .any(|condition| condition.kind == CONVERGED)
    });
    if facts.owner.as_ref().is_some_and(applying) {
        (
            "applying",
            "a new generation is being built".to_owned(),
            palette.blue,
        )
    } else if converged {
        let updated = facts
            .updated
            .map(|age| format!(", updated {}", ago(age)))
            .unwrap_or_default();
        (
            "ready",
            format!("generation {generation}{updated}"),
            palette.green,
        )
    } else {
        (
            "checking",
            format!("generation {generation} is being checked"),
            palette.blue,
        )
    }
}

/// Whether `lmxd` is building a generation.
fn applying(owner: &Owner) -> bool {
    owner
        .operations
        .iter()
        .any(|operation| operation.kind == "SystemApply")
}

/// The first characters of a generation identifier.
fn short_generation(generation: &str) -> &str {
    generation
        .char_indices()
        .nth(GENERATION)
        .map_or(generation, |(end, _)| &generation[..end])
}

/// A state line: the mark and word in their color, then the value with the command in blue.
fn state(
    text: &mut String,
    paint: Paint,
    (mark, word, color): (&str, &str, Color),
    value: &str,
    command: Option<&str>,
) {
    let mut head = paint.color(color, &format!("{:<LABEL$}", format!("{mark} {word}")));
    for line in wrap(value, VALUE) {
        let line = match command {
            Some(command) if line.contains(command) => {
                line.replacen(command, &paint.color(paint.palette().blue, command), 1)
            }
            _ => line,
        };
        text.push_str(&format!("  {head}{line}\n"));
        head = " ".repeat(LABEL);
    }
}

/// The label column, muted.
pub(crate) fn label(paint: Paint, label: &str) -> String {
    paint.color(paint.palette().muted, &format!("{label:<LABEL$}"))
}

/// Writes `label` and `value`; further lines of the value keep the value column.
pub(crate) fn row(text: &mut String, paint: Paint, name: &str, value: &str, color: Option<Color>) {
    let mut name = name;
    for line in wrap(value, VALUE) {
        let line = color.map_or_else(|| line.clone(), |color| paint.color(color, &line));
        text.push_str(&format!("  {}{line}\n", label(paint, name)));
        name = "";
    }
}

/// The VM name in bold, its system and uptime, on one line when they fit.
fn vm(text: &mut String, facts: &Facts, paint: Paint) {
    let up = facts
        .uptime
        .map(|uptime| format!(", up {}", span(uptime)))
        .unwrap_or_default();
    let system = format!("({}){up}", facts.system);
    if columns(&facts.name) + 1 + columns(&system) <= VALUE {
        text.push_str(&format!(
            "  {}{} {}\n",
            label(paint, "VM"),
            paint.bold(&facts.name),
            paint.color(paint.palette().subtext, &system)
        ));
    } else {
        row(text, paint, "VM", &facts.name, None);
        row(text, paint, "", &system, Some(paint.palette().subtext));
    }
}

/// Processors and memory; a part that cannot be read is left out.
fn resources(facts: &Facts) -> String {
    let mut parts = Vec::new();
    match facts.cpus {
        Some(1) => parts.push("1 CPU".to_owned()),
        Some(count) => parts.push(format!("{count} CPUs")),
        None => {}
    }
    if let Some(memory) = facts.memory {
        parts.push(format!("{} memory", gibibytes(memory)));
    }
    parts.join(", ")
}

/// The free share of the guest disk as a bar, then in numbers.
fn disk(text: &mut String, facts: &Facts, usage: DiskUsage, paint: Paint) {
    let palette = paint.palette();
    let (free, total) = (
        u128::from(usage.available_bytes),
        u128::from(usage.bytes.max(1)),
    );
    let filled = usize::try_from((free * BAR as u128 + total / 2) / total)
        .unwrap_or(BAR)
        .min(BAR);
    let percent = free * 100 / total;
    let color = if percent < u128::from(facts.minimum_percent) {
        palette.yellow
    } else {
        palette.green
    };
    let (free, total) = (gibibytes(usage.available_bytes), gibibytes(usage.bytes));
    let free = free.strip_suffix(" GiB").unwrap_or(&free);
    text.push_str(&format!(
        "  {}{}{}  {percent}% free, {free} of {total}\n",
        label(paint, "Disk"),
        paint.color(color, &"█".repeat(filled)),
        paint.color(palette.muted, &"░".repeat(BAR - filled)),
    ));
}

/// The guest's addresses and the ports its firewall opens; `None` when neither is known.
fn network(facts: &Facts) -> Option<String> {
    let addresses = facts.addresses.as_ref().map(|addresses| {
        if addresses.is_empty() {
            "no IPv4 address".to_owned()
        } else {
            listing(addresses)
        }
    });
    let ports = facts.ports.as_ref().map(|ports| {
        let groups: Vec<String> = [("tcp", &ports.tcp), ("udp", &ports.udp)]
            .into_iter()
            .filter(|(_, numbers)| !numbers.is_empty())
            .map(|(protocol, numbers)| {
                let numbers: Vec<String> = numbers.iter().map(u16::to_string).collect();
                format!("{protocol} {}", listing(&numbers))
            })
            .collect();
        if groups.is_empty() {
            "no open ports".to_owned()
        } else {
            format!("ports {}", groups.join(", "))
        }
    });
    let parts: Vec<String> = [addresses, ports].into_iter().flatten().collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

/// Items as a phrase: `a`, `a and b`, `a, b and c`.
fn listing(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// A duration in its largest whole unit, such as `3 min`, `5 h` or `2 days`.
fn span(duration: Duration) -> String {
    let minutes = duration.as_secs() / 60;
    match minutes {
        0 => "under a minute".to_owned(),
        1..60 => format!("{minutes} min"),
        60..1440 => format!("{} h", minutes / 60),
        1440..2880 => "1 day".to_owned(),
        _ => format!("{} days", minutes / 1440),
    }
}

/// How long ago something happened that is `age` old.
fn ago(age: Duration) -> String {
    if age < Duration::from_secs(60) {
        "just now".to_owned()
    } else {
        format!("{} ago", span(age))
    }
}

/// Shared folders with their mode, aligned after the longest target.
fn shared(text: &mut String, facts: &Facts, paint: Paint) {
    let Some(mounts) = &facts.mounts else {
        row(text, paint, "Shared", "unavailable; run lmx info", None);
        return;
    };
    if mounts.is_empty() {
        row(text, paint, "Shared", "(none)", None);
        return;
    }
    let width = mounts
        .iter()
        .map(|mount| columns(&mount.target))
        .max()
        .unwrap_or_default()
        .min(TARGET);
    let mut name = "Shared";
    for mount in mounts {
        let (mode, color) = if mount.read_only {
            ("ro", paint.palette().peach)
        } else {
            ("rw", paint.palette().green)
        };
        for (index, line) in wrap(&mount.target, width).into_iter().enumerate() {
            if index == 0 {
                let padding = width.saturating_sub(columns(&line));
                text.push_str(&format!(
                    "  {}{line}{:padding$}  {}\n",
                    label(paint, name),
                    "",
                    paint.color(color, mode)
                ));
                name = "";
            } else {
                text.push_str(&format!("  {:LABEL$}{line}\n", ""));
            }
        }
    }
}

/// The parts of the guest disk below the platform minimum, such as `4% of inodes`.
fn disk_short(facts: &Facts) -> Vec<String> {
    let Some(disk) = facts.disk else {
        return Vec::new();
    };
    let minimum = u128::from(facts.minimum_percent);
    [
        (disk.available_bytes, disk.bytes, "space"),
        (disk.free_inodes, disk.inodes, "inodes"),
    ]
    .into_iter()
    .filter(|(free, total, _)| *total > 0 && u128::from(*free) * 100 < u128::from(*total) * minimum)
    .map(|(free, total, what)| format!("{}% of {what}", u128::from(free) * 100 / u128::from(total)))
    .collect()
}

#[cfg(test)]
mod tests {
    use lmx_model::{Condition, Operation};

    use super::*;

    const KIB: u64 = 1024;

    fn owner(kinds: &[&str], applying: bool) -> Owner {
        Owner {
            version: "0.1.0".into(),
            conditions: kinds
                .iter()
                .map(|kind| Condition {
                    kind: (*kind).into(),
                    message: String::new(),
                })
                .collect(),
            operations: applying
                .then(|| Operation {
                    task: "system-apply-1".into(),
                    kind: "SystemApply".into(),
                    phase: "running".into(),
                    created_at: 0,
                })
                .into_iter()
                .collect(),
        }
    }

    fn facts() -> Facts {
        Facts {
            name: "dev-box".into(),
            system: "NixOS 26.05, arm64".into(),
            modules: "go, docker, python-3.12".into(),
            minimum_percent: 10,
            generation: Some("0123456789ab".into()),
            built: Some("0123456789ab".into()),
            updated: Some(Duration::from_secs(2 * 86_400 + 600)),
            uptime: Some(Duration::from_secs(3 * 3600 + 120)),
            cpus: Some(4),
            memory: Some(7_969_124 * KIB),
            disk: Some(DiskUsage {
                bytes: 64 << 30,
                free_bytes: 43 << 30,
                available_bytes: 39 << 30,
                inodes: 1_000_000,
                free_inodes: 500_000,
            }),
            addresses: Some(vec!["192.168.105.4".into()]),
            ports: Some(Ports {
                tcp: vec![8080, 5432],
                udp: vec![],
            }),
            mounts: Some(vec![
                Mount {
                    target: "/home/dev".into(),
                    fs_type: "virtiofs".into(),
                    read_only: false,
                },
                Mount {
                    target: "/mnt/limanix".into(),
                    fs_type: "virtiofs".into(),
                    read_only: true,
                },
            ]),
            failed_units: vec![],
            owner: Some(owner(&[CONVERGED], false)),
            tip: Some((
                "lmx help go".into(),
                " shows what the go module gives you.".into(),
            )),
        }
    }

    fn body(text: &str) -> &str {
        let logo_end = text.find(LOGO[7]).expect("logo") + LOGO[7].len() + 1;
        &text[logo_end..]
    }

    #[test]
    fn shows_a_healthy_vm() {
        let text = render(&facts(), Paint::plain());
        assert!(text.starts_with("\n  888      d8b"));
        assert_eq!(
            body(&text),
            "\n  ● ready    generation 0123456, updated 2 days ago\n\
             \n  VM         dev-box (NixOS 26.05, arm64), up 3 h\
             \n  Resources  4 CPUs, 7.6 GiB memory\
             \n  Disk       ████████████░░░░░░░░  60% free, 39 of 64 GiB\
             \n  Network    192.168.105.4, ports tcp 8080 and 5432\
             \n  Modules    go, docker, python-3.12\
             \n  Shared     /home/dev     rw\
             \n             /mnt/limanix  ro\n\
             \n  tip  lmx help go shows what the go module gives you.\
             \n  lmx help for commands, lmx status for details, exit to return to the Mac.\n\n"
        );
    }

    #[test]
    fn lists_what_needs_attention_in_place_of_the_tip() {
        let mut facts = facts();
        facts.owner = Some(owner(&[RESTART_REQUIRED, DEGRADED], false));
        facts.built = Some("4a1f9c2d0e11".into());
        facts.disk = facts.disk.map(|disk| DiskUsage {
            free_inodes: 40_000,
            ..disk
        });
        facts.failed_units = vec!["postgresql.service".into()];
        let text = render(&facts, Paint::plain());
        let state: Vec<&str> = body(&text).lines().skip(1).take(4).collect();
        assert_eq!(
            state,
            [
                "  ▲ degraded the health check failed; run lmx doctor",
                "  ▲ failed   postgresql.service; run systemctl status postgresql.service",
                "  ▲ disk low 4% of inodes free on the guest disk; run sudo lmx store reserve",
                "  ▲ restart  generation 4a1f9c2 is built; run limanix update on the Mac",
            ]
        );
        assert!(!text.contains("tip"), "{text}");
    }

    #[test]
    fn says_when_lmxd_does_not_answer_and_still_checks_the_guest() {
        let mut facts = facts();
        facts.owner = None;
        facts.failed_units = vec!["a.service".into(), "b.service".into()];
        let text = render(&facts, Paint::plain());
        assert!(
            text.contains(
                "\n  ▲ owner    lmxd does not answer; run lmx doctor\
                 \n  ▲ failed   a.service and b.service; run systemctl --failed\n"
            ),
            "{text}"
        );
    }

    #[test]
    fn shows_an_update_in_progress() {
        let mut facts = facts();
        facts.owner = Some(owner(&[OUT_OF_DATE], true));
        assert!(
            render(&facts, Paint::plain())
                .contains("\n  ● applying a new generation is being built\n")
        );
        facts.owner = Some(owner(&[], false));
        assert!(
            render(&facts, Paint::plain())
                .contains("\n  ● checking generation 0123456 is being checked\n")
        );
    }

    #[test]
    fn describes_the_network() {
        let mut facts = facts();
        facts.addresses = Some(vec!["192.168.5.15".into(), "192.168.105.4".into()]);
        facts.ports = Some(Ports {
            tcp: vec![22, 8080, 5432],
            udp: vec![53],
        });
        assert_eq!(
            network(&facts).as_deref(),
            Some("192.168.5.15 and 192.168.105.4, ports tcp 22, 8080 and 5432, udp 53")
        );
        facts.addresses = Some(vec![]);
        facts.ports = Some(Ports::default());
        assert_eq!(
            network(&facts).as_deref(),
            Some("no IPv4 address, no open ports")
        );
        facts.addresses = None;
        facts.ports = None;
        assert_eq!(network(&facts), None);
    }

    #[test]
    fn leaves_out_container_interfaces() {
        let interface = |name: &str, address: &str| Interface {
            name: name.into(),
            mac: None,
            ipv4: vec![address.into()],
        };
        assert_eq!(
            addresses(&[
                interface("enp0s1", "192.168.105.4"),
                interface("docker0", "172.17.0.1"),
                interface("br-1a2b3c", "172.18.0.1"),
            ]),
            ["192.168.105.4"]
        );
    }

    #[test]
    fn explains_missing_shared_folders() {
        let mut facts = facts();
        facts.mounts = Some(vec![]);
        assert!(render(&facts, Paint::plain()).contains("  Shared     (none)\n"));
        facts.mounts = None;
        assert!(
            render(&facts, Paint::plain()).contains("  Shared     unavailable; run lmx info\n")
        );
    }

    #[test]
    fn leaves_out_facts_that_cannot_be_read() {
        let mut facts = facts();
        facts.cpus = None;
        facts.memory = None;
        facts.disk = None;
        facts.uptime = None;
        facts.updated = None;
        facts.addresses = None;
        facts.ports = None;
        let text = render(&facts, Paint::plain());
        for missing in ["Resources", "Disk", "Network", "up ", "updated"] {
            assert!(!text.contains(missing), "{missing}: {text}");
        }
        assert!(text.contains("  ● ready    generation 0123456\n"));
        assert!(text.contains("  VM         dev-box (NixOS 26.05, arm64)\n"));
    }

    #[test]
    fn wraps_long_values_within_eighty_columns() {
        let mut facts = facts();
        facts.name = "long-vm-name-".repeat(10);
        facts.modules = format!("{}git", "console, ".repeat(12));
        facts.mounts = Some(vec![
            Mount {
                target: format!("/workspace/{}", "project-".repeat(16)),
                fs_type: "virtiofs".into(),
                read_only: false,
            },
            Mount {
                target: format!("/workspace/{}", "界".repeat(50)),
                fs_type: "9p".into(),
                read_only: true,
            },
            Mount {
                target: "/workspace/a $literal 'quote'".into(),
                fs_type: "virtiofs".into(),
                read_only: false,
            },
        ]);
        facts.failed_units = vec![
            format!("{}a.service", "unit-".repeat(12)),
            format!("{}b.service", "unit-".repeat(12)),
        ];
        facts.ports = Some(Ports {
            tcp: (8000..8030).collect(),
            udp: vec![],
        });
        let text = render(&facts, Paint::plain());
        assert!(text.contains("'quote'"));
        for line in text.lines() {
            assert!(columns(line) <= 80, "{} columns: {line:?}", columns(line));
            assert_eq!(line.trim_end(), line, "trailing spaces: {line:?}");
        }
    }

    #[test]
    fn every_tip_fits_on_one_line() {
        for (command, text) in TIPS {
            let line = format!("  tip  {command}{text}");
            assert!(columns(&line) <= 80, "{line}");
        }
    }

    #[test]
    fn measures_time_in_its_largest_unit() {
        let minutes = |count: u64| Duration::from_secs(count * 60);
        assert_eq!(ago(Duration::from_secs(30)), "just now");
        assert_eq!(ago(minutes(5)), "5 min ago");
        assert_eq!(span(minutes(3 * 60 + 59)), "3 h");
        assert_eq!(span(minutes(36 * 60)), "1 day");
        assert_eq!(span(minutes(5 * 1440)), "5 days");
        assert_eq!(span(Duration::from_secs(10)), "under a minute");
    }

    #[test]
    fn colors_only_when_asked() {
        assert!(!render(&facts(), Paint::plain()).contains('\x1b'));
        let text = render(&facts(), Paint::colored());
        assert!(text.contains("\x1b[1mdev-box\x1b[0m"));
        assert!(text.contains("\x1b[38;2;250;179;135mro\x1b[0m"));
    }
}
