//! `lmx info`: the workspace, kernel, guest disk, shared folders and failed units in detail.

use std::{io, process::ExitCode};

use lmx_facts::{
    FactError, disk, machine,
    mounts::{self, Mount},
    units,
};

use crate::{
    format, help,
    layout::{columns, rows},
    output,
    system::System,
};

/// Width of the label column.
const LABEL: usize = 14;

/// Runs `lmx info`; fails after printing when any part cannot be read.
pub(crate) fn run(system: &System) -> io::Result<ExitCode> {
    let (page, complete) = page(system);
    output::write_text(&page)?;
    Ok(output::page_status(complete))
}

/// The info page, and whether every part of it could be read.
fn page(system: &System) -> (String, bool) {
    let disk = disk::usage(&system.store()).map(|usage| format::disk(&usage));
    let shared = mounts::shared(&system.mountinfo()).map(|mounts| shared_table(&mounts));
    let failed = units::failed(&system.systemctl()).map(|units| {
        if units.is_empty() {
            "none".to_owned()
        } else {
            units.join("\n")
        }
    });
    let complete = system.config.is_ok() && disk.is_ok() && shared.is_ok() && failed.is_ok();

    let mut text = help::metadata(&system.config);
    text.push('\n');
    text.push_str(&rows(
        &[
            ("Kernel", machine::kernel()),
            ("Disk", or_unavailable(disk)),
            ("Shared", or_unavailable(shared)),
            ("Failed units", or_unavailable(failed)),
        ],
        LABEL,
    ));
    (text, complete)
}

/// A fact, or why it is unavailable.
fn or_unavailable(fact: Result<String, FactError>) -> String {
    fact.unwrap_or_else(|error| format!("unavailable: {error}"))
}

/// One line per shared folder: target, type and mode, in aligned columns.
fn shared_table(mounts: &[Mount]) -> String {
    if mounts.is_empty() {
        return "(none)".to_owned();
    }
    let width = |column: fn(&Mount) -> &str| {
        mounts
            .iter()
            .map(|mount| columns(column(mount)))
            .max()
            .unwrap_or_default()
    };
    let (target_width, type_width) = (width(|mount| &mount.target), width(|mount| &mount.fs_type));
    mounts
        .iter()
        .map(|mount| {
            let target_padding = target_width - columns(&mount.target);
            let type_padding = type_width - columns(&mount.fs_type);
            let mode = if mount.read_only { "ro" } else { "rw" };
            format!(
                "{}{:target_padding$}  {}{:type_padding$}  {mode}",
                mount.target, "", mount.fs_type, ""
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligns_shared_folders() {
        let mounts = [
            Mount {
                target: "/home/dev".into(),
                fs_type: "virtiofs".into(),
                read_only: false,
            },
            Mount {
                target: "/mnt/limanix".into(),
                fs_type: "9p".into(),
                read_only: true,
            },
        ];
        assert_eq!(
            shared_table(&mounts),
            "/home/dev     virtiofs  rw\n/mnt/limanix  9p        ro"
        );
        assert_eq!(shared_table(&mounts[1..]), "/mnt/limanix  9p  ro");
        assert_eq!(shared_table(&[]), "(none)");
    }
}
