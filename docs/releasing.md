# Releasing

How a VST_bomb release is built, packaged, and (optionally) signed. This covers backlog
[000090](../README.md) (release bundles in CI) and [000092](../README.md) (export the synth
to VST3 + AU).

## What a release contains

The four macOS bundles the build produces under `target/bundled/`, packaged into one zip
(`VST_bomb-<tag>-macos.zip`):

| Bundle | Format | Install location |
| --- | --- | --- |
| `vb-plugin.clap` | CLAP | `~/Library/Audio/Plug-Ins/CLAP/` |
| `vb-plugin.vst3` | VST3 (clap-wrapper) | `~/Library/Audio/Plug-Ins/VST3/` |
| `vb-plugin.component` | AUv2 (clap-wrapper) | `~/Library/Audio/Plug-Ins/Components/` |
| `vb-plugin.app` | wrapped standalone | `/Applications/` |

The VST3 and AU dynamically load the CLAP from the standard locations, so install the
`.clap` alongside them. Each bundle already carries the MVP synth's default patch (the
`export` step installs a `.vbp` into the bundle's resources; the plugin loads `VB_PATCH` →
bundled patch → built-in default, in that order).

## The CI workflow

[`.github/workflows/release.yml`](../.github/workflows/release.yml) builds and packages the
bundles on:

- a **version tag** push — `git tag v0.1.0 && git push origin v0.1.0`, or
- a **manual dispatch** from the Actions tab (or `gh workflow run Release`).

It runs on the self-hosted macOS/ARM64 runner, calls the existing `cargo xtask`
bundle commands (reusing the cached clap-wrapper checkout), zips the four bundles, and
uploads them as the **`VST_bomb-macos-bundles`** workflow artifact. It does **not** create a
public GitHub Release — download the artifact from the run and attach it to a release
yourself when you are ready to publish.

### macOS-only, for now

Only the macOS leg is wired: the self-hosted macOS runner is the only one available while
the account's GitHub-hosted runner billing is unavailable (see
[`cross-platform.yml`](../.github/workflows/cross-platform.yml)). Linux/Windows release legs
join this workflow once that is restored.

## Signing and notarization (optional)

The workflow signs and notarizes **only when the credentials are configured** — otherwise it
uploads unsigned bundles, which run on the build machine but are blocked by Gatekeeper on
other Macs. To turn signing on, set these as repository **secrets**:

| Secret | Used by | What it is |
| --- | --- | --- |
| `DEVELOPER_ID` | `cargo xtask sign` | the Developer ID Application identity, e.g. `Developer ID Application: Your Name (TEAMID)` |
| `NOTARY_PROFILE` | `cargo xtask notarize` | the name of a `notarytool` keychain profile stored on the runner |

The signing identity (certificate + private key) and the `notarytool` keychain profile must
exist **on the self-hosted runner's login keychain** — secrets only select which identity/
profile to use. See [signing.md](signing.md) for the one-time setup and the full manual
flow.

## Cutting a release

1. Update the version and [CHANGELOG.md](../CHANGELOG.md).
2. Tag and push: `git tag vX.Y.Z && git push origin vX.Y.Z`.
3. The Release workflow builds, (optionally) signs, and uploads the bundles artifact.
4. Download `VST_bomb-macos-bundles`, verify it loads (`auval -v aumu gWrp clAd`,
   clap-validator, or a DAW), and attach the zip to a GitHub Release.

## Building locally

The same bundles, without CI:

```bash
cargo xtask bundle vb-plugin --release   # CLAP
cargo xtask bundle-vst3                   # VST3
cargo xtask bundle-auv2                   # AUv2
cargo xtask bundle-standalone             # standalone .app
# → target/bundled/{vb-plugin.clap,.vst3,.component,.app}
```
