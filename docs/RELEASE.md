# Releases: the player's build and the first-run setup

Status: started 2026-10-25 (`ROADMAP.md`). The player's build, its
package and the first-run setup are in; a gamepad scheme for play,
autosave, GOG's registry and runs on Linux and the Steam Deck are not
(below, "Not yet").

No game data is ever packaged: a release contains `kjk`, its save editor,
the update script, a readme and the licences. The player's own installs are
found or chosen on the first start and only ever read.

## The player's build

```bash
cargo xtask dist                 # build and pack for this machine's OS
cargo xtask dist --skip-build    # pack the last build again
cargo xtask dist --pgo <profile> # kjk built with a profile (below)
cargo xtask dist --dlss          # Windows and Linux: with DLSS (below)
```

`dist` builds `kjk` with the `dist` profile (`Cargo.toml`: release with
fat LTO in one codegen unit, no incremental state, no symbols) and
`--no-default-features`:
no verbs that check and measure, no probes and no Steam, the game alone
with co-op by address. A release ships MIT code and the crates' own
licences only: Steam would bring Valve's library (`libsteam_api`, under
the Steamworks SDK's terms) and an app id kjk does not have yet (it opens
as Valve's test app 480), so Steam co-op is for builds made from source
(`cargo build --features steam`, the default). The package goes to
`target/packages/`:

| OS | package | inside |
|---|---|---|
| Windows | `kjk-<version>-windows-<arch>.zip` | `kjk.exe`, `kjk-save-editor.exe`, update script and texts below |
| macOS | `kjk-<version>-macos-<arch>.tar.gz` | `kjk.app`, `kjk Save Editor.app`, update script and texts below |
| Linux, Steam Deck | `kjk-<version>-linux-<arch>.tar.gz` | `kjk`, `kjk-save-editor`, update script and texts below |

Every archive also contains `kjk-update.py`, `README.txt`, `LICENSE.txt`,
`NOTICE.txt`, `THIRD-PARTY-LICENSES.html` and
`THIRD-PARTY-LICENSES-save-editor.html`. The updater supports Windows and
Linux and needs Python 3.10 or newer; its setup and channel choices are in
[`UPDATES.md`](UPDATES.md). The macOS package keeps the same script for
consistent contents, but the updater does not install macOS builds.

CI sets `KJK_RELEASE_TAG` to the exact verified tag. The package name, game
menu, launcher version, editor and saved build metadata all carry that
version. A local build without the variable keeps the existing behavior:
package names use `git describe --tags --match 'v*' --always --dirty`, and
the binary's version is its nearest `v*` tag. Each OS packs its own:
Windows on Windows, macOS on a Mac, and Linux on Linux.

### DLSS in a Windows or Linux package

No release package has DLSS yet. `release.yml` packs every package with a
plain `dist`. NVIDIA's licence asks for its written approval of its marks
before a release shows them, and that approval has not been given. To ship
DLSS, give the Windows and Linux lines of `release.yml`'s matrix
`dist: --dlss`. The rest of this section describes such a package.

`dist --dlss` packs Windows and Linux with DLSS: `kjk` built with
`--no-default-features --features dlss` against NVIDIA's DLSS SDK v310.5.3
(`DLSS_SDK`, `BUILD.md`, "DLSS"; CI checks out the files it needs through
`.github/actions/dlss-sdk`), and beside it what NVIDIA's licence and its
programming guide (sections 4.2 and 9.5) ask a program to ship:

| file | from the SDK |
|---|---|
| `nvngx_dlss.dll` (Windows), `libnvidia-ngx-dlss.so.310.5.3` (Linux) | `lib/<platform>/rel/`, the signed release library; never `dev/`'s, nor the frame generation (`dlssg`) or ray reconstruction (`dlssd`) ones, which kjk does not use |
| `NVIDIA-RTX-SDK-LICENSE.txt` | `LICENSE.txt` |
| `NVIDIA-DLSS-NOTICES.txt` | the guide's section 9.5 to 9.6 (notices, trademarks, copyright, the third-party code in the library), read out of `doc/DLSS_Programming_Guide_Release.pdf` by `pdftotext` (poppler-utils; xpdf on Windows) |

