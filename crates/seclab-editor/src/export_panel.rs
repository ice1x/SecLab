//! The "Export plugin" button and dialog (000267).
//!
//! The thin egui view over [`seclab_export::export_plugin`]: the user names the current patch,
//! sees the plugin identity it will get (CLAP id, AU code) and whether it replaces an earlier
//! export, and clicks Export. The bundles are cloned from the installed stock SecLab plugin into
//! the user's plug-in folders on a worker thread (a few file copies + `codesign`), and the dialog
//! reports what was installed. The DAW then lists the patch as its own plugin after a rescan.
//!
//! The exporter is injected ([`ExportHost::with_exporter`]) so the whole flow is unit-tested
//! without touching the real plug-in folders. Export is macOS-only for now (the AU/VST3 bundle
//! layout and ad-hoc signing are Apple's); elsewhere the dialog explains that instead.

use std::path::PathBuf;
use std::sync::{mpsc, Arc};

use egui::{self, Ui};
use seclab_export::bundle::{au_subtype, bundle_stem, default_signer};
use seclab_export::{export_plugin, ExportError, Exported, PluginDirs, PluginIdentity};

/// Runs one export: identity + patch RON → installed bundles.
pub type Exporter =
    Arc<dyn Fn(&PluginIdentity, &str) -> Result<Exported, ExportError> + Send + Sync>;

/// Where a dialog's exports land (and where the templates are read from).
fn user_plugin_dirs() -> Option<PluginDirs> {
    std::env::var_os("HOME").map(|home| PluginDirs::user_macos(&PathBuf::from(home)))
}

/// The production exporter: clone the installed stock bundles into the user's plug-in folders.
#[must_use]
pub fn installed_exporter() -> Exporter {
    Arc::new(|identity, ron| {
        let dirs = user_plugin_dirs().ok_or_else(|| ExportError::Io("HOME is not set".into()))?;
        export_plugin(identity, ron, &dirs.templates(), &dirs, &*default_signer())
    })
}

/// What the dialog shows below the form.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Status {
    Idle,
    Exporting,
    Done(Exported),
    Failed(String),
}

/// The export dialog's state and its worker plumbing.
pub struct ExportHost {
    open: bool,
    name: String,
    status: Status,
    exporter: Exporter,
    /// Folder whose `<Name>.clap` tells whether an export would replace an earlier one.
    clap_dir: Option<PathBuf>,
    /// Export is offered only where the bundle format is supported.
    supported: bool,
    rx: Option<mpsc::Receiver<Result<Exported, ExportError>>>,
}

impl Default for ExportHost {
    fn default() -> Self {
        Self::new()
    }
}

impl ExportHost {
    /// A closed dialog exporting into the user's real plug-in folders.
    #[must_use]
    pub fn new() -> Self {
        Self::with_exporter(
            installed_exporter(),
            user_plugin_dirs().map(|d| d.clap),
            cfg!(target_os = "macos"),
        )
    }

    /// A closed dialog with an injected exporter (tests), the CLAP folder used to detect
    /// replacements, and whether export is supported on this platform.
    #[must_use]
    pub fn with_exporter(exporter: Exporter, clap_dir: Option<PathBuf>, supported: bool) -> Self {
        Self {
            open: false,
            name: String::new(),
            status: Status::Idle,
            exporter,
            clap_dir,
            supported,
            rx: None,
        }
    }

    /// Whether the dialog is showing.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Opens the dialog; a finished export's result is cleared so the form starts fresh.
    pub fn open(&mut self) {
        self.open = true;
        if !self.in_flight() {
            self.status = Status::Idle;
        }
    }

    /// Sets the name the patch will be exported under.
    pub fn set_name(&mut self, name: &str) {
        name.clone_into(&mut self.name);
    }

    /// Whether an export is running on the worker.
    #[must_use]
    pub fn in_flight(&self) -> bool {
        self.rx.is_some()
    }

    /// The identity the current name maps to, or why it can't be exported.
    ///
    /// # Errors
    /// A user-facing reason (empty/invalid name, or the reserved stock name).
    pub fn preview(&self) -> Result<PluginIdentity, String> {
        let identity = PluginIdentity::for_name(&self.name).map_err(|e| e.to_string())?;
        let stem = bundle_stem(&identity.name);
        if stem.is_empty() || stem == seclab_export::bundle::TEMPLATE_STEM {
            return Err(ExportError::Reserved.to_string());
        }
        Ok(identity)
    }

