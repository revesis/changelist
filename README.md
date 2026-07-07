# gitcl

An IntelliJ IDEA-style **Changelist** TUI for git. Group your uncommitted
changes into named, persistent buckets, then commit, shelve, or push each
one independently — something plain git and git TUIs like lazygit/tig
don't offer.

## Usage

```sh
cargo build --release
./target/release/gitcl            # uses the git repo containing the cwd
./target/release/gitcl --repo /path/to/repo
```

## Keybindings

| Key | Action |
|---|---|
| `Tab` / `Shift+Tab` | Cycle pane focus (tree ↔ diff) |
| `j`/`k`, `↓`/`↑` | Move selection, or scroll the diff when it's focused |
| `h`/`l`, `←`/`→` | Scroll the focused pane horizontally (long paths/lines) |
| `Enter` | Fold/unfold the changelist under the cursor |
| `Space` | Stage/unstage the selected (or visual-range) files |
| `Shift+V` | Visual mode on a file row (batch select with `j`/`k`) |
| `Esc` | Exit visual mode |
| `v` | Toggle working-tree/staged diff |
| `n` | New changelist |
| `r` | Rename the selected changelist |
| `d` | Delete the selected changelist (files move to Default) |
| `a` | Set the selected changelist active (new changes default into it) |
| `m` | Move the selected (or visual-range) files to another changelist |
| `c` | Commit the selected changelist |
| `Shift+S` | Shelve the selected changelist |
| `Shift+U` | Unshelve — pick a shelf to apply (`d` inside deletes it) |
| `Shift+P` | Push the current branch |
| `Ctrl+R` / `F5` | Manual refresh |
| `?` | Toggle help |
| `q` / `Ctrl+C` | Quit |

## Shelving

`Shift+S` on a changelist saves its changes as a patch and reverts the
files to HEAD — like IntelliJ's Shelve Changes, or a `git stash` that only
touches the files you picked. `Shift+U` lists your shelves; applying one
restores the changes as plain working-tree edits and puts the files in a
changelist named after the shelf. Shelves live in `.git/gitcl-shelf/` as
ordinary `git apply`-able patch files (binary files included), so even
without gitcl you can inspect or apply them by hand. If a shelf no longer
applies cleanly because the code moved on, nothing is touched and the
shelf is kept.

## Pushing

`Shift+P` pushes the current branch to its configured upstream in the
background (a spinner shows progress; the UI stays responsive). The first
attempt never prompts — if the push actually needs input (HTTPS
credentials, an unknown SSH host key, a key passphrase), it fails fast
with a hint, and pressing `Shift+P` again reruns the push on the real
terminal where git and ssh can prompt you normally, then returns to the
TUI.

## How it works

- Changelist assignments are stored in `.git/changelist.json` — private to
  your local clone, never committed, survives restarts.
- All git access shells out to the `git` CLI (`status`, `diff`, `add`,
  `reset`, `commit`, `checkout`, `rm`, `apply`, `push`) — no `git2`/libgit2
  dependency.
- Committing a changelist uses `git commit --only -- <paths>`, which commits
  exactly those paths' working-tree content on top of HEAD without touching
  the index for any other path — so staged/partially-staged changes in other
  changelists are left untouched.
- Shelving follows the same isolation rule: only the shelved changelist's
  paths are ever passed to a git command, and the patch is written to disk
  before anything in the working tree is reverted, so a failure can't lose
  changes.

## Known limitations (by design, see plan for rationale)

- No hunk-level (partial-file) changelist assignment — a changelist commits
  or shelves the whole current working-tree content of its files.
- A partially-staged file commits/shelves its working-tree content; the
  staged/unstaged split within a file is not preserved.
- No multi-VCS support or remote sync.