The readme gains a DLSS paragraph and `THIRD-PARTY-LICENSES.html` the
crates of the feature (`dlss_wgpu`, `ash`, ...). NVIDIA's library must keep
its own signature: nothing re-signs or strips it (`dist` signs only the
macOS app, which has no DLSS). The macOS package is as before.

What a DLSS build changes for every player, card or not: Bevy makes the
renderer through a Vulkan instance alone (so it can add NVIDIA's
extensions), where wgpu would otherwise pick among Vulkan and DX12 (on
Windows, Vulkan first when the driver has it). When no Vulkan instance
can be made (no Vulkan driver), it falls back to wgpu's own choice, DX12
on Windows; `WGPU_BACKEND` is then read as before. Without an RTX card, or
a driver without DLSS, the DLSS choices are hidden and a saved one plays as
TAA (`temporal::choose`).

NVIDIA's licence, as it bears on kjk (the NVIDIA RTX SDKs licence and its
supplement, v. 14 March 2024, in the SDK's `LICENSE.txt`): the SDK's
distributable parts may ship in object code inside an application that does
more than the SDK, under terms at least as protective as NVIDIA's, and
never on their own; they may not be reverse engineered, nor put under an
open source licence, so they ship beside the MIT code under NVIDIA's own
licence (`NOTICE`). DLSS is licensed for systems with NVIDIA GPUs. The
supplement also asks for NVIDIA to be told before a commercial release
(developer.nvidia.com/sw-notification) and, for a game using DLSS, the
SDK's use and NVIDIA's marks credited on its splash screen, about box and
credits, with NVIDIA approving each use of its marks; kjk has neither yet
(owner's call, below).

The app's `Info.plist` carries the release tag's numbers (`v0.2.1` is
0.2.1), 0.0.0 on a build that is not a release.
`THIRD-PARTY-LICENSES.html` is written by `cargo about` (`about.toml`,
`about.hbs`; `cargo install --locked cargo-about --features cli`), for
the crates the player's build is made of. The icons are the logo,
`docs/logo/kjk.svg` (our own design, no game's art), rendered by `xtask`
(`xtask/src/icon.rs`): the macOS app's as it packs (made into an
`.icns` by `iconutil`); `kjk.exe`'s (`crates/launcher/kjk.ico`, built in
through `kjk.rc`) and the window's (`crates/game/icon.png`) are kept in
the repo and rendered again by `cargo xtask icon` when the logo changes.

Sizes, macOS arm64 (2026-10-27):

| profile | `kjk` | packed | build |
|---|---|---|---|
| release (development, with probes) | 129 MB, 86 MB stripped | 30 MB | |
| dist, thin LTO, 16 codegen units | 99 MB | 33.7 MB | 7.5 min |
| **dist, fat LTO, 1 codegen unit** (in use) | **67 MB** | **28.6 MB** | 16.5 min |

### A profile-guided build

```bash
rustup component add llvm-tools          # once: llvm-profdata
cargo xtask pgo                          # target/pgo/kjk.profdata (~1 h)
cargo xtask dist --pgo target/pgo/kjk.profdata
```

`pgo` builds `kjk` as `dist` does but counting what it runs (its own target
folder, `target/pgo/gen`), plays the bench's scenes with it on the default
and low presets (`kjk bench --exe`, driven by a development build, so it
needs the games), and merges the counts. `dist --pgo` then builds `kjk`
laid out by them: hot code together and inlined, cold code apart. The
picture is the same; only where the time goes changes. A function changed
since the profile was made is built as without one, so a profile goes stale
slowly but should be made again for each release.

Measured on Windows (Ryzen 7 3700X, RTX 4070 Ti SUPER, 2026-11-07), the
same commit's `dist` build without and with a profile, `kjk bench`, back to
back:

| fps | low | default |
|---|---|---|
| Javyar's cantina | 360 → 430 (+19%) | 229 → 262 (+14%) |
| Harbinger | 274 → 332 (+21%) | 215 → 246 (+14%) |
| Taris Undercity | 323 → 377 (+17%) | 312 → 348 (+12%) |
| Arena, 8 foes | 354 → 412 (+16%) | 288 → 320 (+11%) |
| Citadel Station (not trained on) | 328 → 378 (+15%) | 249 → 284 (+14%) |
| Kashyyyk's Great Walkway (not trained on) | 335 → 387 (+16%) | 216 → 245 (+13%) |

