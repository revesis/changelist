# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`gitcl` — an IntelliJ IDEA-style **Changelist** TUI for git, written in Rust with ratatui. It groups uncommitted changes into named, persistent buckets and commits each independently, which neither plain git nor TUIs like lazygit/tig offer (they only model the index's staged/unstaged binary, with no durable extra grouping layer).

## Commands

```sh
cargo build              # debug build -> target/debug/gitcl
cargo build --release    # release build -> target/release/gitcl
cargo test                # run all tests (unit tests live inline in src/ via #[cfg(test)])
cargo test <substring>    # run a single test, e.g. `cargo test commits_rename`
cargo run -- --repo /path/to/repo   # run against a specific repo instead of cwd
```

Tests that matter most live in `src/app/commit.rs` (`commit_isolation`-style integration tests against real temporary git repos via `tempfile`) and `src/model/store.rs` (reconciliation logic). `src/git/status.rs` has porcelain-v2 parser tests. There is no separate `tests/` directory — everything is inline `#[cfg(test)] mod tests`.

When changing anything in `app/commit.rs` or `model/store.rs::reconcile`, run the full suite — these are the two places where a subtle bug silently corrupts the user's git state or loses a changelist assignment.

## Architecture

Four-layer module split, each with a one-directional dependency on the one below it. This separation is what makes the riskiest logic (the isolated commit algorithm) testable without a TTY:

- **`git/`** — the *only* layer allowed to shell out to the `git` CLI (via `std::process::Command`, chokepointed through `git/command.rs::run_git`). No `git2`/libgit2 anywhere. `git/status.rs` parses `git status --porcelain=v2 -z` into `StatusEntry`. There is no caching here — callers decide when to invoke it.
- **`model/`** — pure data + reconciliation, no I/O. `ChangelistStore` holds `changelists: Vec<Changelist>` and `files: HashMap<path, ChangelistId>`, persisted to `.git/changelist.json` (private, not committed, atomic write-then-rename). `store.rs::reconcile()` is the core invariant-preserving function: it runs every refresh against live `git status` output and must handle three path-identity-change cases without losing a file's changelist assignment — exact renames (via `orig_path`), and the less obvious **directory collapse/expand**: git reports a whole untracked directory as one `dir/` entry when nothing tracked is inside it, then expands back into individual file paths once something appears — `inherited_changelist_for()` detects this by directory-prefix matching against paths that are about to vanish, so `qwe/321` moved to a custom changelist doesn't silently fall back to the active changelist when staging/unstaging causes git to collapse/expand the entry.
- **`app/`** — controller layer, no ratatui dependency. `App` owns `ChangelistStore` + live `StatusEntry` cache + UI selection state, and is testable by calling `Action` dispatch directly. `app/actions.rs` defines the `Action` enum and `Popup` enum (modal state machine: `NewChangelist`, `Rename`, `MoveFile`, `ConfirmDelete`, `CommitMessage`) and `App::dispatch()`. `app/commit.rs::commit_changelist()` is the highest-risk function in the codebase (see below).
- **`ui/`** — the only ratatui-dependent layer. `ui/mod.rs` lays out the 3-pane view (changelists | files | diff) plus status bar and popups; `ui/keymap.rs` maps `KeyCode` -> `Action` depending on whether a popup has input focus, and special-cases the `MoveFile` popup (a picker, not a text field) to also accept `j`/`k`. `ui/diff_pane.rs` renders colored diff text.

### The isolated-commit invariant

`commit_changelist()` commits exactly one changelist's files via `git commit --only -- <pathspec>` and must **never** call `git add`/`git reset`/any pathspec-affecting command on a path outside the target changelist — that's what guarantees other changelists' staged/partially-staged state is untouched. Two empirically-verified (not assumed) git quirks this code depends on:
- Untracked files need a prior scoped `git add` before `--only` will accept them (git refuses a pathspec with no HEAD/index entry to diff against).
- A rename needs **both** the old and new path in the `--only` pathspec — passing only the new path silently leaves the old path's tree entry behind instead of recording a rename/deletion.

### Diff caching

`App::selected_file_diff()` caches the result keyed on `(path, DiffMode)` in `App.diff_cache`. This exists because the render loop redraws continuously (every ~250ms poll tick) independent of input — without the cache, that means a `git diff` subprocess spawn on every idle frame, which is the main perceived-slowness lever on large or slow (e.g. network-mounted) repos. The cache is invalidated in `App::refresh()`; if you add new state that affects diff output, make sure it goes through `refresh()` or explicitly clears `diff_cache`.

### Persistence format

