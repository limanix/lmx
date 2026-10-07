//! `lmx doctor`: what is wrong with the guest owner, and what to do about it.
//!
//! It works without `lmxd`, because diagnosing `lmxd` is one of its jobs. Each check is a record with
//! a status and, when there is something to do, a hint; `--json` answers with the records.

use std::{io, process::ExitCode};

use lmx_facts::{FactError, generations};
use lmx_model::{
    CONFIG_PATH, CONVERGED, Check, CheckStatus, DEGRADED, DISK_LOW, Doctor, Envelope,
    FINALIZE_FAILED, Generations, OUT_OF_DATE, Owner, RESTART_REQUIRED,
};

use crate::{
    cli::OutputArgs,
    findings, output,
    owner::{self, CallError},
    palette::{Paint, Palette},
    system::System,
};

/// Hint of an `lmxd` that does not answer.
const OWNER_HINT: &str = "Check systemctl status lmx.socket lmx.service and journalctl -u lmx.";

/// Runs `lmx doctor`.
pub(crate) fn run(system: &System, args: &OutputArgs) -> io::Result<ExitCode> {
    let owner = owner::status(&system.owner_socket());
    let (markers, errors) = generations::read(&system.generation_paths());
    let mut checks = vec![config(system), owner_check(&owner)];
    checks.extend(conditions(owner.as_ref().ok(), &markers, errors.first()));
    if args.json {
        output::write_json(&Envelope::success(Doctor {
            checks: checks.clone(),
        }))?;
        return Ok(findings::status(&checks));
    }
    findings::write(&checks, Paint::detect(Palette::of_config(&system.config)))
}

/// Whether the configuration is readable and valid.
fn config(system: &System) -> Check {
    match &system.config {
        Ok(config) => Check::new(
            "config",
            CheckStatus::Ok,
            format!("{CONFIG_PATH} is valid; generation {}.", config.generation),
        ),
        Err(message) => Check::new("config", CheckStatus::Failed, message.clone())
            .hint("The platform renders it; limanix update on the Mac restores it."),
    }
}

/// Whether `lmxd` answers, and with the version of `lmx`.
fn owner_check(answer: &Result<Owner, CallError>) -> Check {
    let version = env!("CARGO_PKG_VERSION");
    match answer {
        Ok(owner) if owner.version == version => Check::new(
            "owner",
            CheckStatus::Ok,
            format!("lmxd {} answers.", owner.version),
        ),
        Ok(owner) => Check::new(
            "owner",
            CheckStatus::Warning,
            format!("lmxd {} answers, but lmx is {version}.", owner.version),
        )
        .hint("Restart the VM so both come from the booted generation."),
        Err(error) => Check::new("owner", CheckStatus::Failed, error.to_string()).hint(OWNER_HINT),
    }
}

/// The conditions of `lmxd`, or what the generation markers say without it; `error` is the first
/// marker that exists but could not be read.
fn conditions(
    owner: Option<&Owner>,
    markers: &Generations,
    error: Option<&FactError>,
) -> Vec<Check> {
    let Some(owner) = owner else {
        return vec![from_markers(markers, error)];
    };
    let holds = |kind: &str| {
        owner
            .conditions
            .iter()
            .find(|condition| condition.kind == kind)
            .map(|condition| condition.message.clone())
    };
    let applying = owner
        .operations
        .iter()
        .any(|operation| operation.kind == "SystemApply");
    let generation = if let Some(message) = holds(DEGRADED) {
        Check::new("generations", CheckStatus::Failed, message).hint(
            "See sudo lmx logs health, and the unit the reason names with systemctl status; lmxd \
             checks again every minute.",
        )
    } else if let Some(message) = holds(FINALIZE_FAILED) {
        Check::new("generations", CheckStatus::Warning, message).hint("See sudo lmx logs finalize.")
    } else if let Some(message) = holds(RESTART_REQUIRED) {
        Check::new("generations", CheckStatus::Warning, message)
            .hint("Restart the VM from the Mac; limanix update does it.")
    } else if let Some(message) = holds(OUT_OF_DATE) {
        let hint = if applying {
            "An update is in progress; wait for limanix update to finish."
        } else {
            "The host builds it: run limanix update on the Mac."
        };
        Check::new("generations", CheckStatus::Warning, message).hint(hint)
    } else if let Some(message) = holds(CONVERGED) {
        Check::new("generations", CheckStatus::Ok, message)
    } else {
        settled(markers)
    };
    let mut checks = vec![generation];
    if let Some(message) = holds(DISK_LOW) {
        checks.push(
            Check::new("disk", CheckStatus::Warning, message)
                .hint("Free space with sudo lmx store reserve."),
        );
    }
    checks
}

