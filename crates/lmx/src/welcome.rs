//! `lmx welcome`: the summary an interactive shell shows when it starts.
//!
//! The layout follows the platform's former shell welcome: logo, VM, resources, modules, shared
//! folders, warnings and the next commands. Labels take 11 columns after a two-column indent, and
//! values wrap so no line is wider than 80 columns. Facts are read first, and rendering is a pure
//! function of them.

use std::{io, process::ExitCode};

use lmx_facts::{
    disk, machine,
    mounts::{self, Mount},
    units,
};
use lmx_model::DiskUsage;

use crate::{
    format::gibibytes,
    help,
    layout::{columns, wrap},
    output,
    palette::{Color, Paint, Palette},
    system::System,
};

/// Columns of the label after the indent.
const LABEL: usize = 11;
/// Columns left for a value: 80 minus the indent and the label.
const VALUE: usize = 67;
/// Widest shared-folder target before it wraps, leaving room for the mode.
const TARGET: usize = 63;
/// Columns of a warning after the indent.
const WARNING: usize = 78;
/// Columns of each logo line drawn in blue; the rest is mauve.
const LOGO_SPLIT: usize = 36;
/// Warning threshold when the configuration, and with it the platform policy, is unreadable.
const DEFAULT_MINIMUM_PERCENT: u8 = 10;

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
    /// Processors, when readable.
    cpus: Option<usize>,
    /// Total memory in bytes, when readable.
    memory: Option<u64>,
    /// Guest disk usage, when readable.
    disk: Option<DiskUsage>,
    /// Shared folders, or `None` when the mount table is unreadable.
    mounts: Option<Vec<Mount>>,
    /// Failed units; empty when there are none or they cannot be listed.
    failed_units: Vec<String>,
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
    let (name, os, modules, minimum_percent) = match &system.config {
        Ok(config) => (
            config.vm.name.clone(),
            format!("{}, {}", config.vm.system, config.vm.arch),
            help::modules(config),
            config.disk.minimum_percent,
        ),
        Err(_) => (
            "unknown".to_owned(),
            "workspace metadata cannot be read".to_owned(),
            "unknown".to_owned(),
            DEFAULT_MINIMUM_PERCENT,
        ),
    };
    Facts {
        name,
        system: os,
        modules,
        minimum_percent,
        cpus: machine::cpus().ok(),
        memory: machine::memory(&system.meminfo()).ok(),
        disk: disk::usage(&system.store()).ok(),
        mounts: mounts::shared(&system.mountinfo()).ok(),
        failed_units: units::failed(&system.systemctl()).unwrap_or_default(),
    }
}