The last two areas are `kjk bench --scenes telos,kashyyyk`, left out of the
training, so the gain is not the trained scenes' alone. `release.yml` has
no games to train on, so a release with a profile takes one made on the
owner's machine (committed, or the Windows package built by hand); the
releases are built without one until the owner decides.

### Before the tag

Players keep their saves across releases, so a tag waits for the owner's
machine, which has the games (`release.yml` has none) and the corpus of real
saves (`docs/superpowers/specs/2026-10-28-save-migrations-design.md`, 2b):

```bash
kjk saves capture v0.5.0 --from <slot, zip or .kjkv>...  # this release's saves
kjk saves check --mute                                    # the whole corpus
```

1. **Capture** the release's corpus under its tag: the chain's checkpoints
   for the Endar Spire, Taris, mid-Korriban and TSL's Peragus (no `--from`
   takes them from `kjk-artifacts/checkpoints/<game>/`), plus any owner save
   worth keeping. They go to `kjk-artifacts/save-corpus/<tag>/<name>/`
   (git-ignored), each with a `story.json` made if it has none.
2. **Check** the full corpus, every release's saves on this build. The table
   says for each save the formats migrated, stamps moved, what reconciling
   did, whether its story restores, and PASS, EXPECTED (its release is before
   a `breaking` entry of `bumps/BREAKING.md`) or FAIL. Any FAIL stops the tag.
3. **Mirror** `kjk-artifacts/save-corpus/` to the private archive, so the
   corpus is not lost with a disk.

### Releases from CI

The release workflow has two automatic channels:

| Tag | Source commit | Release behavior |
|---|---|---|
| `vMAJOR.MINOR.PATCH` | reachable from `origin/main`, with no pending `bumps/*.toml` | Draft full Release; the owner completes the save-corpus review above and publishes |
| `dev-<identifier>` | reachable from `origin/develop` | Published prerelease after the required gates, explicitly never Latest |
| `vMAJOR.MINOR.PATCH-<suffix>` | reachable from main or develop | Draft prerelease; records main when reachable from both |

The development identifier and candidate suffix start with an ASCII letter
or digit and contain only ASCII letters, digits, dots, underscores and
hyphens. Release tags have at most 128 characters. Examples are
`dev-20261123.1` and `v0.20.0-rc.1`. A tag starts the workflow from the commit
it names, so these workflows must first reach the relevant branch before a
new tag can use them. Tagging an older commit with an older workflow does
not retrofit its automation.

The workflow resolves annotated and lightweight tags to the exact checked-out
commit, verifies that the remote tag still names it, and checks ordinary
ancestry with `git merge-base --is-ancestor`. It does not require today's
branch tip: historical branch commits can be tagged and rerun. The tag and
branch checks run again immediately before publication. Branch names in
`push` filters alone would not prove a tag's ancestry. Stable tags require
pending bumps to have been pinned through the existing process in
[`../bumps/README.md`](../bumps/README.md); this workflow never edits format
versions or the sim hash.

Every tag runs the same no-content CI gate as every pull request: the existing
approved suite and a player build/startup on Windows and Linux, plus the
Linux source and license checks (`BUILD.md`). `release.yml` calls `ci.yml`
directly for release tags, so publication depends on those results. A malformed
release tag is refused for packaging while its CI still runs.

The packages use `cargo xtask dist`, with locked builds, on Windows x86-64,
Linux x86-64 (Ubuntu 22.04), and macOS arm64. Each completed archive is extracted
into a fresh temporary directory. Its exact program, updater and license
contents are checked; links, extra paths and duplicate paths are refused;
both programs run with `--help`; `kjk --version` must identify the exact tag
and commit. macOS also checks its plists and ad-hoc signatures.

Both Windows and Linux packages and their full CI gate are required before
any Release is created. macOS remains optional because its hosted runner may
be unavailable. Its package is included only when its build and determinism
check pass; otherwise the notes name its absence. The workflow keeps package
artifacts even when release creation is blocked. It never replaces an existing
Release or its assets on a rerun, including published development builds.
Delete an unwanted draft manually before rerunning; use a new tag for another
published build.