    /// Whether exporting the current name would replace an earlier export.
    #[must_use]
    pub fn replaces_existing(&self) -> bool {
        let Ok(identity) = self.preview() else {
            return false;
        };
        self.clap_dir.as_ref().is_some_and(|dir| {
            dir.join(format!("{}.clap", bundle_stem(&identity.name)))
                .exists()
        })
    }

    /// Whether the Export button is enabled.
    #[must_use]
    pub fn can_export(&self) -> bool {
        self.supported && !self.in_flight() && self.preview().is_ok()
    }

    /// Starts exporting `patch_ron` under the current name on a worker thread. A no-op while
    /// one is running or when the name is not exportable.
    pub fn start(&mut self, patch_ron: String) {
        if !self.can_export() {
            return;
        }
        let Ok(identity) = self.preview() else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        let exporter = Arc::clone(&self.exporter);
        std::thread::spawn(move || {
            let _ = tx.send(exporter(&identity, &patch_ron));
        });
        self.rx = Some(rx);
        self.status = Status::Exporting;
    }

    /// Collects a finished export's result. Call once per frame.
    pub fn poll(&mut self) {
        let Some(rx) = &self.rx else {
            return;
        };
        match rx.try_recv() {
            Ok(result) => {
                self.status = match result {
                    Ok(out) => Status::Done(out),
                    Err(e) => Status::Failed(e.to_string()),
                };
                self.rx = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.status = Status::Failed("the export worker stopped unexpectedly".into());
                self.rx = None;
            }
        }
    }

    /// The last finished export, if it succeeded.
    #[must_use]
    pub fn exported(&self) -> Option<&Exported> {
        match &self.status {
            Status::Done(out) => Some(out),
            _ => None,
        }
    }