`.git/changelist.json`: `changelists` have a stable `id` distinct from the display `name` (so renames don't require rewriting the `files` map); `"Default"` always has fixed id `"default"`. Written via tmp-file-then-`fs::rename` for atomicity.

### Pane/focus model

Three focusable panes cycle via `Tab`/`Shift+Tab`: Changelists -> Files -> Diff -> (back to Changelists). The Files pane supports a vim-style visual mode (`Shift+V` to anchor, `j`/`k` to extend, `Esc` to cancel) for batch stage/unstage and batch move; `App::selected_file_paths()` returns either the single selected file or the full visual-mode range depending on `visual_anchor`. Each pane has independent vertical/horizontal scroll state; horizontal scrolling (`h`/`l`) is manual char-skipping for `List` widgets (which have no built-in horizontal scroll) but uses `Paragraph::scroll`'s native horizontal offset for the diff pane.

### Shelve (IntelliJ-style Shelve Changes)

`Shift+S` on a changelist header shelves that changelist: `app/shelve.rs::shelve_changelist()` captures the files' working-tree state as a `git apply`-able patch (`git diff --binary HEAD -- <pathspec>`) under `.git/gitcl-shelf/<id>.patch` (indexed by `.git/gitcl-shelf/shelf.json`, `model/shelf.rs::ShelfStore`, same atomic write-then-rename as the changelist store), then reverts those files to HEAD. `Shift+U` opens the shelf picker: `Enter` unshelves (applies the patch to the working tree only — never the index — and assigns the restored files to a changelist named after the shelf, created or reused by name), `d` deletes a shelf after confirmation.

Invariants and quirks it shares with `commit_changelist` (treat `app/shelve.rs` with the same care as `app/commit.rs` — run the full suite when touching it):
- Never passes a path outside the target changelist to any pathspec-affecting git command. The revert step splits target paths into in-HEAD (scoped `git checkout HEAD --`, which also restores deletions and the old side of renames) and not-in-HEAD (`git rm -q -f -r --` for untracked/added/rename-new paths; `-r` because git can report a whole untracked dir as one `dir/` entry).
- Untracked files need a scoped `git add` before `git diff HEAD` can see them; renames need both old and new path in the pathspec so the patch records the deletion side; a partially-staged file shelves its working-tree content (staged/unstaged split not preserved — same intentional limitation as commit).
- Ordering is the safety property: the patch and shelf index are persisted to disk *before* any revert command touches the working tree, so a mid-operation failure leaves the changes in at least one place, never zero. On unshelve, a failed `git apply` (tree diverged since shelving) leaves the shelf entry and patch untouched.

### Push

`Shift+P` runs plain `git push` (the branch's configured upstream; no auto-detection of remote/branch). Two things make it different from every other git invocation in this codebase:
- It runs on a background thread (`App::start_push()` spawns it, `App::poll_push()` is called once per main-loop iteration to non-blockingly check an `mpsc::Receiver` for the result). Every other git command in `app/` runs synchronously on the main thread because they're local and fast; `push` is network-bound and would otherwise freeze the whole TUI for the duration. While pending, `poll_push()` writes a rotating Braille-spinner string into `status_message` so the wait is visible; the main loop in `main.rs` calls `poll_push()` before every `terminal.draw()`, including idle ticks, so the spinner animates even with no key input.
- It's invoked via `run_git_with_env` (not the plain `run_git` chokepoint) with `GIT_TERMINAL_PROMPT=0` and a null stdin. Without this, a repo with no cached credentials makes git block forever trying to read a username/password from a terminal — but the TUI itself holds the real terminal in raw mode, so that prompt has no usable stdin to read from and would hang indefinitely. With it, git fails fast with a normal `GitError::NonZeroExit`, surfaced as `push failed: ...` in the status bar. Credentials (SSH key, or an HTTPS PAT cached via a credential helper) need to be set up out-of-band; `gitcl` will never prompt for them itself.
- `GIT_TERMINAL_PROMPT=0` only silences prompts issued by *git itself*. ssh's own prompts (unknown-host-key verification, key passphrases) open `/dev/tty` directly, bypassing both the null stdin and that variable — observed in practice: the "Are you sure you want to continue connecting?" prompt scribbled over the raw-mode TUI and raced the event loop for keystrokes. So `push` also sets `GIT_SSH_COMMAND` with `-o BatchMode=yes` appended to the user's configured ssh command (respecting git's precedence: `GIT_SSH_COMMAND` env > `core.sshCommand` config > `GIT_SSH` env > plain `ssh`), making ssh fail fast instead of prompting.
- When the batch push fails with a *prompt-shaped* error (`app/mod.rs::push_error_needs_terminal`: could-not-read-username/password, host key verification failed, permission denied, …), the status bar offers pressing `Shift+P` again, which escalates to an **interactive push**: `App` sets `wants_interactive_push`, and `main.rs::run_interactive_push()` — the only place that owns the terminal — leaves raw mode/alternate screen (`ratatui::restore()`), runs plain `git push` via `git/command.rs::run_git_interactive` (inherited stdio, none of the prompt-suppressing env), so git and ssh prompt natively on the real terminal; it pauses for Enter so their output stays readable, then re-inits the TUI and refreshes. Genuine push errors (non-fast-forward, no upstream) never trigger the hint. This layering matters: `app/` can't suspend the TUI (no ratatui dependency), so it communicates the request via the flag.
