//! `lmx welcome`: the summary an interactive shell shows when it starts.

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
/// Columns available after the two-column page indent.
const WIDTH: usize = 78;
/// Columns reserved for guest details.
const LEFT: usize = 42;
/// Columns separating guest details and modules.
const GAP: usize = 4;
/// Columns reserved for selected modules.
const RIGHT: usize = WIDTH - LEFT - GAP;
/// Columns of a label inside the guest column.
const DETAIL: usize = 8;
/// Maximum number of module names before an ellipsis.
const MODULE_LIMIT: usize = 20;
/// Warning threshold when the configuration, and with it the platform policy, is unreadable.
const DEFAULT_MINIMUM_PERCENT: u8 = 10;
/// How long the welcome waits for `lmxd`; a shell must start quickly.
const OWNER_TIMEOUT: Duration = Duration::from_millis(500);
/// Characters of a generation identifier that the welcome shows.
const GENERATION: usize = 7;
/// Name prefixes of container and virtual interfaces, whose addresses the Mac does not use.
const VIRTUAL: [&str; 11] = [
    "docker", "br-", "veth", "virbr", "cni", "flannel", "cali", "vxlan", "kube", "lxc", "podman",
];
/// The intertwined X mark.
const LOGO: [&str; 4] = ["╲ ╲  ╱ ╱", " ╲ ╲╱ ╱", " ╱ ╱╲ ╲", "╱ ╱  ╲ ╲"];
/// Commands below the summary, with their descriptions.
const COMMANDS: [(&str, &str); 3] = [
    ("lmx help", "Explore commands"),
    ("nix profile add nixpkgs#ripgrep", "Install a package"),
    ("nix profile remove ripgrep", "Remove a package"),
];

/// Everything the welcome shows.
#[derive(Debug)]
struct Facts {
    /// VM name.
    name: String,
    /// Operating system and architecture, such as `NixOS 26.05, arm64`.
    system: String,
    /// Selected catalog modules, or `None` when the configuration cannot be read.
    modules: Option<Vec<String>>,
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
}

/// A rendered line and its width before color sequences were added.
#[derive(Debug)]
struct Line {
    /// Text, possibly containing color sequences.
    text: String,
    /// Visible terminal columns.
    width: usize,
}

impl Line {
    /// Pairs visible text with its styled representation.
    fn new(plain: &str, text: String) -> Self {
        Self {
            text,
            width: columns(plain),
        }
    }

    /// Text in one color, measured before styling.
    fn colored(paint: Paint, color: Color, text: &str) -> Self {
        Self::new(text, paint.color(color, text))
    }

    /// An empty line between groups.
    fn blank() -> Self {
        Self::new("", String::new())
    }
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
            Some(selected(&config.modules)),
            config.disk.minimum_percent,
            Some(config.generation.clone()),
            Some(config.network.ports.clone()),
        ),
        Err(_) => (
            "unknown".to_owned(),
            "workspace metadata cannot be read".to_owned(),
            None,
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
    }
}

