# Updating a test device

`kjk-update` installs published releases from a trusted public GitHub
repository on Windows or Linux x86-64. It is included beside the game in
new packages. It is a standalone native Rust executable, built from
`crates/updater`; it does not start Python or load the game engine.
The default repository is **`shadowndacorner/kjk`**, this testing fork.
The default channel is **stable**, from **main**. Development builds require
`--channel dev`, which selects **develop** and tags matching **`dev-*`**.

This covers Windows on the first-generation ROG Ally Z1 Extreme and Linux
on the Steam Deck or Ally. Native Windows execution and actual handheld
play remain qualification steps; synthetic Linux probes do not establish
device compatibility. macOS packages may appear in a release manifest,
but this updater supports only Windows and Linux x86-64.

An eligible release must contain `kjk-release.json` from the new delivery
workflow. Older releases without it cannot be installed. Until that
workflow lands and a tagged build is published, there may be no eligible
release. Stable drafts become eligible only after the owner publishes
them. Development releases must be published prereleases.

## One-time setup

Download and extract one trusted release package for your operating system.
Keep its `kjk-update.exe` (Windows) or `kjk-update` (Linux) in a convenient
folder outside the managed versions, such as `KJK-test` in your home
directory. Keep `LICENSE.txt`, `NOTICE.txt` and
`THIRD-PARTY-LICENSES-updater.html` with it. No Python, Git, Rust,
administrator access or GitHub token is needed on the test device.

Use that same executable for subsequent commands. Updates install the
packaged updater as part of each version, but **never replace or run that
new updater copy**. To adopt a newer updater, replace your launcher copy
deliberately while it is not running. A developer can build it without the
game engine using `cargo build --locked -p updater`; the binary is written
to `target/debug/`. The Rust toolchain is needed only to build from source.

The following commands run from the folder holding that copy.

### Windows / ROG Ally

In PowerShell:

```powershell
.\kjk-update.exe --channel dev check
.\kjk-update.exe --channel dev update
.\kjk-update.exe --channel dev launch
```

For a shortcut that checks for a new development build on each start,
use this shortcut target, replacing the executable path with yours:

```text
"C:\Users\you\KJK-test\kjk-update.exe" --channel dev launch --update
```

### Linux / Steam Deck

In a terminal, in Desktop Mode on the Deck:

```bash
./kjk-update --channel dev check
./kjk-update --channel dev update
./kjk-update --channel dev launch
```

For Steam Game Mode, add a non-Steam game whose target is the native
updater, such as `/home/you/KJK-test/kjk-update`. Set its launch options:

```text
--channel dev launch --update
```

Use the updater's folder for Steam's "Start In" directory. The updater
starts the game as its child and waits for it. This shortcut arrangement
still needs a real Deck gamepad and Game Mode playtest. The game's
first-run setup locates your own game installs, which remain read-only.

## Select a source

All source options go **before** `check`, `update`, `launch` or `rollback`.
Only `launch --update` and game arguments go after the command.

| Option | Meaning |
|---|---|
| `--repo owner/name` | Trusted public repository; defaults to `shadowndacorner/kjk`. Its identity must match the manifest. |
| `--channel stable` | Published non-prerelease `vX.Y.Z` tags whose manifest names `main`; this is the default. |
| `--channel dev` | Published prereleases; defaults to source branch `develop` and tag glob `dev-*`. |
| `--branch name` | Exact branch recorded by the release manifest. Other development branches must be selected explicitly. Stable is restricted to `main`. |
| `--tag-pattern 'glob'` | Restrict automatic selection by tag; defaults to `v*` for stable or `dev-*` for dev. Quote globs in the shell. |
| `--tag exact-tag` | Choose one published release. Mutually exclusive with `--tag-pattern`; explicitly permits an older or divergent commit. Channel and branch checks still apply. |
| `--root path` | Parent folder for managed installations and profiles. The normal OS data folder is used when omitted. |

Examples on Linux (use `.\kjk-update.exe` instead of `./kjk-update` on Windows):

```bash
# Stable from main, with an update before starting.
./kjk-update launch --update

# A narrower series of develop tags.
./kjk-update --channel dev --tag-pattern 'dev-ally-*' update

# A published manual prerelease from another branch.
./kjk-update --channel dev --branch feature/handheld --tag-pattern 'pre-*' update

# One exact published develop tag, including an intentional older build.
./kjk-update --channel dev --tag dev-ally-001 update

# Pass arguments directly to the installed game.
./kjk-update --channel dev launch -- --help
```

The branch is read from the manifest, not inferred from a GitHub release's
title or its mutable target label. A branch without a published release
cannot be installed. Default dev selection excludes manual `pre-*` tags
and version candidates; select their tag or glob and recorded branch
explicitly. Drafts are never eligible, including with `--tag`.

Automatic selection scans published releases and chooses the newest
publication matching the channel, branch and tag filter. It never uses
GitHub's global "latest" endpoint. When a profile already has a version,
an automatic update to a different commit must also pass GitHub's commit
comparison: the candidate must descend from the installed commit.
A newer publication on an older or divergent history stops the update.
Use `--tag` only when deliberately choosing that history, or `rollback`
for the previous installed version. Same-commit packages are allowed.

