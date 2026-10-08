# Everyday commands

These commands work in any guest shell. None of them needs `sudo`, although
`sudo` lets `lmx status` read a few more facts.

## Where am I?

`lmx help` shows the VM, your account, the selected modules, and the commands
inside the VM and on the Mac. `lmx welcome` shows the summary that greets you
when a shell starts: the VM, its resources, modules and shared folders, plus
warnings when services failed or the disk runs low. `lmx info` adds the kernel,
the guest disk, the shared folders and failed services.

## How is the guest doing?

```console
$ lmx status
Generation    desired 0123456789ab, built 0123456789ab, booted 0123456789ab
Disk          8.0 GiB of 16 GiB free, 495616 of 1048576 inodes free
Network       enp0s1 192.0.2.10
Failed units  none
Owner         lmxd 0.1.0, idle
```

| Line | Tells you |
| -- | -- |
| Generation | Which configuration the Mac handed over (`desired`), which one is built for the next boot (`built`), and which one runs now (`booted`). When all three match, the VM runs what you asked for. |
| Disk | Free space and free inodes on the disk that holds the Nix store. Either one can run out first. |
| Network | The guest's addresses, the ones the Mac connects to. |
| Failed units | systemd services that failed. |
| Owner | The `lmxd` version and what it is doing right now, such as `StoreCollect running`. |

When something needs your attention, `lmx status` adds a `Condition` line that
says what and why. A fact that cannot be read becomes `unknown`, and a `Problem`
line explains it; the other lines still show.

> [!TIP] Without `sudo`, the generation the Mac mounted may read as `unknown`:
> that mount is open to root only. `sudo lmx status` shows every fact.

### The status word in tmux and the prompt

When the Cozy workbench or the tmux and Starship modules are selected, a short
word appears in the tmux status line, or in the prompt outside tmux. It comes
from `lmx status --short`, and it stays empty while all is well.

| Word | What it means | What to do |
| -- | -- | -- |
| `degraded` | The running generation failed its health check | Run `lmx doctor`, then `sudo lmx logs health` |
| `finalize` | The update works, but removing older generations failed | Nothing urgent: `lmxd` tries again later |
| `disk-low` | Less than 10% of the store disk is free | Run `sudo lmx store reserve`, see [Disk and the Nix store](disk.md) |
| `restart` | A new generation is built and waits for a restart | Run `limanix update` on the Mac; it restarts the VM |
| `applying` | An update is being built right now | Wait for `limanix update` to finish |
| `apply` | The Mac handed over a generation that is not built yet | Run `limanix update` on the Mac |
| `lmxd?` | `lmxd` did not answer within 200 ms | Run `lmx doctor` |

## Copy and paste with the Mac

```console
$ pbcopy < notes.txt
$ git diff | pbcopy
$ pbpaste > snippet.txt
```

`pbcopy` and `pbpaste` behave like their macOS namesakes. They travel through
your terminal with OSC 52, or through tmux when you work inside tmux. Your
terminal on the Mac must allow it; reading the clipboard usually needs one more
permission than writing. `pbpaste` waits up to 10 seconds for the terminal to
answer. They are other names of `lmx clipboard copy` and `lmx clipboard paste`.

> [!NOTE] Clipboard access is a terminal setting on the Mac.
> [Terminal and clipboard](https://limanix.dev/terminal.html) lists the settings
> for common terminals.

## Named sessions

```console
$ limanix-session api
```

A named session survives a closed terminal: open it again and you are back where
you left off. `limanix-session NAME` opens or attaches the session `NAME` with
the provider your modules select, such as tmux. From the Mac,
`limanix shell NAME --session PROJECT` lands you in the same session.
`lmx session NAME` does the same as `limanix-session NAME`.

Without a provider, `limanix-session` tells you which module to select. A plain
shell needs no provider.

## Colors

`lmx` colors its text with the
[theme](https://limanix.dev/categories/client/configuration.html#choose-a-theme)
you choose in `[theme]` of `limanix.toml`, Catppuccin Mocha by default. Output
that does not go to a terminal, `NO_COLOR` and `TERM=dumb` all turn colors off.

## Which version runs here?

`lmx version` prints the binary version and the version of the host contract it
speaks.
