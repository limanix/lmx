//! `lmx help`: the guest's help page.
//!
//! `lmx`, `lmx help`, `lmx --help` and `lmx -h` print this page, as the platform's shell `lmx` did.
//! Help for one command, such as `lmx status --help`, comes from the command line parser.

use std::{io, process::ExitCode};

use lmx_model::Config;

use crate::{output, system::System};

/// Commands people use inside the VM and on the Mac.
const REFERENCE: &str = "\
Inside this VM
  lmx info              Show the kernel, guest disk, shared folders and failed units.
  lmx status            Show generations, disk, network, failed units; sudo shows every fact.
  lmx welcome           Show the workspace welcome again.
  limanix-session NAME  Open a named session with the selected session provider.
  pbcopy < FILE         Copy to the Mac clipboard through the terminal.
  pbpaste               Print the Mac clipboard, if the terminal allows reads.
  exit                  Return to the Mac.

On the Mac
  limanix list                          Show VM state and network addresses.
  limanix shell NAME                    Open the guest user's login shell.
  limanix shell NAME --session PROJECT  Open a named project session.
  limanix update --config FILE          Apply resources, modules and environment.
  limanix stop NAME                     Stop the VM; keep its disk and home.

Tools come from the modules selected in your TOML configuration.
No session provider is required for a normal shell.
Guide: https://limanix.dev/categories/client/getting-started.html
";

/// Runs `lmx help`; fails after printing the page when the workspace metadata is unreadable.
pub(crate) fn run(system: &System) -> io::Result<ExitCode> {
    let (page, readable) = page(&system.config);
    output::write_text(&page)?;
    Ok(output::page_status(readable))
}

/// The help page, and whether the workspace metadata in it was readable.
fn page(config: &Result<Config, String>) -> (String, bool) {
    let mut page = String::from("LimaNix workspace\n\n");
    page.push_str(&metadata(config));
    page.push('\n');
    page.push_str(REFERENCE);
    (page, config.is_ok())
}

/// The workspace summary lines, or why they cannot be shown.
pub(crate) fn metadata(config: &Result<Config, String>) -> String {
    match config {
        Ok(config) => summary(config),
        Err(message) => format!("Workspace metadata cannot be read: {message}\n"),
    }
}

/// The VM, its user and the selected modules, one per line.
fn summary(config: &Config) -> String {
    format!(
        "VM: {} ({})\nUser: {}\nHome: {}\nModules: {}\n",
        config.vm.name,
        config.vm.arch,
        config.user.name,
        config.user.home,
        modules(config)
    )
}

/// The selected catalog modules, or `none`.
pub(crate) fn modules(config: &Config) -> String {
    if config.modules.is_empty() {
        "none".to_owned()
    } else {
        config.modules.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use lmx_model::{Config, DiskPolicy, Session, Tools, User, Vm};

    use super::*;

    /// Configuration of a VM named `dev-box` with no modules.
    fn config() -> Config {
        Config {
            schema: 1,
            vm: Vm {
                name: "dev-box".into(),
                arch: "arm64".into(),
                system: "NixOS 26.05".into(),
            },
            generation: "0123456789ab".into(),
            user: User {
                name: "dev".into(),
                home: "/home/dev".into(),
                uid: 501,
            },
            modules: vec![],
            disk: DiskPolicy {
                collect_percent: 20,
                minimum_percent: 10,
            },
            session: Session {
                command: None,
                providers: vec!["lmx:tmux".into()],
            },
            tools: Tools {
                ip: "ip".into(),
                systemctl: "systemctl".into(),
                nix_store: "nix-store".into(),
                nice: "nice".into(),
                ionice: "ionice".into(),
                grep: "grep".into(),
            },
        }
    }

    #[test]
    fn shows_the_workspace_and_every_command() {
        let (page, readable) = page(&Ok(config()));
        assert!(readable);
        assert!(page.starts_with(
            "LimaNix workspace\n\nVM: dev-box (arm64)\nUser: dev\nHome: /home/dev\nModules: none\n\n"
        ));
        for command in [
            "lmx info",
            "lmx status",
            "lmx welcome",
            "limanix-session NAME",
            "pbcopy",
            "pbpaste",
            "limanix shell NAME",
            "limanix update --config FILE",
        ] {
            assert!(page.contains(command), "missing {command}");
        }
    }

    #[test]
    fn explains_unreadable_metadata_and_still_lists_commands() {
        let (page, readable) = page(&Err("cannot read /etc/lmx/config.json".into()));
        assert!(!readable);
        assert!(
            page.contains("Workspace metadata cannot be read: cannot read /etc/lmx/config.json")
        );
        assert!(page.contains("lmx info"));
    }

    #[test]
    fn lists_selected_modules() {
        let mut config = config();
        config.modules = vec!["lmx:console".into(), "lmx:go".into()];
        assert_eq!(modules(&config), "lmx:console, lmx:go");
    }
}