`check` reports the available candidate without downloading a package,
creating a profile or changing an installation. The ancestry check against
an installed version happens during `update` and `launch --update`.
`--tag` and `--tag-pattern` affect checking and updating; an offline
`launch` always uses the profile's current installed version.

## Profiles, saves and rollback

The default parent folder is `%LOCALAPPDATA%\kjk-updates` on Windows and
`$XDG_DATA_HOME/kjk-updates` on Linux (`~/.local/share/kjk-updates` when
unset). Each **repository + channel + exact branch** has a separate
subfolder, named by a shortened hash and checked against its `scope.json`.
The updater prints the selected `profile` folder. The native updater keeps
the previous updater's scope hash and installed IDs, so existing managed
profiles and offline launches remain accessible.

| Managed path inside that subfolder | Purpose |
|---|---|
| `versions/<commit-and-archive-id>/` | A complete installed package. Existing versions are never overwritten. |
| `current.json` | Current and previous version IDs, replaced together atomically. |
| `profile/settings.json` | Passed to the game as `KJK_SETTINGS`. |
| `profile/saves/` | Passed as `KJK_SAVES`. |
| `profile/store/` | Passed as `KJK_STORE`; imported data is isolated too. |

Your existing settings, saves and imported store are not copied or
modified. The overrides replace any inherited `KJK_SETTINGS`, `KJK_SAVES`
and `KJK_STORE` values for the launched child. A new profile therefore
starts with its own setup, settings, saves and imported data. Builds in
the same repository/channel/branch share that profile across tags.

**The game's automatic save-version backup is skipped when `KJK_SAVES`
is set.** Managed launches set it, and this updater does not add a backup
system. Keep your own copies of the managed saves and settings before
testing builds whose save compatibility matters.

To switch the current and previous installed binaries, without a network
request:

```bash
./kjk-update --channel dev rollback
./kjk-update --channel dev launch
```

Rollback preserves the profile as it is. It does **not** restore, migrate
or make saves compatible with older binaries. Restoring a save requires
your own saved copy. A repeated update to the same package keeps the
previous pointer; rollback can be repeated to switch back. The first
installation has no previous version.

Versions are retained; this first updater has no pruning command. Storage
must accommodate the downloaded archive and the new extracted package in
addition to retained versions and imported data.

## Failure and trust boundaries

`launch` plays the installed version offline. `launch --update` requires
a successful check/update before starting; a network, rate-limit,
verification or installation failure exits without starting the game.
Run `launch` explicitly to use the existing version after such a failure.
The updater uses the unauthenticated public GitHub API and does not read
credentials or support private repositories.

Each package's exact filename, byte count and SHA-256 must agree with
`kjk-release.json` and the selected GitHub release. The tag must still
resolve to the manifest's commit. The manifest must include Windows and
Linux packages. Missing or malformed metadata stops selection. Download
URLs are constructed from the selected repository, tag and asset name;
metadata cannot supply a command or arbitrary download URL. HTTPS
redirects are restricted to GitHub's asset hosts.
TLS certificate verification uses the operating system's trust store through
rustls; keep the device's system certificate store up to date.

The limits are 1 GiB per archive, 4 GiB expanded tar or file content,
10,000 archive entries (including tar metadata), 32 path components and 240 characters per archive
path. ZIP central directories are limited to 4 MiB; ZIP64, encryption and
compression other than stored/deflate are unsupported. Downloads have a
30-second socket timeout and a ten-minute streaming deadline. Metadata
responses are capped at 4 MiB and manifests at 64 KiB. Local tar metadata
is bounded to 16 KiB and only paths and ordinary ownership/time fields are
accepted; GNU long names are bounded to 240 characters. Global tar metadata,
ACLs, extended attributes and unknown PAX fields are refused. Automatic selection
scans at most 1,000 releases; use an exact tag if the repository exceeds
that limit. These limits can reject a future oversized package safely.

Archives are extracted into a new temporary directory. Absolute paths,
traversal, links, device files, sparse tar files, duplicate paths, case
collisions and unsafe Windows names are refused. Archive paths must use
printable ASCII with a single package root derived from the filename;
tilde names are refused to avoid Windows short-name aliases.
Only verified, complete packages become immutable version directories;
the current pointer changes last. A failed or interrupted installation
before that replacement leaves the selected version unchanged. An
interrupted update may leave an unused staging folder or complete version
that has not been selected.

An OS file lock serializes updates, rollback and updater-launched games
within a profile. A second operation exits as busy while that game is
running. **On Windows, completing the game's first-run setup can spawn a
replacement process and end the updater's wait and lock. Close that game
before another updater operation.** Directly starting an executable also
bypasses the updater's launch lock. Linux setup replaces the existing
process, so the updater continues waiting. The updater never overwrites
an installed executable, including one still running after a restart.

Checksums provide integrity against the trusted repository's release
metadata. This updater does not verify Sigstore attestations or an
independent signature. Choose `--repo` with the same care as downloading
and running that repository's programs yourself.
