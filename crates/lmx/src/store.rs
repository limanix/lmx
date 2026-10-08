//! `lmx store reserve`: room in the Nix store before the host stops the VM.
//!
//! The collection runs in `lmxd`, which keeps going if this command is interrupted. The answer
//! carries the store disk usage before and after, for the host to warn about a disk that stays low.

use std::{io, process::ExitCode};

use lmx_model::{Envelope, ErrorBody, Reserve, Shortage};
use serde_json::Value;

use crate::{cli::OutputArgs, format, output, owner, system::System};

/// Runs `lmx store reserve`.
pub(crate) fn reserve(system: &System, args: &OutputArgs) -> io::Result<ExitCode> {
    match owner::reserve(&system.owner_socket()) {
        Ok(Ok(reserve)) => {
            if args.json {
                output::write_json(&Envelope::success(&reserve))?;
            } else {
                output::write_text(&render(&reserve))?;
            }
            Ok(ExitCode::SUCCESS)
        }
        Ok(Err(error)) => fail(args, error),
        Err(error) => owner::report(error, args.json),
    }
}

/// Reports a failed reserve; people also see the disk a shortage left.
fn fail(args: &OutputArgs, error: ErrorBody) -> io::Result<ExitCode> {
    let shortage = serde_json::from_value::<Shortage>(Value::Object(error.details.clone()));
    let status = output::failure(args.json, error, output::FAILURE)?;
    if let (false, Ok(shortage)) = (args.json, shortage) {
        eprintln!("Guest disk: {}.", format::disk(&shortage.after));
    }
    Ok(status)
}

/// Renders a reserve for people.
fn render(reserve: &Reserve) -> String {
    let done = if reserve.collected {
        format!(
            "Collected unreferenced store paths and freed {}.",
            format::gibibytes(reserve.freed_bytes)
        )
    } else {
        "Nothing was collected.".to_owned()
    };
    format!("{done}\nGuest disk: {}.\n", format::disk(&reserve.after))
}

#[cfg(test)]
mod tests {
    use lmx_model::DiskUsage;

    use super::*;

    fn usage(free: u64) -> DiskUsage {
        DiskUsage {
            bytes: 16 << 30,
            free_bytes: free << 30,
            available_bytes: free << 30,
            inodes: 0,
            free_inodes: 0,
        }
    }

    #[test]
    fn says_how_much_a_collection_freed() {
        let reserve = Reserve {
            before: usage(2),
            after: usage(6),
            freed_bytes: 4 << 30,
            collected: true,
        };
        assert_eq!(
            render(&reserve),
            "Collected unreferenced store paths and freed 4.0 GiB.\n\
             Guest disk: 6.0 GiB of 16 GiB free.\n"
        );
    }

    #[test]
    fn says_when_nothing_was_collected() {
        let reserve = Reserve {
            before: usage(8),
            after: usage(8),
            freed_bytes: 0,
            collected: false,
        };
        assert_eq!(
            render(&reserve),
            "Nothing was collected.\nGuest disk: 8.0 GiB of 16 GiB free.\n"
        );
    }
}
