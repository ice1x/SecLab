# Releasing

How a SecLab release is built, packaged, and (optionally) signed. This covers backlog
[000090](backlog.md) (release bundles in CI) and [000092](backlog.md) (export the synth to
VST3 + AU).

## What a release contains

Every archive carries the license documents (`LICENSE`, `NOTICE`, `EULA-BETA.md` — the beta
binaries ship under the freeware beta EULA, the source under PolyForm Strict; see the README's
License section). File names use the version without the `v` (tag `v0.1.0-beta.1` →
`0.1.0-beta.1`).

### macOS (Apple Silicon)

| File | What it is |
| --- | --- |
| `SecLab-<version>-macOS-arm64.pkg` | **Installer** — all four bundles, system-wide, one choice per format (CLAP locked on); shows the beta EULA |
| `SecLab-<version>-macOS-arm64-{CLAP,VST3,AU,Standalone}.zip` | one bundle each, for a manual install |
| `SecLab-<version>-macOS-arm64.zip` | all four bundles in one zip |

| Bundle | Format | Installer puts it in |
| --- | --- | --- |
| `seclab-plugin.clap` | CLAP | `/Library/Audio/Plug-Ins/CLAP/` |
| `seclab-plugin.vst3` | VST3 (clap-wrapper) | `/Library/Audio/Plug-Ins/VST3/` |
| `seclab-plugin.component` | AUv2 (clap-wrapper) | `/Library/Audio/Plug-Ins/Components/` |
| `seclab-plugin.app` | wrapped standalone | `/Applications/` |

The VST3, AU and app load the CLAP from the standard CLAP folders at runtime, so the CLAP is
always installed. The installer's bundles are marked non-relocatable (Installer would otherwise
"upgrade" a same-id developer build wherever it finds one instead of installing to `/Library`),
and the AU package's `postinstall` restarts `AudioComponentRegistrar` so hosts see the component
without a reboot. Files installed by a `.pkg` carry no download quarantine. "Export plugin"
finds the stock bundles both in `~/Library` and in `/Library`. Apple Silicon only (the hosted
`macos-15` runner is arm64; the installer refuses Intel Macs).

### Linux — `SecLab-<version>-Linux-x86_64.zip`

| Bundle | Format | Install location |
| --- | --- | --- |
| `seclab-plugin.clap` | CLAP | `~/.clap/` |
| `seclab-plugin.vst3` | VST3 (clap-wrapper, bundle folder) | `~/.vst3/` |
| `seclab-standalone` | native standalone (cpal + eframe) | anywhere on `$PATH` |

### Windows — `SecLab-<version>-Windows-x86_64.zip`

| Bundle | Format | Install location |
| --- | --- | --- |
| `seclab-plugin.clap` | CLAP | `%COMMONPROGRAMFILES%\CLAP\` |
| `seclab-plugin.vst3` | VST3 (clap-wrapper, bundle folder) | `%COMMONPROGRAMFILES%\VST3\` |
| `seclab-standalone.exe` | native standalone (cpal + eframe) | anywhere |

Linux and Windows get the CLAP (with the embedded editor), a VST3 wrapper, and the native
standalone. As on macOS, the VST3 loads the CLAP at runtime from the standard CLAP folder, so
install the `.clap` too. The AUv2 and the wrapped `.app` are macOS-only (`cargo xtask
bundle-auv2` / `bundle-standalone` refuse to run elsewhere); off macOS the standalone is the
native `seclab-standalone`.

Every macOS bundle's `Info.plist` carries the workspace version: `cargo xtask bundle*` passes it
to clap-wrapper and stamps `CFBundleShortVersionString` / `CFBundleVersion` (the numeric core,
`0.1.0`), `CFBundleLongVersionString` (the full `0.1.0-beta.1`) and the AU's component version
(see `xtask/src/version.rs`).

Each CLAP already carries the MVP synth's default patch (the `export` step installs a `.vbp`
into the bundle's resources; the plugin loads `VB_PATCH` → bundled patch → built-in default,
in that order).

## The CI workflow

[`.github/workflows/release.yml`](../.github/workflows/release.yml) builds and packages the
bundles on:

- a **version tag** push — `git tag v0.1.0 && git push origin v0.1.0`, or
- a **manual dispatch** from the Actions tab (or `gh workflow run Release`).

It runs three independent legs on GitHub-hosted runners (`macos-15`, `ubuntu-latest`,
`windows-latest`). The macOS leg calls the existing `cargo xtask` bundle commands (cloning
and building clap-wrapper under `target/` on the first run); the Linux/Windows legs run
`cargo xtask bundle-vst3` (the CLAP with the editor, then the clap-wrapper VST3 built with the
runner's own C++ toolchain) and `cargo build --release -p seclab-standalone`. Each leg uploads
its files as a workflow artifact — **`SecLab-macOS-arm64`**, **`SecLab-Linux-x86_64`**,
**`SecLab-Windows-x86_64`**.

On a **version tag** the macOS leg first runs `cargo xtask release-notes <tag>`, which fails
the run early when the tag does not match the workspace version or `CHANGELOG.md` has no
`## [<version>]` section. A final **publish** job then downloads every leg's files, writes
`SHA256SUMS.txt`, and creates a **draft pre-release** for the tag with all of them attached and
the notes as its body: the section's `### Highlights` (the whole section when it has none) plus
install instructions (installer, Gatekeeper's *Open Anyway*, the `xattr` command for manual
installs, Windows/Linux folders). **Nothing is public until a maintainer opens the draft on the
Releases page, checks it, and clicks Publish.** A manual dispatch builds and uploads the
artifacts only.

