# Signing and notarizing the macOS bundles

To open a SecLab plugin or the standalone app on **another** Mac, the bundle must be
code-signed with an Apple **Developer ID** certificate and **notarized** by Apple.
Otherwise Gatekeeper blocks it ("cannot be opened because the developer cannot be
verified", or a silent failure to load in the host). Locally-built, unsigned bundles run
fine on the machine that built them — signing matters only for distribution.

This is a manual, credential-bound process (it needs your Apple Developer account), so it
does not run in CI. The `cargo xtask sign` / `cargo xtask notarize` helpers wrap it; this
document explains the whole flow and the one-time setup they assume.

## What gets signed

The four bundles the build produces under `target/bundled/`:

| Bundle | Install location |
| --- | --- |
| `vb-plugin.clap` | `~/Library/Audio/Plug-Ins/CLAP/` |
| `vb-plugin.vst3` | `~/Library/Audio/Plug-Ins/VST3/` |
| `vb-plugin.component` (AU) | `~/Library/Audio/Plug-Ins/Components/` |
| `vb-plugin.app` (standalone) | `/Applications/` |

Build them first (see [the xtask docs](../xtask/src/main.rs)):

```bash
cargo xtask bundle-vst3
cargo xtask bundle-auv2
cargo xtask bundle-standalone
```

## One-time setup

1. **Enroll** in the [Apple Developer Program](https://developer.apple.com/programs/) and,
   in Xcode or the developer portal, create a **Developer ID Application** certificate.
   Install it into your login keychain.

2. **Find your signing identity** — the full certificate name is your `DEVELOPER_ID`:

   ```bash
   security find-identity -v -p codesigning
   ```

   e.g. `Developer ID Application: Your Name (TEAMID)`.

3. **Store notary credentials** once, under a profile name you choose. Use an
   [app-specific password](https://support.apple.com/en-us/102654) for your Apple ID:

   ```bash
   xcrun notarytool store-credentials seclab-notary \
     --apple-id you@example.com --team-id TEAMID
   ```

   `seclab-notary` is then your `NOTARY_PROFILE`.

## Sign

```bash
export DEVELOPER_ID='Developer ID Application: Your Name (TEAMID)'
cargo xtask sign
```

For each bundle this runs, in effect:

```bash
codesign --force --deep --options runtime --timestamp \
  --sign "$DEVELOPER_ID" target/bundled/vb-plugin.vst3
codesign --verify --deep --strict --verbose=2 target/bundled/vb-plugin.vst3
```

- `--options runtime` enables the **hardened runtime**, which notarization requires.
- `--timestamp` embeds a secure timestamp, so the signature stays valid after the
  certificate expires.
- `--deep` signs nested code (the wrapped VST3/AU/standalone embed the CLAP). Apple
  discourages `--deep` for large apps in favour of signing inside-out; for these
  self-contained plugin bundles it is the pragmatic choice. If you add nested frameworks
  later, switch to signing the inner code first, then the outer bundle.

## Notarize

```bash
export NOTARY_PROFILE=seclab-notary
cargo xtask notarize
```

For each **already-signed** bundle this runs, in effect:

```bash
# notarytool needs an archive, not a raw bundle directory
ditto -c -k --keepParent target/bundled/vb-plugin.vst3 target/bundled/vb-plugin.vst3.zip
xcrun notarytool submit target/bundled/vb-plugin.vst3.zip \
  --keychain-profile "$NOTARY_PROFILE" --wait
xcrun stapler staple target/bundled/vb-plugin.vst3
```

`--wait` blocks until Apple returns `Accepted` or `Invalid`. On `Invalid`, inspect the log:

```bash
xcrun notarytool log <submission-id> --keychain-profile "$NOTARY_PROFILE"
```

Stapling attaches the notarization ticket to the bundle so it validates **offline** — the
end user does not need a network round-trip to Apple on first launch. The intermediate
`.zip` is removed after stapling.

## Verify the result

```bash
# Signature is valid and sealed
codesign --verify --deep --strict --verbose=2 target/bundled/vb-plugin.vst3
# Notarization ticket is stapled
xcrun stapler validate target/bundled/vb-plugin.vst3
# Gatekeeper acceptance (use the app for a clearer verdict)
spctl --assess --type execute --verbose target/bundled/vb-plugin.app
```

## Troubleshooting

- **`no signable bundles`** — build the bundles first (`bundle-vst3` / `bundle-auv2` /
  `bundle-standalone`); `sign`/`notarize` only act on what already exists in
  `target/bundled/`.
- **`The specified item could not be found in the keychain`** — the `DEVELOPER_ID` string
  does not exactly match an installed identity; re-check `security find-identity`.
- **Notarization `Invalid` with "not signed with a valid Developer ID"** — the bundle was
  signed without `--options runtime`, or with a Mac App Store / development certificate
  rather than Developer ID. Re-run `cargo xtask sign`.
