# gitcl

An IntelliJ IDEA-style **Changelist** TUI for git. Group your uncommitted
changes into named, persistent buckets and commit each one independently —
something plain git and git TUIs like lazygit/tig don't offer.

## Usage

```sh
cargo build --release
./target/release/gitcl            # uses the git repo containing the cwd
./target/release/gitcl --repo /path/to/repo
```

## Keybindings

| Key | Action |
|---|---|
| `Tab` / `Shift+Tab` | Cycle pane focus (changelists → files) |
| `j`/`k`, `↓`/`↑` | Move selection |
| `Enter` | Drill into the files pane |
| `Space` | Stage/unstage the selected file |
| `v` | Toggle working-tree/staged diff |
| `n` | New changelist |
| `r` | Rename the selected changelist |
| `d` | Delete the selected changelist (files move to Default) |
| `a` | Set the selected changelist active (new changes default into it) |
| `m` | Move the selected file to another changelist |
| `c` | Commit the selected changelist |
| `Ctrl+R` / `F5` | Manual refresh |
| `?` | Toggle help |
| `q` / `Ctrl+C` | Quit |

## How it works

- Changelist assignments are stored in `.git/changelist.json` — private to
  your local clone, never committed, survives restarts.
- All git access shells out to the `git` CLI (`status`, `diff`, `add`,
  `reset`, `commit`) — no `git2`/libgit2 dependency.
- Committing a changelist uses `git commit --only -- <paths>`, which commits
  exactly those paths' working-tree content on top of HEAD without touching
  the index for any other path — so staged/partially-staged changes in other
  changelists are left untouched.

## Known limitations (by design, see plan for rationale)

- No hunk-level (partial-file) changelist assignment — a changelist commits
  the whole current working-tree content of its files.
- No multi-VCS support, remote sync, or shelving/stashing.