/// Renders the welcome.
fn render(facts: &Facts, paint: Paint) -> String {
    let mut text = String::from("\n");
    for line in LOGO {
        let (left, right) = line.split_at(LOGO_SPLIT.min(line.len()));
        text.push_str(&format!(
            "  {}{}\n",
            paint.color(paint.palette().blue, left),
            paint.color(paint.palette().mauve, right)
        ));
    }
    text.push('\n');

    vm(&mut text, facts, paint);
    text.push('\n');

    let resources = resources(facts);
    if !resources.is_empty() {
        row(&mut text, paint, "Resources", &resources, None);
        text.push('\n');
    }

    row(&mut text, paint, "Modules", &facts.modules, None);
    text.push('\n');

    shared(&mut text, facts, paint);
    text.push('\n');

    if let Some(warning) = disk_warning(facts) {
        warn(&mut text, paint, &warning);
    }
    if !facts.failed_units.is_empty() {
        let units = facts.failed_units.join(", ");
        warn(
            &mut text,
            paint,
            &format!("▲ Failed: {units}. Run lmx info for details."),
        );
    }

    text.push_str(&format!(
        "  {}{}{}{}{}{}\n\n",
        paint.color(paint.palette().blue, "lmx help"),
        paint.color(paint.palette().muted, " for commands, "),
        paint.color(paint.palette().blue, "lmx info"),
        paint.color(paint.palette().muted, " for details, "),
        paint.color(paint.palette().blue, "exit"),
        paint.color(paint.palette().muted, " to return to the Mac.")
    ));
    text
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

/// The VM name in bold and its system, on one line when they fit.
fn vm(text: &mut String, facts: &Facts, paint: Paint) {
    let system = format!("({})", facts.system);
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

/// Processors, memory and free disk; a part that cannot be read is left out.
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
    if let Some(disk) = facts.disk {
        parts.push(format!("{} disk free", gibibytes(disk.available_bytes)));
    }
    parts.join(", ")
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

/// The disk warning when free space or free inodes are below the platform minimum.
fn disk_warning(facts: &Facts) -> Option<String> {
    let disk = facts.disk?;
    let minimum = u128::from(facts.minimum_percent);
    let short: Vec<String> = [
        (disk.available_bytes, disk.bytes, "space"),
        (disk.free_inodes, disk.inodes, "inodes"),
    ]
    .into_iter()
    .filter(|(free, total, _)| *total > 0 && u128::from(*free) * 100 < u128::from(*total) * minimum)
    .map(|(free, total, what)| {
        format!(
            "{}% of {what} free",
            u128::from(free) * 100 / u128::from(total)
        )
    })
    .collect();
    (!short.is_empty()).then(|| {
        format!(
            "▲ Guest disk nearly full: {}. Run lmx info for details.",
            short.join(" and ")
        )
    })
}

/// Writes a warning in yellow, followed by a blank line.
fn warn(text: &mut String, paint: Paint, warning: &str) {
    for line in wrap(warning, WARNING) {
        text.push_str(&format!(
            "  {}\n",
            paint.color(paint.palette().yellow, &line)
        ));
    }
    text.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    const KIB: u64 = 1024;

    fn facts() -> Facts {
        Facts {
            name: "dev-box".into(),
            system: "NixOS 26.05, arm64".into(),
            modules: "none".into(),
            minimum_percent: 10,
            cpus: Some(4),
            memory: Some(7_969_124 * KIB),
            disk: Some(DiskUsage {
                bytes: 103_081_248 * KIB,
                free_bytes: 41_846_681 * KIB,
                available_bytes: 39_800_000 * KIB,
                inodes: 1_000_000,
                free_inodes: 500_000,
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
            "\n  VM         dev-box (NixOS 26.05, arm64)\n\
             \n  Resources  4 CPUs, 7.6 GiB memory, 38 GiB disk free\n\
             \n  Modules    none\n\
             \n  Shared     /home/dev     rw\n             /mnt/limanix  ro\n\
             \n  lmx help for commands, lmx info for details, exit to return to the Mac.\n\n"
        );
    }

    #[test]
    fn warns_about_a_nearly_full_disk_and_failed_units() {
        let mut facts = facts();
        facts.disk = facts.disk.map(|disk| DiskUsage {
            free_inodes: 40_000,
            ..disk
        });
        facts.failed_units = vec!["limanix-store-guard.service".into()];
        let text = render(&facts, Paint::plain());
        assert!(text.contains(
            "  ▲ Guest disk nearly full: 4% of inodes free. Run lmx info for details.\n\n"
        ));
        assert!(
            text.contains("  ▲ Failed: limanix-store-guard.service. Run lmx info for details.\n\n")
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
    fn leaves_out_resources_that_cannot_be_read() {
        let mut facts = facts();
        facts.cpus = None;
        facts.memory = None;
        facts.disk = None;
        let text = render(&facts, Paint::plain());
        assert!(!text.contains("Resources"));
        assert!(text.contains("  Modules    none\n"));
    }

    #[test]
    fn wraps_long_values_within_eighty_columns() {
        let mut facts = facts();
        facts.name = "long-vm-name-".repeat(10);
        facts.modules = format!("{}lmx:git", "lmx:console, ".repeat(12));
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
        let text = render(&facts, Paint::plain());
        assert!(text.contains("'quote'"));
        for line in text.lines() {
            assert!(columns(line) <= 80, "{} columns: {line:?}", columns(line));
            assert_eq!(line.trim_end(), line, "trailing spaces: {line:?}");
        }
    }

    #[test]
    fn colors_only_when_asked() {
        assert!(!render(&facts(), Paint::plain()).contains('\x1b'));
        let text = render(&facts(), Paint::colored());
        assert!(text.contains("\x1b[1mdev-box\x1b[0m"));
        assert!(text.contains("\x1b[38;2;250;179;135mro\x1b[0m"));
    }
}
