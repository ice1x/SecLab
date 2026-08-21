# Real-host testing checklist

Automated CI covers a lot — `clap-validator` runs on every push, and the engine, nodes,
project format, and assistant are unit- and integration-tested — but it cannot tell you
whether the plugin actually **loads, plays, and survives a session** inside a real DAW.
This is the manual checklist for that: run it before calling a build releasable, and after
any change to the plugin wrapper, state format, or parameter set.

It is a manual procedure (it needs a DAW and your ears), so it is not part of CI.

## 0. Build and install

Build the bundles and copy each into its standard location:

```bash
cargo xtask bundle-vst3
cargo xtask bundle-auv2
cargo xtask bundle-standalone
```

| Bundle | Copy to |
| --- | --- |
| `vb-plugin.clap` | `~/Library/Audio/Plug-Ins/CLAP/` |
| `vb-plugin.vst3` | `~/Library/Audio/Plug-Ins/VST3/` |
| `vb-plugin.component` | `~/Library/Audio/Plug-Ins/Components/` |
| `vb-plugin.app` | run directly (standalone) |

For distribution to another machine, sign and notarize first — see
[`docs/signing.md`](signing.md). On the build machine, unsigned bundles load fine.

## 1. Pre-flight validation (before opening a DAW)

- [ ] **CLAP** — `cargo xtask validate` passes (bundles + runs `clap-validator`).
- [ ] **AU** — `auval -v aumu gWrp clAd` reports the AU as valid (`auval` ships with macOS).
- [ ] **VST3 / CLAP** — optionally run [`pluginval`](https://github.com/Tracktion/pluginval)
      at strictness 8+ over `vb-plugin.vst3`.

## 2. Standalone smoke test

- [ ] `vb-plugin.app` launches and shows the editor window (the run-view: patch summary +
      parameters).
- [ ] Audio device opens; the default patch produces sound.
- [ ] Quitting and relaunching is clean (no crash, no stuck audio thread).

## 3. In each DAW — run for Reaper **and** Logic Pro

Reaper is the quick iteration host (fast rescan, all formats); Logic is the AU
gatekeeper (strict AU validation, sandboxed). Test the relevant format in each: CLAP/VST3
in Reaper, AU in Logic.

### Scan & instantiate
- [ ] The plugin appears in the DAW's plugin list after a rescan (in Logic, AU must pass
      the system validation on first scan).
- [ ] Add it to an instrument track without error; the editor window opens and shows the
      loaded patch's node/edge summary and parameters.

### Play
- [ ] Incoming MIDI notes produce sound (play from a MIDI keyboard or a drawn clip).
- [ ] No zipper noise, clicks, or dropouts at the default block size.
- [ ] Change the host sample rate (44.1k ↔ 48k ↔ 96k) and buffer size; audio stays correct
      and glitch-free after each change.

### Automate
- [ ] Every exposed parameter appears in the host's automation list.
- [ ] Writing and playing back automation changes the sound as expected; the editor's
      parameter read-out follows the automation.

### Persist (state — task 000068)
- [ ] Save the project, close it, reopen it: the plugin restores the **same patch** and the
      **same parameter values** (the plugin serializes its patch into host session state).
- [ ] Duplicating the track duplicates the patch faithfully.
- [ ] Two instances in one project stay independent.

### Render
- [ ] An offline bounce / freeze produces audio identical to real-time playback (no
      allocation or denormal blow-ups off the audio thread).

### Stability
- [ ] Leave it playing for several minutes: no rising CPU, no memory growth, no crash.
- [ ] Remove and re-add the plugin repeatedly without error.

## 4. Record the result

For each (host × format) combination, note pass/fail and the versions:

| Host | Format | DAW version | OS | Result | Notes |
| --- | --- | --- | --- | --- | --- |
| Reaper | CLAP | | | | |
| Reaper | VST3 | | | | |
| Logic Pro | AU | | | | |
| — | Standalone | | | | |

File a bug for any failure with the host, format, versions, and repro steps. A crash log
(`~/Library/Logs/DiagnosticReports/`) and, for AU, the `auval` output should be attached.