/// What the generation markers say when `lmxd` does not answer.
///
/// A marker the caller may not read, such as the mounted one for people, makes the check `unknown`;
/// any other read error fails it.
fn from_markers(markers: &Generations, error: Option<&FactError>) -> Check {
    match error {
        Some(FactError::Io { source, .. }) if source.kind() == io::ErrorKind::PermissionDenied => {
            return Check::new(
                "generations",
                CheckStatus::Unknown,
                "The mounted generation is readable only by root.",
            )
            .hint("Run sudo lmx doctor.");
        }
        Some(error) => {
            return Check::new("generations", CheckStatus::Failed, error.to_string());
        }
        None => {}
    }
    match (&markers.desired, &markers.built, &markers.booted) {
        (Some(desired), built, _) if built.as_ref() != Some(desired) => Check::new(
            "generations",
            CheckStatus::Warning,
            format!("Generation {desired} is mounted but not built; apply it."),
        )
        .hint("The host builds it: run limanix update on the Mac."),
        (Some(desired), _, booted) if booted.as_ref() != Some(desired) => Check::new(
            "generations",
            CheckStatus::Warning,
            format!("Generation {desired} is built; restart the VM to boot it."),
        )
        .hint("Restart the VM from the Mac; limanix update does it."),
        _ => settled(markers),
    }
}

/// A generation without a condition: booted and finalizing, or a system without markers.
fn settled(markers: &Generations) -> Check {
    match &markers.booted {
        Some(booted) => Check::new(
            "generations",
            CheckStatus::Ok,
            format!("Generation {booted} is booted."),
        ),
        None => Check::new(
            "generations",
            CheckStatus::Ok,
            "The system has no generation markers; it was built before lmx.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use lmx_model::{Condition, Operation};

    use super::*;

    /// An answer of `lmxd` with conditions of `kinds` and, when `applying`, a running apply.
    fn owner(kinds: &[&str], applying: bool) -> Owner {
        Owner {
            version: env!("CARGO_PKG_VERSION").into(),
            conditions: kinds
                .iter()
                .map(|kind| Condition {
                    kind: (*kind).into(),
                    message: format!("{kind} message"),
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

    /// Markers of three stages.
    fn markers(desired: &str, built: &str, booted: &str) -> Generations {
        let stage = |value: &str| (!value.is_empty()).then(|| value.to_owned());
        Generations {
            desired: stage(desired),
            built: stage(built),
            booted: stage(booted),
        }
    }

    /// Statuses and names of `checks`.
    fn summary(checks: &[Check]) -> Vec<(CheckStatus, &str)> {
        checks
            .iter()
            .map(|check| (check.status, check.check.as_str()))
            .collect()
    }

    #[test]
    fn turns_the_conditions_of_lmxd_into_findings() {
        let settled = markers("g1", "g1", "g1");
        let checks = conditions(Some(&owner(&[DEGRADED, DISK_LOW], false)), &settled, None);
        assert_eq!(
            summary(&checks),
            [
                (CheckStatus::Failed, "generations"),
                (CheckStatus::Warning, "disk")
            ]
        );
        let checks = conditions(Some(&owner(&[OUT_OF_DATE], true)), &settled, None);
        assert_eq!(
            checks[0].hint.as_deref(),
            Some("An update is in progress; wait for limanix update to finish.")
        );
        let checks = conditions(Some(&owner(&[], false)), &settled, None);
        assert_eq!(checks[0].message, "Generation g1 is booted.");
        let checks = conditions(Some(&owner(&[FINALIZE_FAILED], false)), &settled, None);
        assert_eq!(checks[0].status, CheckStatus::Warning);
        assert_eq!(
            checks[0].hint.as_deref(),
            Some("See sudo lmx logs finalize.")
        );
    }

    #[test]
    fn reads_the_markers_without_lmxd() {
        let check = |desired, built, booted| {
            conditions(None, &markers(desired, built, booted), None)[0].status
        };
        assert_eq!(check("g2", "g1", "g1"), CheckStatus::Warning);
        assert_eq!(check("g2", "g2", "g1"), CheckStatus::Warning);
        assert_eq!(check("g2", "g2", "g2"), CheckStatus::Ok);
        assert_eq!(check("", "", ""), CheckStatus::Ok);
        let error = |kind| FactError::Io {
            what: "the desired generation",
            source: io::Error::from(kind),
        };
        let denied = error(io::ErrorKind::PermissionDenied);
        let unreadable = conditions(None, &markers("", "g1", "g1"), Some(&denied));
        assert_eq!(unreadable[0].status, CheckStatus::Unknown);
        let broken = error(io::ErrorKind::NotConnected);
        let failed = conditions(None, &markers("", "g1", "g1"), Some(&broken));
        assert_eq!(failed[0].status, CheckStatus::Failed);
    }

    #[test]
    fn a_daemon_of_another_version_is_a_warning() {
        let mut other = owner(&[], false);
        other.version = format!("{}-other", env!("CARGO_PKG_VERSION"));
        assert_eq!(owner_check(&Ok(other)).status, CheckStatus::Warning);
        let unreachable = CallError::Unavailable("lmxd is not reachable".into());
        assert_eq!(owner_check(&Err(unreachable)).status, CheckStatus::Failed);
    }
}