    /// The last export's error message, if it failed.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        match &self.status {
            Status::Failed(e) => Some(e),
            _ => None,
        }
    }

    /// The toolbar button that opens the dialog.
    pub fn toolbar_button(&mut self, ui: &mut Ui) {
        if ui
            .button("Export plugin")
            .on_hover_text("Export this patch as its own CLAP / VST3 / AU plugin")
            .clicked()
        {
            self.open();
        }
    }

    /// Draws the dialog. Returns `true` when the user clicked Export this frame — the caller
    /// then serializes the current patch and calls [`Self::start`] (the dialog does not own
    /// the patch).
    pub fn ui(&mut self, ui: &mut Ui) -> bool {
        if !self.open {
            return false;
        }
        if self.in_flight() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        let mut open = self.open;
        let mut clicked = false;
        egui::Window::new("Export plugin")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(380.0)
            .show(ui.ctx(), |ui| clicked = self.form_ui(ui));
        self.open = open;
        clicked
    }

    fn form_ui(&mut self, ui: &mut Ui) -> bool {
        if !self.supported {
            ui.label("Exporting a patch as a plugin is available on macOS for now.");
            return false;
        }
        ui.label("Plugin name");
        ui.add(
            egui::TextEdit::singleline(&mut self.name)
                .hint_text("e.g. Acid Bass")
                .desired_width(f32::INFINITY),
        );
        match self.preview() {
            Ok(identity) => {
                ui.weak(format!(
                    "CLAP id {} · AU aumu {} clAd",
                    identity.clap_id,
                    au_subtype(&identity.clap_id)
                ));
                let stem = bundle_stem(&identity.name);
                ui.weak(format!(
                    "Installs {stem}.clap, {stem}.vst3 and {stem}.component into your plug-in folders."
                ));
                if self.replaces_existing() {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        "An export with this name exists and will be replaced.",
                    );
                }
            }
            Err(reason) if !self.name.trim().is_empty() => {
                ui.colored_label(ui.visuals().error_fg_color, reason);
            }
            Err(_) => {}
        }
        ui.separator();
        let mut clicked = false;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.can_export(), egui::Button::new("Export"))
                .clicked()
            {
                clicked = true;
            }
            if self.in_flight() {
                ui.spinner();
                ui.label("exporting…");
            }
        });
        match &self.status {
            Status::Done(out) => {
                ui.separator();
                ui.label(format!("✓ Exported \"{}\"", out.identity.name));
                for path in [Some(&out.clap), out.vst3.as_ref(), out.component.as_ref()]
                    .into_iter()
                    .flatten()
                {
                    ui.weak(path.display().to_string());
                }
                ui.label("Rescan plug-ins in your DAW to see it.");
            }
            Status::Failed(e) => {
                ui.separator();
                ui.colored_label(ui.visuals().error_fg_color, format!("Export failed: {e}"));
            }
            Status::Idle | Status::Exporting => {}
        }
        clicked
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    /// Every (identity, patch RON) a fake exporter was asked to export.
    type Calls = Arc<Mutex<Vec<(PluginIdentity, String)>>>;

    /// A fake exporter recording what it was asked to export.
    fn recording() -> (Exporter, Calls) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&calls);
        let exporter: Exporter = Arc::new(move |identity, ron| {
            seen.lock()
                .unwrap()
                .push((identity.clone(), ron.to_owned()));
            Ok(Exported {
                identity: identity.clone(),
                clap: PathBuf::from(format!("/P/CLAP/{}.clap", identity.name)),
                vst3: None,
                component: None,
                au_subtype: au_subtype(&identity.clap_id),
            })
        });
        (exporter, calls)
    }

    fn wait(host: &mut ExportHost) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while host.in_flight() && Instant::now() < deadline {
            host.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!host.in_flight(), "export did not finish");
    }

    #[test]
    fn a_named_patch_exports_through_the_worker_and_reports_the_result() {
        let (exporter, calls) = recording();
        let mut host = ExportHost::with_exporter(exporter, None, true);
        host.open();
        host.set_name("Acid Bass");
        assert!(host.can_export());
        host.start("(patch)".into());
        assert!(host.in_flight());
        assert!(!host.can_export(), "no second export while one runs");
        wait(&mut host);

        let out = host.exported().expect("export succeeded");
        assert_eq!(out.identity.clap_id, "com.ice1x.seclab.user.acid-bass");
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1, "(patch)");
    }

    #[test]
    fn an_empty_or_reserved_name_cannot_be_exported() {
        let (exporter, calls) = recording();
        let mut host = ExportHost::with_exporter(exporter, None, true);
        for name in ["", "   ", "seclab-plugin", "..."] {
            host.set_name(name);
            assert!(!host.can_export(), "{name:?}");
            host.start("(patch)".into());
        }
        // The stock plugin's own id is refused by the exporter itself (ExportError::Reserved);
        // a user name can never map to it, since user ids live under `…seclab.user.`.
        host.set_name("SecLab");
        assert!(host
            .preview()
            .unwrap()
            .clap_id
            .starts_with(seclab_export::identity::USER_ID_PREFIX));
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn a_failed_export_shows_the_reason() {
        let exporter: Exporter = Arc::new(|_, _| {
            Err(ExportError::MissingTemplate(PathBuf::from(
                "/P/seclab-plugin.clap",
            )))
        });
        let mut host = ExportHost::with_exporter(exporter, None, true);
        host.set_name("Pad");
        host.start("(patch)".into());
        wait(&mut host);
        assert!(host.exported().is_none());
        assert!(host.error().unwrap().contains("not installed"));
    }

    #[test]
    fn reopening_after_an_export_starts_a_fresh_form_but_keeps_the_name() {
        let (exporter, _) = recording();
        let mut host = ExportHost::with_exporter(exporter, None, true);
        host.open();
        host.set_name("Pad");
        host.start("(patch)".into());
        wait(&mut host);
        assert!(host.exported().is_some());
        host.open();
        assert!(host.exported().is_none());
        assert_eq!(host.preview().unwrap().name, "Pad");
    }

    #[test]
    fn an_existing_export_of_the_same_name_is_flagged_as_replaced() {
        let dir = std::env::temp_dir().join(format!("seclab-export-panel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Acid Bass.clap")).unwrap();
        let (exporter, _) = recording();
        let mut host = ExportHost::with_exporter(exporter, Some(dir.clone()), true);
        host.set_name("Acid Bass");
        assert!(host.replaces_existing());
        host.set_name("Pad");
        assert!(!host.replaces_existing());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unsupported_platforms_never_export() {
        let (exporter, calls) = recording();
        let mut host = ExportHost::with_exporter(exporter, None, false);
        host.set_name("Pad");
        assert!(!host.can_export());
        host.start("(patch)".into());
        assert!(!host.in_flight());
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn the_dialog_renders_in_every_state_without_panicking() {
        let (exporter, _) = recording();
        let ctx = egui::Context::default();
        let mut host = ExportHost::with_exporter(exporter, None, true);
        let frame = |host: &mut ExportHost| {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    host.poll();
                    host.ui(ui);
                });
            });
        };
        frame(&mut host); // closed: draws nothing
        host.open();
        frame(&mut host); // empty form
        host.set_name("bad/name:\u{7}");
        frame(&mut host); // invalid name
        host.set_name("Acid Bass");
        host.start("(patch)".into());
        frame(&mut host); // exporting
        wait(&mut host);
        frame(&mut host); // done
        assert!(host.is_open());
        let mut off = ExportHost::with_exporter(recording().0, None, false);
        off.open();
        frame(&mut off); // unsupported note
    }
}