/// The selected modules in declaration order, without the catalog's `lmx:` prefix.
fn selected(modules: &[String]) -> Vec<String> {
    modules
        .iter()
        .map(|module| module.strip_prefix("lmx:").unwrap_or(module).to_owned())
        .collect()
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

/// Renders the welcome.
fn render(facts: &Facts, paint: Paint) -> String {
    let palette = paint.palette();
    let mut text = String::from("\n");
    let attention = attention(facts);
    let left = guest(facts, paint, &attention);
    let right = modules(facts, paint);
    for index in 0..left.len().max(right.len()) {
        let left = left.get(index);
        let right = right.get(index);
        let value = left.map_or("", |line| line.text.as_str());
        if let Some(right) = right.filter(|line| !line.text.is_empty()) {
            let padding = LEFT + GAP - left.map_or(0, |line| line.width);
            text.push_str(&format!("  {value}{:padding$}{}\n", "", right.text));
        } else if value.is_empty() {
            text.push('\n');
        } else {
            text.push_str(&format!("  {value}\n"));
        }
    }
    if !attention.is_empty() {
        text.push('\n');
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
    text.push_str(&format!(
        "\n  {}\n\n",
        paint.color(palette.muted, &"─".repeat(WIDTH))
    ));
    for (command, description) in COMMANDS {
        text.push_str(&format!(
            "  {}  {}\n",
            paint.color(palette.blue, &format!("{command:<35}")),
            paint.color(palette.subtle, description)
        ));
    }
    text.push('\n');
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

/// The left column: branding, state and the facts known about the guest.
fn guest(facts: &Facts, paint: Paint, attention: &[Attention]) -> Vec<Line> {
    let palette = paint.palette();
    let mut lines = Vec::new();
    let captions = [
        "LimaNix",
        "https://github.com/limanix",
        "https://limanix.dev",
        "",
    ];
    for (mark, caption) in LOGO.into_iter().zip(captions) {
        let color = if caption == "LimaNix" {
            palette.blue
        } else {
            palette.subtle
        };
        let plain = if caption.is_empty() {
            mark.to_owned()
        } else {
            format!("{mark:<8}  {caption}")
        };
        let text = if caption.is_empty() {
            paint.color(palette.blue, mark)
        } else {
            format!(
                "{}  {}",
                paint.color(palette.blue, &format!("{mark:<8}")),
                paint.color(color, caption)
            )
        };
        lines.push(Line::new(&plain, text));
    }
    lines.push(Line::blank());

    let (word, value, color) = settled(facts, paint);
    let (mark, word, color) = attention.first().map_or(("●", word, color), |item| {
        ("▲", item.word, palette.yellow)
    });
    let state = format!("{mark} {word}");
    if columns(&facts.name) + 2 + columns(&state) <= LEFT {
        lines.push(Line::new(
            &format!("{}  {state}", facts.name),
            format!(
                "{}  {}",
                paint.bold(&facts.name),
                paint.color(color, &state)
            ),
        ));
    } else {
        for name in wrap(&facts.name, LEFT) {
            lines.push(Line::new(&name, paint.bold(&name)));
        }
        lines.push(Line::colored(paint, color, &state));
    }
    extend(&mut lines, paint, palette.subtle, &facts.system, LEFT);
    if attention.is_empty() {
        extend(&mut lines, paint, palette.muted, &value, LEFT);
    }

    if facts.cpus.is_some()
        || facts.memory.is_some()
        || facts.uptime.is_some()
        || facts.disk.is_some()
    {
        group(&mut lines, paint, "SYSTEM");
    }
    if let Some(cpus) = facts.cpus {
        detail(&mut lines, paint, "CPU", &cpus.to_string(), None);
    }
    if let Some(memory) = facts.memory {
        detail(&mut lines, paint, "RAM", &gibibytes(memory), None);
    }
    if let Some(uptime) = facts.uptime {
        detail(&mut lines, paint, "Uptime", &span(uptime), None);
    }
    if let Some(usage) = facts.disk {
        let (free, total) = (gibibytes(usage.available_bytes), gibibytes(usage.bytes));
        let free = free.strip_suffix(" GiB").unwrap_or(&free);
        let color = (!disk_short(facts).is_empty()).then_some(palette.yellow);
        detail(
            &mut lines,
            paint,
            "Disk",
            &format!("{free} / {total} free"),
            color,
        );
    }
    let network = network(facts);
    if !network.is_empty() {
        group(&mut lines, paint, "NETWORK");
        for value in network {
            extend(&mut lines, paint, palette.subtle, &value, LEFT);
        }
    }
    shared(&mut lines, facts, paint);
    lines
}

/// A section heading with a blank line before it.
fn group(lines: &mut Vec<Line>, paint: Paint, name: &str) {
    lines.push(Line::blank());
    lines.push(Line::colored(paint, paint.palette().blue, name));
}

/// Wrapped text measured before applying a color.
fn extend(lines: &mut Vec<Line>, paint: Paint, color: Color, value: &str, width: usize) {
    lines.extend(
        wrap(value, width)
            .iter()
            .map(|line| Line::colored(paint, color, line)),
    );
}

/// A labeled fact within the left column.
fn detail(lines: &mut Vec<Line>, paint: Paint, name: &str, value: &str, color: Option<Color>) {
    let mut name = name;
    for value in wrap(value, LEFT - DETAIL) {
        let plain = format!("{name:<DETAIL$}{value}");
        let text = format!(
            "{}{}",
            paint.color(paint.palette().muted, &format!("{name:<DETAIL$}")),
            color.map_or_else(|| value.clone(), |color| paint.color(color, &value))
        );
        lines.push(Line::new(&plain, text));
        name = "";
    }
}

/// The right column, capped by module count rather than wrapped line count.
fn modules(facts: &Facts, paint: Paint) -> Vec<Line> {
    let palette = paint.palette();
    let total = facts
        .modules
        .as_ref()
        .map_or(String::new(), |modules| format!(" {}", modules.len()));
    let mut lines = vec![
        Line::new(
            &format!("MODULES{total}"),
            format!(
                "{}{}",
                paint.color(palette.blue, "MODULES"),
                paint.color(palette.muted, &total)
            ),
        ),
        Line::blank(),
    ];
    match &facts.modules {
        None => lines.push(Line::colored(paint, palette.subtle, "unknown")),
        Some(modules) if modules.is_empty() => {
            lines.push(Line::colored(paint, palette.subtle, "none"))
        }
        Some(modules) => {
            for module in modules.iter().take(MODULE_LIMIT) {
                for line in wrap(module, RIGHT) {
                    lines.push(Line::new(&line, line.clone()));
                }
            }
            if modules.len() > MODULE_LIMIT {
                lines.push(Line::colored(paint, palette.muted, "..."));
            }
        }
    }
    lines
}

/// Guest addresses and firewall ports on separate lines; unreadable facts are omitted.
fn network(facts: &Facts) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(addresses) = &facts.addresses {
        if addresses.is_empty() {
            lines.push("no IPv4 address".into());
        } else {
            lines.extend(addresses.iter().cloned());
        }
    }
    if let Some(ports) = &facts.ports {
        if ports.tcp.is_empty() && ports.udp.is_empty() {
            lines.push("no open ports".into());
        }
        for (protocol, numbers) in [("TCP", &ports.tcp), ("UDP", &ports.udp)] {
            if !numbers.is_empty() {
                let numbers: Vec<String> = numbers.iter().map(u16::to_string).collect();
                lines.push(format!("{protocol} {}", numbers.join(", ")));
            }
        }
    }
    lines
}

/// Items as a phrase: `a`, `a and b`, `a, b and c`.
pub(crate) fn listing(items: &[String]) -> String {
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
fn shared(lines: &mut Vec<Line>, facts: &Facts, paint: Paint) {
    group(lines, paint, "SHARED");
    let Some(mounts) = &facts.mounts else {
        extend(
            lines,
            paint,
            paint.palette().subtle,
            "unavailable; run lmx info",
            LEFT,
        );
        return;
    };
    if mounts.is_empty() {
        lines.push(Line::colored(paint, paint.palette().subtle, "none"));
        return;
    }
    let width = mounts
        .iter()
        .map(|mount| columns(&mount.target))
        .max()
        .unwrap_or_default()
        .min(LEFT - 4);
    for mount in mounts {
        let (mode, color) = if mount.read_only {
            ("ro", paint.palette().orange)
        } else {
            ("rw", paint.palette().green)
        };
        for (index, line) in wrap(&mount.target, width).into_iter().enumerate() {
            if index == 0 {
                let padding = width.saturating_sub(columns(&line));
                let plain = format!("{line}{:padding$}  {mode}", "");
                lines.push(Line::new(
                    &plain,
                    format!("{line}{:padding$}  {}", "", paint.color(color, mode)),
                ));
            } else {
                lines.push(Line::new(&line, line.clone()));
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
            modules: Some(vec!["go".into(), "docker".into(), "python-3.12".into()]),
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
        }
    }

    #[test]
    fn shows_a_healthy_vm() {
        let text = render(&facts(), Paint::plain());
        assert!(text.starts_with("\n  ╲ ╲  ╱ ╱  LimaNix"));
        for value in [
            "https://github.com/limanix",
            "https://limanix.dev",
            "dev-box  ● ready",
            "NixOS 26.05, arm64",
            "generation 0123456, updated 2 days ago",
            "CPU     4",
            "RAM     7.6 GiB",
            "Uptime  3 h",
            "Disk    39 / 64 GiB free",
            "192.168.105.4",
            "TCP 8080, 5432",
            "/home/dev     rw",
            "/mnt/limanix  ro",
            "MODULES 3",
        ] {
            assert!(text.contains(value), "{value}: {text}");
        }
        for name in ["go", "docker", "python-3.12"] {
            let line = text
                .lines()
                .find(|line| line.ends_with(name))
                .expect("module row");
            assert_eq!(columns(&line[..line.len() - name.len()]), 48, "{line}");
        }
        let footer = text.split(&"─".repeat(WIDTH)).nth(1).expect("separator");
        for command in [
            "lmx help",
            "nix profile add nixpkgs#ripgrep",
            "nix profile remove ripgrep",
        ] {
            assert!(footer.contains(command), "{footer}");
        }
        assert!(!footer.contains("lmx status"));
        assert!(!footer.contains("lmx doctor"));
        assert!(!text.contains("  tip  "));
    }

    #[test]
    fn keeps_actionable_warnings_below_the_columns() {
        let mut facts = facts();
        facts.owner = Some(owner(&[RESTART_REQUIRED, DEGRADED], false));
        facts.built = Some("4a1f9c2d0e11".into());
        facts.disk = facts.disk.map(|disk| DiskUsage {
            free_inodes: 40_000,
            ..disk
        });
        facts.failed_units = vec!["postgresql.service".into()];
        let text = render(&facts, Paint::plain());
        let state: Vec<&str> = text
            .lines()
            .filter(|line| line.starts_with("  ▲ "))
            .collect();
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
        assert!(render(&facts, Paint::plain()).contains("dev-box  ● applying"));
        assert!(render(&facts, Paint::plain()).contains("a new generation is being built"));
        assert!(!render(&facts, Paint::plain()).contains("run limanix update"));
        facts.owner = Some(owner(&[], false));
        assert!(render(&facts, Paint::plain()).contains("dev-box  ● checking"));
        assert!(render(&facts, Paint::plain()).contains("generation 0123456 is being checked"));
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
            network(&facts),
            [
                "192.168.5.15",
                "192.168.105.4",
                "TCP 22, 8080, 5432",
                "UDP 53"
            ]
        );
        facts.addresses = Some(vec![]);
        facts.ports = Some(Ports::default());
        assert_eq!(network(&facts), ["no IPv4 address", "no open ports"]);
        facts.addresses = None;
        facts.ports = None;
        assert!(network(&facts).is_empty());
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
        assert!(render(&facts, Paint::plain()).contains("  SHARED\n  none\n"));
        facts.mounts = None;
        assert!(render(&facts, Paint::plain()).contains("  SHARED\n  unavailable; run lmx info\n"));
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
        for missing in ["SYSTEM", "Disk", "NETWORK", "Uptime", "updated"] {
            assert!(!text.contains(missing), "{missing}: {text}");
        }
        assert!(text.contains("dev-box  ● ready"));
        assert!(text.contains("generation 0123456"));
        assert!(text.contains("NixOS 26.05, arm64"));
    }

    #[test]
    fn wraps_long_values_within_eighty_columns() {
        let mut facts = facts();
        facts.name = "long-vm-name-".repeat(10);
        facts.modules = Some(vec!["界".repeat(50); 21]);
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
    fn caps_the_module_count_and_only_then_adds_an_ellipsis() {
        let mut facts = facts();
        for count in [0, 20, 21] {
            facts.modules = Some((1..=count).map(|n| format!("module-{n:02}")).collect());
            let text = render(&facts, Paint::plain());
            assert_eq!(text.matches("module-").count(), count.min(20), "{text}");
            assert_eq!(text.contains("..."), count > 20, "{text}");
            assert_eq!(text.contains("module-20"), count >= 20, "{text}");
            assert!(!text.contains("module-21"), "{text}");
            assert!(text.contains(&format!("MODULES {count}")));
            if count == 0 {
                assert!(text.contains("none"));
            }
        }
        facts.modules = None;
        let text = render(&facts, Paint::plain());
        assert!(text.contains("unknown"));
        assert!(!text.contains("MODULES 0"));
    }

    #[test]
    fn keeps_module_order_and_only_strips_the_lmx_prefix() {
        assert_eq!(
            selected(&[
                "lmx:go".into(),
                "third-party:custom".into(),
                "lmx:docker".into()
            ]),
            ["go", "third-party:custom", "docker"]
        );
    }

    #[test]
    fn counts_long_module_names_once_even_when_they_wrap() {
        let mut facts = facts();
        facts.modules = Some(
            (1..=21)
                .map(|n| format!("{}-{n:02}", "界".repeat(40)))
                .collect(),
        );
        let text = render(&facts, Paint::plain());
        assert!(text.contains("-20"));
        assert!(!text.contains("-21"));
        assert!(text.contains("..."));
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
        let mut plain = String::new();
        let mut chars = text.chars();
        while let Some(character) = chars.next() {
            if character == '\x1b' {
                for character in chars.by_ref() {
                    if character == 'm' {
                        break;
                    }
                }
            } else {
                plain.push(character);
            }
        }
        assert_eq!(
            plain,
            render(&facts(), Paint::plain()),
            "color must not shift columns"
        );
    }
}
