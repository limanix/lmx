//! `lmx logs KIND`: the output of the latest run of an `lmxd` task kind, from the journal.
//!
//! The journal is the history: `lmxd` keeps its runs in memory only, and an apply ends with a
//! restart. `--previous` reads the boot before the running one, such as the one that built the
//! running generation.

use std::{io, process::ExitCode};

use lmx_facts::journal::{self, Boot, Record};

use crate::{cli::LogKind, output, system::System};

/// Runs `lmx logs KIND`.
pub(crate) fn run(system: &System, kind: LogKind, previous: bool) -> io::Result<ExitCode> {
    let boot = if previous {
        Boot::Previous
    } else {
        Boot::Current
    };
    let name = kind.name();
    let (records, readable) = match journal::records(&system.journalctl(), kind.task_kind(), boot) {
        Ok(answer) => answer,
        Err(error) => {
            eprintln!("lmx: the journal cannot be read: {error}");
            return Ok(ExitCode::from(output::FAILURE));
        }
    };
    match latest(&records) {
        Some(run) => {
            output::write_text(&render(&run))?;
            Ok(ExitCode::SUCCESS)
        }
        None if !readable => {
            eprintln!(
                "lmx: the journal of lmxd is readable by root and the groups wheel and \
                 systemd-journal; run sudo lmx logs {name}"
            );
            Ok(ExitCode::from(output::FAILURE))
        }
        None if previous => {
            eprintln!("lmx: no {name} ran in the previous boot");
            Ok(ExitCode::from(output::FAILURE))
        }
        None => {
            eprintln!("lmx: no {name} ran in this boot; try lmx logs {name} --previous");
            Ok(ExitCode::from(output::FAILURE))
        }
    }
}

/// The records of the latest task in `records`.
///
/// A run is a task name of one process: a restarted `lmxd` numbers its tasks from 1 again.
fn latest(records: &[Record]) -> Option<Vec<&Record>> {
    let last = records
        .iter()
        .rev()
        .find(|record| !record.task.is_empty())?;
    Some(
        records
            .iter()
            .filter(|record| record.task == last.task && record.pid == last.pid)
            .collect(),
    )
}

/// The run's task, generation and start, then its lines.
fn render(run: &[&Record]) -> String {
    let Some(first) = run.first() else {
        return String::new();
    };
    let generation = run
        .iter()
        .find_map(|record| record.generation.as_deref())
        .map_or_else(String::new, |generation| {
            format!(", generation {generation}")
        });
    let mut text = format!("{}{generation}, {}\n", first.task, utc(first.time));
    for record in run {
        text.push_str(&record.message);
        text.push('\n');
    }
    text
}

/// `micros` since the Unix epoch as a UTC date and time, such as `2026-10-07 09:12:03 UTC`.
fn utc(micros: u64) -> String {
    let seconds = micros / 1_000_000;
    let (days, of_day) = (seconds / 86_400, seconds % 86_400);
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let of_era = shifted - era * 146_097;
    let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
    let day_of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        of_day / 3600,
        of_day % 3600 / 60,
        of_day % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(pid: u32, task: &str, message: &str, generation: Option<&str>) -> Record {
        Record {
            task: task.into(),
            pid,
            generation: generation.map(Into::into),
            message: message.into(),
            time: 1_791_364_323_000_000,
        }
    }

    #[test]
    fn shows_the_latest_run_of_the_latest_process() {
        let records = [
            record(7, "system-apply-1", "applying generation g1", Some("g1")),
            record(7, "system-apply-2", "old build", None),
            record(9, "system-apply-1", "applying generation g2", Some("g2")),
            record(
                9,
                "system-apply-1",
                "building the system configuration...",
                None,
            ),
            record(9, "", "an event outside a task", None),
        ];
        let run = latest(&records).expect("a run");
        assert_eq!(
            render(&run),
            "system-apply-1, generation g2, 2026-10-07 09:12:03 UTC\n\
             applying generation g2\n\
             building the system configuration...\n"
        );
        assert!(latest(&[]).is_none());
    }

    #[test]
    fn writes_utc_dates() {
        assert_eq!(utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(utc(951_782_400_000_000), "2000-02-29 00:00:00 UTC");
        assert_eq!(utc(1_791_364_323_000_000), "2026-10-07 09:12:03 UTC");
    }
}
