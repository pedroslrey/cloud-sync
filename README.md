# cloud-sync

A lightweight Linux synchronization tool for small Git-backed folders such as source code and LaTeX projects. It watches configured folders, creates recoverable session snapshots, synchronizes them through any standard Git remote, and squashes each completed session into one commit on the base branch.

## Requirements

- Linux with systemd user services
- Git
- A stable Rust toolchain with Cargo
- An existing remote Git repository for each synchronized folder

SSH remotes are recommended for unattended synchronization because SSH keys do not routinely expire.

## Install

Clone and build cloud-sync:

```bash
git clone https://github.com/pedroslrey/cloud-sync.git
cd cloud-sync
cargo build --release
```

If Cargo is not available in the current shell after installing Rust:

```bash
source "$HOME/.cargo/env"
```

Register at least one folder before starting the background service:

```bash
./target/release/cloud-sync add \
  "$HOME/Documents/thesis" \
  git@github.com:USERNAME/thesis.git
```

Test the configuration and perform an initial synchronization:

```bash
./target/release/cloud-sync list
./target/release/cloud-sync status
./target/release/cloud-sync sync thesis
./target/release/cloud-sync finalize thesis
```

Install the binary, systemd user units, safety timer, and shell completion:

```bash
./target/release/cloud-sync install-service
```

Open a new terminal after installation. Verify the service:

```bash
systemctl --user status cloud-sync.service
systemctl --user status cloud-sync.timer
journalctl --user -u cloud-sync.service -f
```

## Adding folders

```bash
cloud-sync add <folder> <remote-url>
```

Examples:

```bash
cloud-sync add "$HOME/Documents/thesis" git@github.com:USERNAME/thesis.git
cloud-sync add "$HOME/Projects/application" git@github.com:USERNAME/application.git
```

When adding a folder, cloud-sync will:

- Create a missing folder.
- Clone a populated remote into an empty folder.
- Initialize Git and import eligible files when the remote is empty.
- Stop for manual reconciliation if local and remote locations both contain unrelated work.
- Detect project presets for Rust, Python, Node, and LaTeX.
- Skip new binary files by default while continuing to synchronize tracked binaries.

The remote repository must already exist. Cloud-sync is provider-independent and does not create GitHub repositories.

## Daily use

List configured repositories:

```bash
cloud-sync list
```

Inspect status:

```bash
cloud-sync status
cloud-sync status thesis
```

Create a snapshot immediately instead of waiting for the inactivity timer:

```bash
cloud-sync sync thesis
cloud-sync sync-all
```

Finalize a session into one base-branch commit:

```bash
cloud-sync finalize thesis
cloud-sync finalize-all
```

Remove a folder from cloud-sync configuration without deleting its files:

```bash
cloud-sync remove thesis
```

## Sessions

Cloud-sync starts a repository session after its first relevant change. By default it:

- Creates a snapshot after five minutes without relevant changes.
- Checks the remote every 15 minutes.
- Does not force snapshots during continuous editing.
- Pushes snapshots to a device-specific session branch.
- Squashes the session into one base-branch commit during finalization.
- Retains completed session branches for seven days.

If another laptop has an unfinished active session, cloud-sync stops and reports it rather than silently taking ownership.

## File filtering

Repository `.gitignore` files are honored. Already tracked files always synchronize, including binaries.

Add persistent include or exclude patterns:

```bash
cloud-sync include thesis "output/final.pdf"
cloud-sync exclude thesis "scratch/**"
```

An explicit include can override `.gitignore` and binary detection. Use overrides carefully to avoid tracking secrets or generated files unintentionally.

Extensionless files are treated normally.

## History and restoration

Show repository or file history:

```bash
cloud-sync history thesis
cloud-sync history thesis chapters/introduction.tex
cloud-sync history thesis --limit 100
```

Inspect a revision:

```bash
cloud-sync show thesis HEAD~2
cloud-sync show thesis HEAD~2 chapters/introduction.tex
```

Restore one file:

