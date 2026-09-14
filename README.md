# trashy
Plans, prompts, and executes a cleanse of your trashy filesystem(s)


```
trashy                   open the TUI
trashy ~/code ~/media    scan directories instead
trashy -t /mnt/primary   target another mount point
trashy -u /              stop targeting one
trashy -m delete         delete permanently instead of trashing
trashy -c path.yaml      use a config
```

## Install

With cargo (any os):

cargo install --git https://github.com/nickheyer/trashy trashy

On Arch:

yay -S trashy-bin

## Config

Priority load order for config file: `-c PATH` -> `$TRASHY_CONFIG` -> `./trashy.yaml` -> `~/.config/trashy/config.yaml` (default, gets auto created)


```yaml
targets: { /: true, /mnt/primary: true, /mnt/backups: false }   # mount points/dirs
mode: trash                                                     # trash || delete
top_files: 500                                                  # size of the Files tab
exclude: ["/mnt/primary/vm/**"]                                 # path globs never scanned
protect: [/usr, /etc, /opt, /var, /boot, /bin, /sbin, /lib, /lib64, /srv, /nix, /snap]  
dupes: { enabled: true, min_size: 1M }
recipes:
  - { id: yay-cache, name: yay cache, kind: path, glob: "~/.cache/yay", risk: low, cmd: yay -Sc --noconfirm }
  - { id: big-logs, enabled: false }
```

## Recipes

| field | meaning |
|---|---|
| `kind` | `dir` matches directory names, `file` matches file names, `path` points at literal paths |
| `glob` | name glob for dir/file (`{a,b}` alternation); for path: absolute paths, `~`, `{mount}`, `*` per component, `\|` for several |
| `markers` | files that must exist for a dir match: `CACHEDIR.TAG` inside it, `../Cargo.toml` beside it, `a\|b` for alternatives, globs allowed |
| `not_under` | directory names that must not appear anywhere above a dir/file match
| `risk` | `safe` items are pre-selected, `low` and `medium` wait for you |
| `children` | path kind: offer every entry of the directory separately (`~/.cache`, `~/Downloads`) |
| `min_size` `min_age` | only report entries at least this big (`100M`) or this old (days) |
| `cmd` | run a shell command instead of deleting (`sudo pacman -Sc --noconfirm`) |
| `permanent` | never trash, always delete (trash bins) |

Builtins cover Cargo, npm/pnpm, Python venvs and caches, JS framework caches and build output, CMake, Gradle, Maven,
.NET, Zig, Elixir, Haskell, Terraform, `CACHEDIR.TAG`-tagged dirs, OS/editor junk, partial downloads, core dumps,
rotated and huge logs, trash bins, `~/.cache`, package manager and language caches, Electron/Chromium caches,
stale `~/Downloads` and `/var/tmp`, systemd journal and coredumps, pacman/apt/dnf caches, Docker, Podman and Flatpak.
Matches nested inside another match are folded into the outer one, and nothing under a different mount is ever counted.
