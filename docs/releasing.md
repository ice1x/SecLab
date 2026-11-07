# Releasing

How a SecLab release is built, packaged, and (optionally) signed. This covers backlog
[000090](backlog.md) (release bundles in CI) and [000092](backlog.md) (export the synth to
VST3 + AU).

## What a release contains

One zip per OS, each carrying that OS's bundles plus the license documents (`LICENSE`,
`NOTICE`, `EULA-BETA.md` — the beta binaries ship under the freeware beta EULA, the source
under PolyForm Strict; see the README's License section).

### macOS — `SecLab-<tag>-macos-arm64.zip`

| Bundle | Format | Install location |
| --- | --- | --- |
| `seclab-plugin.clap` | CLAP | `~/Library/Audio/Plug-Ins/CLAP/` |
| `seclab-plugin.vst3` | VST3 (clap-wrapper) | `~/Library/Audio/Plug-Ins/VST3/` |
| `seclab-plugin.component` | AUv2 (clap-wrapper) | `~/Library/Audio/Plug-Ins/Components/` |
| `seclab-plugin.app` | wrapped standalone | `/Applications/` |

The VST3 and AU dynamically load the CLAP from the standard locations, so install the
`.clap` alongside them. Apple Silicon only (the hosted `macos-15` runner is arm64).

### Linux — `SecLab-<tag>-linux-x86_64.zip`

| Bundle | Format | Install location |
| --- | --- | --- |
| `seclab-plugin.clap` | CLAP | `~/.clap/` |
| `seclab-plugin.vst3` | VST3 (clap-wrapper, bundle folder) | `~/.vst3/` |
| `seclab-standalone` | native standalone (cpal + eframe) | anywhere on `$PATH` |

### Windows — `SecLab-<tag>-windows-x86_64.zip`

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
runner's own C++ toolchain) and `cargo build --release -p seclab-standalone`. Each leg uploads its zip as a workflow
artifact — **`SecLab-macos-arm64`**, **`SecLab-linux-x86_64`**, **`SecLab-windows-x86_64`**.
The workflow does **not** create a public GitHub Release — download the artifacts from the
run and attach them to a release yourself when you are ready to publish.

The runner image labels are perishable (GitHub retires images roughly yearly and announces
it in [actions/runner-images](https://github.com/actions/runner-images/issues?q=label%3Aannouncement));
when `ci.yml` moves to a newer image, move `release.yml` with it.

## Signing and notarization (macOS, optional)

The macOS leg signs and notarizes **only when the credentials are configured** — otherwise it
uploads unsigned bundles, which Gatekeeper blocks on other Macs (right-click → Open, or
`xattr -d com.apple.quarantine`, works around it for testers). To turn signing on, set these
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

1. Update the version and [CHANGELOG.md](../CHANGELOG.md).
2. Tag and push: `git tag vX.Y.Z && git push origin vX.Y.Z`.
3. The Release workflow builds, (optionally) signs, and uploads the three artifacts.
4. Download them, verify they load (`clap-validator`, `auval -v aumu gWrp clAd` on macOS, or a
   DAW), and attach the zips to a GitHub Release.

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