```bash
cloud-sync restore thesis HEAD~2 chapters/introduction.tex
```

Restore the complete tracked repository state:

```bash
cloud-sync restore thesis HEAD~2 --yes
```

Cloud-sync creates and pushes a recovery branch before restoring files.

## Backups

List active, completed, and recovery branches:

```bash
cloud-sync backup list thesis
```

Inspect or restore a backup:

```bash
cloud-sync backup show thesis <reference>
cloud-sync backup restore thesis <reference> --yes
```

## Conflicts

List unresolved conflicts:

```bash
cloud-sync conflicts
cloud-sync conflicts thesis
```

Inspect a conflict and its diff:

```bash
cloud-sync conflict show <id>
```

Resolve conflicting files:

```bash
cloud-sync conflict resolve <id> local
cloud-sync conflict resolve <id> remote
cloud-sync conflict resolve <id> newest
cloud-sync conflict resolve <id> keep-both
```

Resolution choices:

- `local` selects the local version of conflicting files.
- `remote` selects the remote version.
- `newest` compares the local and remote commit timestamps.
- `keep-both` keeps remote files at their normal paths and creates local copies with a `.cloud-sync-local` suffix.

Non-conflicting changes from both sides are retained. Before resolution, cloud-sync pushes a recovery branch containing the complete local version.

## Logout integration

Run this before logging out:

```bash
cloud-sync finalize-all
```

A custom logout launcher should only continue when finalization succeeds:

```bash
cloud-sync finalize-all && <desktop-logout-command>
```

The desktop logout command depends on the desktop environment. The systemd service also attempts finalization when stopped, but explicit logout integration provides clearer failure handling.

## Authentication

Prefer SSH repository URLs:

```text
git@github.com:USERNAME/REPOSITORY.git
```

Verify SSH and repository access before enabling the service:

```bash
ssh -T git@github.com
git -C "$HOME/Documents/thesis" fetch origin
```

Cloud-sync invokes standard Git and SSH tools. It does not store passwords, access tokens, or private keys.

## Configuration and logs

Print the configuration path:

```bash
cloud-sync config-path
```

On Linux, configuration is normally stored at:

```text
~/.config/cloud-sync/config.toml
```

Runtime state is normally stored under:

```text
~/.local/state/cloud-sync/
```

After manually changing configuration, restart the daemon:

```bash
systemctl --user restart cloud-sync.service
```

View logs:

```bash
journalctl --user -u cloud-sync.service
```

## Shell completion

`install-service` installs completion for the detected Bash, Zsh, or Fish shell. Completion scripts can also be generated manually:

```bash
cloud-sync completions bash
cloud-sync completions zsh
cloud-sync completions fish
```

Completion includes commands, options, shells, and conflict-resolution choices. Open a new shell after installation.

## Updating

Cloud-sync checks GitHub Releases daily. When a newer version is available, it sends a desktop notification when `notify-send` is installed. Update checks never install software automatically.

Check manually:

```bash
cloud-sync update check
```

Install the latest release:

```bash
cloud-sync update install
```

The updater:

1. Finalizes all active synchronization sessions.
2. Downloads the Linux x86_64 release binary and its SHA-256 file.
3. Rejects unexpected download locations or checksum mismatches.
4. Saves the currently installed binary as `~/.local/bin/cloud-sync.previous`.
5. Stops the user service and atomically replaces the binary.
6. Restarts the service and verifies that it becomes active.
7. Automatically restores and restarts the previous binary if activation fails.

Release-based updates currently support Linux x86_64. Other architectures can continue using source updates.

To update from source instead, finalize sessions and stop the daemon:

```bash
cloud-sync finalize-all
systemctl --user stop cloud-sync.service
```

Then update, verify, rebuild, and reinstall from the source checkout:

```bash
git pull --ff-only
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
./target/release/cloud-sync install-service
```

Verify the service afterward:

```bash
systemctl --user status cloud-sync.service
systemctl --user status cloud-sync-update-check.timer
```
