//! `lmx help`: the guest's help page, and the help cards of modules.
//!
//! `lmx`, `lmx help`, `lmx --help` and `lmx -h` print the page, as the platform's shell `lmx` did.
//! `lmx help TOPIC` prints the help card a module declares, such as `lmx help python`.
//! Help for one command, such as `lmx status --help`, comes from the command line parser.

use std::{io, process::ExitCode};

use lmx_model::{Card, Config, Help};

use crate::{
    layout::wrap,
    output,
    palette::{Paint, Palette},
    system::System,
    welcome::{label, row},
};

/// Commands people use inside the VM, after `lmx help TOPIC`.
const INSIDE: [(&str, &str); 9] = [
    (
        "lmx info",
        "Show the kernel, guest disk, shared folders and failed units.",
    ),
    (
        "lmx status",
        "Show generations, disk, network, failed units; sudo shows every fact.",
    ),
    (
        "lmx doctor",
        "Find what is wrong with the guest owner and what to do about it.",
    ),
    (
        "lmx net check PORT",
        "Check why a port may be unreachable from the Mac.",
    ),
    ("lmx welcome", "Show the workspace welcome again."),
    (
        "limanix-session NAME",
        "Open a named session with the selected session provider.",
    ),
    (
        "pbcopy < FILE",
        "Copy to the Mac clipboard through the terminal.",
    ),
    (
        "pbpaste",
        "Print the Mac clipboard, if the terminal allows reads.",
    ),
    ("exit", "Return to the Mac."),
];

/// Commands people use on the Mac, and where to read more.
const MAC: &str = "\
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

/// Columns of a command in the list inside the VM, after the two-column indent.
const COMMAND: usize = 22;
/// Columns left for the description of a command: 80 minus the indent and the command.
const DESCRIPTION: usize = 56;
/// Columns of the title and the summary of a card, after the two-column indent.
const CARD: usize = 78;

/// Runs `lmx help`, or `lmx help TOPIC` with a topic.
pub(crate) fn run(system: &System, topic: Option<&str>) -> io::Result<ExitCode> {
    let help = system.help();
    match topic {
        None => {
            let (page, readable) = page(&system.config, &help);
            output::write_text(&page)?;
            Ok(output::page_status(readable))
        }
        Some(name) => topic_card(system, &help, name),
    }
}

/// The help page, and whether the workspace metadata and the help cards in it were readable.
fn page(config: &Result<Config, String>, help: &Result<Help, String>) -> (String, bool) {
    let mut page = String::from("LimaNix workspace\n\n");
    page.push_str(&metadata(config));
    page.push_str("\nInside this VM\n");
    page.push_str(&topics(help));
    for (command, description) in INSIDE {
        page.push_str(&format!("  {command:<COMMAND$}{description}\n"));
    }
    page.push('\n');
    page.push_str(MAC);
    (page, config.is_ok() && help.is_ok())
}