The runner image labels are perishable (GitHub retires images roughly yearly and announces
it in [actions/runner-images](https://github.com/actions/runner-images/issues?q=label%3Aannouncement));
when `ci.yml` moves to a newer image, move `release.yml` with it.

## Signing and notarization (macOS, optional)

The macOS leg signs with a Developer ID and notarizes **only when the credentials are
configured** — otherwise `cargo xtask sign-adhoc` seals each bundle with an ad-hoc signature.
Gatekeeper still blocks ad-hoc builds downloaded on another Mac: testers clear it once via
*System Settings → Privacy & Security → Open Anyway* (installer) or `xattr -dr
com.apple.quarantine` (manual zips), as the release notes explain. To turn signing on, set these
as repository **secrets**:

| Secret | Used by | What it is |
| --- | --- | --- |
| `DEVELOPER_ID` | `cargo xtask sign` | the Developer ID Application identity, e.g. `Developer ID Application: Your Name (TEAMID)` |
| `NOTARY_PROFILE` | `cargo xtask notarize` | the name of a `notarytool` keychain profile |

On a GitHub-hosted runner there is no persistent login keychain, so the signing certificate
(+ private key) and the notary credentials additionally have to be imported into the job's
temporary keychain from secrets before `cargo xtask sign` runs — that import step is not
wired yet (it needs an Apple Developer account to test against). See
[signing.md](signing.md) for the identities involved and the full manual flow.

Linux and Windows bundles are unsigned (Windows SmartScreen will warn on first launch of the
standalone; Authenticode signing is a follow-up).

## Cutting a release

1. Bump `version` in the root `Cargo.toml` (e.g. `0.1.0-beta.1`) and move the CHANGELOG's
   _Unreleased_ notes under `## [0.1.0-beta.1] - <date>`; for a long section, start it with a
   short `### Highlights` list — that is what the release page quotes. Check the notes locally:
   `cargo xtask release-notes v0.1.0-beta.1`. Merge.
2. Tag the merged commit and push the tag: `git tag v0.1.0-beta.1 && git push origin v0.1.0-beta.1`.
3. The Release workflow builds, signs (Developer ID or ad-hoc), packages, and opens a **draft**
   pre-release with every file + `SHA256SUMS.txt`.
4. Download the `.pkg` and a zip from the draft, install on a clean user account, check the
   plug-ins load (`auval -v aumu gWrp clAd`, a DAW, `clap-validator`), then **Publish** the draft.

To try the packaging locally (macOS): `cargo xtask sign-adhoc && cargo xtask package-macos v0.0.0-test`
→ `target/dist/`.

## Building locally

The same bundles, without CI:

```bash
cargo xtask bundle seclab-plugin --release --features gui   # CLAP with the editor (any OS)
cargo build --release -p seclab-standalone                 # native standalone (any OS)
cargo xtask bundle-vst3                                    # CLAP + VST3 (any OS; cmake + C++)
cargo xtask bundle-auv2                                    # AUv2 (macOS)
cargo xtask bundle-standalone                              # wrapped standalone .app (macOS)
# → target/bundled/{seclab-plugin.clap,.vst3,.component,.app}, target/release/seclab-standalone
```