Each Release has `SHA256SUMS` and `kjk-release.json`. The manifest records the
repository, channel, source branch, exact tag and commit, and each available
archive's platform, name, size and SHA-256. It follows
`tools/release-manifest.schema.json`; `version` equals `tag`. Checksums cover
the archives and manifest. The Windows/Linux updater selects only published
Releases whose metadata matches its selected channel and branch, then checks
the archive's bytes (`UPDATES.md`). A checksum checks integrity against that
metadata; it is not independent signature verification.

Each downloaded archive also retains the existing build provenance attestation
(`actions/attest-build-provenance`), identifying the repository, workflow,
commit and archive hash. Check it with
`gh attestation verify <archive> --repo <owner>/<repository>`. It proves build
origin, not that the code or gameplay is correct. A package built by hand has
no workflow attestation.

Main and develop pushes prewarm package caches when the lockfile, toolchain
or delivery workflow changes; those runs create no Release. Tags read caches
available to their ref and the default branch, not automatically develop's
cache or another tag's cache.

#### Manual builds and pre-releases

Actions → release → Run workflow still builds packages from the chosen branch
as workflow artifacts, without a Release by default. Tick **pre-release** to
create a draft prerelease for owner review. Its explicit package/version tag is
`pre-<branch>-<git describe>`: unsafe branch-name characters become hyphens and
very long names are shortened to fit the tag limit. The Release targets the
run's exact commit. Existing drafts are refused rather than overwritten.

Manual and candidate prereleases use the `dev` manifest channel and record
their actual verified source branch. They are excluded from the updater's
default develop selection, which requires both branch `develop` and `dev-*`.
Select an experimental branch and tag pattern explicitly to use one. A
prerelease skips the stable save-corpus review and must not be treated as
proof that older saves are compatible. Prereleases are neither announced by
`discord-release.yml` nor uploaded by the automatic Nexus release event.

Publishing a stable draft announces it in the Discord server's #releases
channel (`.github/workflows/discord-release.yml`, the message in
`.github/discord-release.jq`): the repository secret
`DISCORD_RELEASES_WEBHOOK` holds the channel's webhook, and the variable
`DISCORD_RELEASE_ROLE`, a role's id, is pinged when set. Progress goes to
#progress by hand: `cargo xtask discord "caption" clip.mp4 shot.png`
(its webhook in `KJK_DISCORD_PROGRESS_WEBHOOK` or the git-ignored
`context/discord-progress-webhook`; larger files are shrunk to Discord's
20 MB, `--dry-run` sends nothing).

Publishing a stable release also puts the packages on the Nexus Mods page,
nexusmods.com/kotor/mods/1911 (`.github/workflows/nexus-release.yml`,
the calls in `.github/nexus-upload.sh`): the repository secret
`NEXUS_API_KEY` holds the owner's key (nexusmods.com/settings/api-keys);
without it nothing is uploaded. Each OS's package becomes a new main
version of its own file on the page, the file named by the variable or
secret `NEXUS_FILE_<PLATFORM>` (`NEXUS_FILE_MACOS_AARCH64`,
`NEXUS_FILE_WINDOWS_X86_64`, `NEXUS_FILE_LINUX_X86_64`): the API's id for
the file, or the `file_id` in a download link of any of its versions. The
mod's version follows. Nexus takes no `.tar.gz`, so the macOS and Linux
packages go up repacked as `.zip` (modes and symlinks kept). A package with no such variable is added as a new file,
and the run's notice names its id to save as the variable. Actions →
nexus release → Run workflow with a tag uploads that Release again.

The macOS app is **not signed or notarised** (the owner's decision,
2026-10-25: no Apple developer account). `dist` seals it with an ad-hoc
signature (`codesign --sign -`): left as the linker signs it, the plist
and icon break the program's signature and a downloaded app is
"damaged", which nothing in Gatekeeper's dialogs gets past. Sealed,
Gatekeeper refuses it on the first open as from an unknown developer;
the player's readme says how through: System Settings → Privacy &
Security → Open Anyway (right-click → Open up to macOS 14), or `xattr
-dr com.apple.quarantine kjk.app`.