/// The `lmx help TOPIC` line of the page, with the topics that have a card.
fn topics(help: &Result<Help, String>) -> String {
    let description = match help {
        Ok(help) if help.topics.is_empty() => {
            "Show what a module gives you; none has a card.".to_owned()
        }
        Ok(help) => format!(
            "Show what a module gives you: {}.",
            help.topics
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Err(message) => format!("Module help cannot be read: {message}"),
    };
    let mut command = "lmx help TOPIC";
    let mut text = String::new();
    for line in wrap(&description, DESCRIPTION) {
        text.push_str(&format!("  {command:<COMMAND$}{line}\n"));
        command = "";
    }
    text
}

/// Prints the card of the topic `name`, or explains why there is none.
fn topic_card(system: &System, help: &Result<Help, String>, name: &str) -> io::Result<ExitCode> {
    let help = match help {
        Ok(help) => help,
        Err(message) => {
            eprintln!("lmx: module help cannot be read: {message}");
            return Ok(ExitCode::from(output::FAILURE));
        }
    };
    let Some((_, card)) = help.find(name) else {
        if help.topics.is_empty() {
            eprintln!("lmx: no help for {name}; no selected module has a help card");
        } else {
            let topics: Vec<&str> = help.topics.keys().map(String::as_str).collect();
            eprintln!("lmx: no help for {name}; topics: {}", topics.join(", "));
        }
        return Ok(ExitCode::from(output::FAILURE));
    };
    output::write_text(&render(
        card,
        Paint::detect(Palette::of_config(&system.config)),
    ))?;
    Ok(ExitCode::SUCCESS)
}

/// A card: title and summary, then commands and tips in a labeled column, then the guide.
///
/// The guide's address is never wrapped, which keeps it whole for copying.
fn render(card: &Card, paint: Paint) -> String {
    let palette = paint.palette();
    let mut text = String::new();
    for line in wrap(&card.title, CARD) {
        text.push_str(&format!("  {}\n", paint.bold(&line)));
    }
    for line in wrap(&card.summary, CARD) {
        text.push_str(&format!("  {}\n", paint.color(palette.subtext, &line)));
    }
    if !card.commands.is_empty() || !card.tips.is_empty() {
        text.push('\n');
    }
    if !card.commands.is_empty() {
        row(
            &mut text,
            paint,
            "Commands",
            &card.commands.join(", "),
            Some(palette.blue),
        );
    }
    for tip in &card.tips {
        row(&mut text, paint, &tip.label, &tip.text, None);
    }
    if let Some(guide) = &card.guide {
        text.push_str(&format!(
            "\n  {}{}\n",
            label(paint, "Guide"),
            paint.color(palette.blue, guide)
        ));
    }
    text
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
    use lmx_model::{Config, DiskPolicy, Health, Network, Session, Theme, Tools, User, Vm};

    use lmx_model::Tip;

    use super::*;

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
                gid: 100,
            },
            modules: vec![],
            disk: DiskPolicy {
                collect_percent: 20,
                minimum_percent: 10,
            },
            health: Health { units: vec![] },
            network: Network {
                ports: Default::default(),
            },
            theme: Theme {
                flavor: "mocha".into(),
                palette: Default::default(),
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
                nixos_rebuild: "nixos-rebuild".into(),
                nix_env: "nix-env".into(),
                sudo: "sudo".into(),
                bash: "bash".into(),
                systemd_run: "systemd-run".into(),
                journalctl: "journalctl".into(),
            },
        }
    }

    fn help() -> Help {
        let card = Card {
            title: "Python 3.12.14".into(),
            summary: "Python 3 with venv, virtualenv, and the Pyright language server.".into(),
            commands: vec!["python".into(), "virtualenv".into(), "pyright".into()],
            tips: vec![Tip {
                label: "New venv".into(),
                text: "python -m venv .venv".into(),
            }],
            guide: Some("https://limanix.dev/categories/nixos/modules/python/README.html".into()),
        };
        Help {
            schema: 1,
            topics: [("python".to_owned(), card)].into(),
        }
    }

    #[test]
    fn shows_the_workspace_and_every_command() {
        let (page, readable) = page(&Ok(config()), &Ok(help()));
        assert!(readable);
        assert!(page.starts_with(
            "LimaNix workspace\n\nVM: dev-box (arm64)\nUser: dev\nHome: /home/dev\nModules: none\n\n"
        ));
        for command in [
            "lmx info",
            "lmx status",
            "lmx doctor",
            "lmx net check PORT",
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
        let (page, readable) = page(&Err("cannot read /etc/lmx/config.json".into()), &Ok(help()));
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

    #[test]
    fn lists_the_topics_with_a_card_first_inside_the_vm() {
        let (page, _) = page(&Ok(config()), &Ok(help()));
        assert!(
            page.contains("Inside this VM\n  lmx help TOPIC        Show what a module gives you: python.\n  lmx info "),
            "{page}"
        );
    }

    #[test]
    fn explains_unreadable_help_cards_and_fails() {
        let (page, readable) = page(&Ok(config()), &Err("cannot read /etc/lmx/help.json".into()));
        assert!(!readable);
        assert!(
            page.contains("lmx help TOPIC        Module help cannot be read: cannot read"),
            "{page}"
        );
        assert!(page.contains("lmx info"));
    }

    #[test]
    fn renders_a_card_with_commands_tips_and_the_guide() {
        let card = &help().topics["python"];
        assert_eq!(
            render(card, Paint::plain()),
            "  Python 3.12.14\n  \
             Python 3 with venv, virtualenv, and the Pyright language server.\n\n  \
             Commands   python, virtualenv, pyright\n  \
             New venv   python -m venv .venv\n\n  \
             Guide      https://limanix.dev/categories/nixos/modules/python/README.html\n"
        );
    }

    #[test]
    fn a_card_without_commands_tips_or_guide_is_its_title_and_summary() {
        let card = Card {
            title: "Cozy".into(),
            summary: "A complete workbench.".into(),
            commands: vec![],
            tips: vec![],
            guide: None,
        };
        assert_eq!(
            render(&card, Paint::plain()),
            "  Cozy\n  A complete workbench.\n"
        );
    }
}
