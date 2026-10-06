//! `lmx version`: the binary and the host contract it speaks.

use std::{io, process::ExitCode};

use lmx_model::{CONTRACT_VERSION, Envelope, Version};

use crate::{cli::OutputArgs, output};

/// Runs `lmx version`.
pub(crate) fn run(args: &OutputArgs) -> io::Result<ExitCode> {
    let version = Version {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        contract: CONTRACT_VERSION,
    };
    if args.json {
        output::write_json(&Envelope::success(version))?;
    } else {
        output::write_text(&format!(
            "lmx {} (host contract {})\n",
            version.version, version.contract
        ))?;
    }
    Ok(ExitCode::SUCCESS)
}