What differs in the player's build (without `probes`):

- **Runs' folders** (log, manifest, a script's screenshots) go to the
  user's folder's `logs/` (beside `settings.json` and `saves/`), and only
  the last 10 are kept (`runtime::KEPT_RUNS`). A development build keeps
  every run in `kjk-artifacts/runs/` under the working folder, as before.
- Every verb under `kjk` that checks or measures is absent; `kjk --help`
  lists what plays.

## The first-run setup (`crates/game/src/setup.rs`)

A plain `kjk` (no script, not headless, no save to load, not back from a
game) opens on the setup when a game is not found or the setup was never
done (`setup.done` in the settings). `kjk setup` opens it at any time;
`kjk setup --start` starts the import at once (with `--headless --cmds`,
for scripted checks).

It draws with Bevy's own UI and font (the `bevy_ui`, `bevy_text` and
`default_font` features), as no game's may be found yet. For each of the
three games it shows the folder and how it was found (Steam, GOG, chosen,
or an environment variable), or why a folder was refused, or that none was
found, with:

- **Locate…**: the platform's folder dialog (`rfd`: Windows' and macOS'
  own, the XDG desktop portal on Linux and the Deck);
- **Type path**: the folder typed or pasted (Bevy's `EditableText`, with
  the system clipboard), Enter to take it, Escape to leave it.

A chosen folder goes in the settings as `install.k1`, `install.ja`,
`install.tsl`; `discover` reads them after the environment's variables
(`KJK_KOTOR1`, …) and before `.env`. A folder that is not that game is a
refusal, shown with its reason, never replaced by another install.

When all three are found, **Import and play** runs on a thread: Jedi
Academy's and KOTOR II's files opened (a broken install is told here, not
in the menu), then KOTOR's install fingerprinted and its templates and
voices made into the store (`import_verbs::prepare`, what `kjk import k1`
does before its checks). Then `setup.done` is saved and the program starts
again on its main menu (`setup::restart`; a new process, as winit makes one
event loop a process).

Mouse, keys (arrows, Tab, Enter, Space, Escape) and gamepads (D-pad or the
left stick, A, B) all drive it.

## Where a player's files are

| what | Windows | macOS | Linux |
|---|---|---|---|
| settings, saves, logs | `%APPDATA%\kjk` | `~/Library/Application Support/kjk` | `$XDG_CONFIG_HOME/kjk` (`~/.config`) |
| the store (imported data) | `%LOCALAPPDATA%\kjk` | `~/Library/Caches/kjk` | `$XDG_CACHE_HOME/kjk` (`~/.cache`) |

Nothing is written next to the program or into a game's folder.

## Not yet

- **The gamepad on real hardware.** Play, the menus (with their hint bar),
  rebinding and the Deck's keyboard are built and probed (`docs/controls`),
  but no physical pad or Deck has played a build yet.
- **Autosave** (the option is greyed).
- **GOG.** Its two default Windows folders only: no registry (a custom
  folder), no Heroic or Lutris on Linux, no macOS. The setup's Locate covers
  the rest.
- **Linux and the Steam Deck** have not been played on; CI builds the
  Linux package and starts it. The Deck's Game Mode
  folder dialog depends on its desktop portal; Type path is the fallback.
- **KOTOR II** is required for the menus' look; making it optional (KOTOR's
  own menus) would let more players in.
- **Movies** need `ffmpeg` on the `PATH`; without it they are skipped.
- **DLSS from a release package.** A local build has run DLAA and the four
  DLSS modes on an RTX 4070 Ti SUPER. No release package has been played
  with DLSS yet. CI builds and links the feature (ci.yml's `dlss` job).
- **NVIDIA's marks.** NVIDIA has been notified of the release. Its written
  approval of its marks on a splash screen and in the credits has not been
  asked for. Releases are packed without DLSS until it is given (NVIDIA's
  licence, above). The splash screen that carries the marks is not built.
- **A Linux AppImage**, and Linux and Windows on ARM. `release.yml` packs
  macOS on Apple silicon (no Intel build), Windows and Linux on x86-64.
