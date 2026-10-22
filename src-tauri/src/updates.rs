//! Signed desktop updates. The application owns the idle/maintenance policy;
//! this module never opens a wallet, changes AppData, or starts a chat process.

use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::{Update, UpdaterExt};

pub const UPDATE_EVENT: &str = "update-status";
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub status: String,
    pub current_version: String,
    pub available_version: Option<String>,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub last_checked_at: Option<String>,
    pub error: Option<String>,
}

impl UpdateStatus {
    fn new(version: String, enabled: bool) -> Self {
        Self {
            status: if enabled { "idle" } else { "disabled" }.into(),
            current_version: version,
            available_version: None,
            downloaded_bytes: 0,
            total_bytes: None,
            last_checked_at: None,
            error: None,
        }
    }

    fn begin_check(&mut self) {
        self.status = "checking".into();
        self.error = None;
        self.available_version = None;
        self.downloaded_bytes = 0;
        self.total_bytes = None;
    }

    fn can_check(&self, enabled: bool, busy: bool, _has_pending: bool) -> bool {
        enabled && !busy
    }

    fn fail(&mut self, error: String) {
        self.status = "error".into();
        self.error = Some(error);
    }
}

struct PendingUpdate {
    update: Update,
    bytes: Vec<u8>,
}

struct Inner {
    status: UpdateStatus,
    busy: bool,
    pending: Option<PendingUpdate>,
}

/// Clones share progress and the single pending verified package.
#[derive(Clone)]
pub struct UpdateManager {
    enabled: bool,
    inner: Arc<Mutex<Inner>>,
}

/// Kept alive from the final idle check until the installer exits the process.
/// A host can return a maintenance guard here to prevent a new save/chat/import
/// from racing with installation. Returning `None` defers the update.
pub type InstallPermit = Box<dyn Send>;
pub type AcquireInstallPermit = Arc<dyn Fn() -> Option<InstallPermit> + Send + Sync>;

impl UpdateManager {
    pub fn new(current_version: impl Into<String>, enabled: bool) -> Self {
        Self {
            enabled,
            inner: Arc::new(Mutex::new(Inner {
                status: UpdateStatus::new(current_version.into(), enabled),
                busy: false,
                pending: None,
            })),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn status(&self) -> Result<UpdateStatus, String> {
        self.inner
            .lock()
            .map(|inner| inner.status.clone())
            .map_err(|_| "O estado do atualizador está indisponível.".into())
    }

    fn emit(&self, app: &AppHandle) {
        if let Ok(status) = self.status() {
            let _ = app.emit(UPDATE_EVENT, status);
        }
    }

    fn change(&self, app: &AppHandle, change: impl FnOnce(&mut Inner)) {
        if let Ok(mut inner) = self.inner.lock() {
            change(&mut inner);
        }
        self.emit(app);
    }

    fn fail(&self, app: &AppHandle, error: String) -> String {
        self.change(app, |inner| {
            inner.busy = false;
            inner.status.fail(error.clone());
        });
        error
    }

    /// Checks the configured HTTPS manifest and downloads a newer signed
    /// package. A missing manifest or network failure is reported as an error,
    /// never as proof that the installed version is current.
    pub async fn check_and_download(&self, app: &AppHandle) -> Result<UpdateStatus, String> {
        {
            let mut inner = self
                .inner
                .lock()
                .map_err(|_| "O estado do atualizador está indisponível.".to_string())?;
            if !inner
                .status
                .can_check(self.enabled, inner.busy, inner.pending.is_some())
            {
                return Ok(inner.status.clone());
            }
            // An explicit check must fetch the current manifest even when an
            // older verified package is waiting. Otherwise the application can
            // remain pinned to that version until the process is restarted.
            inner.pending = None;
            inner.busy = true;
            inner.status.begin_check();
        }
        self.emit(app);

        let result = self.fetch(app).await;
        match result {
            Ok(pending) => {
                self.change(app, |inner| {
                    inner.busy = false;
                    inner.status.last_checked_at = Some(now());
                    match pending {
                        Some(pending) => {
                            inner.status.status = "ready".into();
                            inner.status.available_version = Some(pending.update.version.clone());
                            inner.status.downloaded_bytes = pending.bytes.len() as u64;
                            inner.pending = Some(pending);
                        }
                        None => inner.status.status = "up_to_date".into(),
                    }
                });
                self.status()
            }
            Err(error) => {
                self.change(app, |inner| inner.status.last_checked_at = Some(now()));
                Err(self.fail(app, error))
            }
        }
    }

    async fn fetch(&self, app: &AppHandle) -> Result<Option<PendingUpdate>, String> {
        let updater = app
            .updater_builder()
            .timeout(REQUEST_TIMEOUT)
            .restart_after_install(true)
            .build()
            .map_err(|error| format!("Não foi possível configurar o atualizador: {error}"))?;
        let Some(update) = updater
            .check()
            .await
            .map_err(|error| format!("Não foi possível consultar as atualizações: {error}"))?
        else {
            return Ok(None);
        };

        self.change(app, |inner| {
            inner.status.status = "downloading".into();
            inner.status.available_version = Some(update.version.clone());
        });
        // Progress updates are throttled so a fast download cannot flood IPC.
        let mut last_emitted = Instant::now();
        let bytes = update
            .download(
                |chunk, total| {
                    if let Ok(mut inner) = self.inner.lock() {
                        inner.status.downloaded_bytes =
                            inner.status.downloaded_bytes.saturating_add(chunk as u64);
                        inner.status.total_bytes = total;
                    }
                    if last_emitted.elapsed() >= Duration::from_millis(250) {
                        self.emit(app);
                        last_emitted = Instant::now();
                    }
                },
                || {},
            )
            .await
            .map_err(|error| {
                format!("Não foi possível baixar ou validar a atualização: {error}")
            })?;
        // The plugin verifies the package before `download` returns. No bytes
        // or unsigned manifest-controlled installer path are executed here.
        Ok(Some(PendingUpdate { update, bytes }))
    }

    /// Installs only with an exclusive host permit. On Windows the updater
    /// exits this process after starting NSIS and restarts the installed app.
    /// The host should allow background installation only when the window is
    /// hidden and its saves, imports, dialogs, and chat runs have finished.
    pub fn install_pending(
        &self,
        app: &AppHandle,
        acquire_permit: &AcquireInstallPermit,
    ) -> Result<UpdateStatus, String> {
        {
            let inner = self
                .inner
                .lock()
                .map_err(|_| "O estado do atualizador está indisponível.".to_string())?;
            if !self.enabled || inner.busy || inner.pending.is_none() {
                return Ok(inner.status.clone());
            }
            // Do not repeatedly spawn an installer whose launch already failed.
            // A manual retry can use `retry_install`, otherwise the next process
            // start performs a fresh signed download.
            if inner.status.status == "error" {
                return Ok(inner.status.clone());
            }
        }
        let Some(_permit) = acquire_permit() else {
            self.change(app, |inner| {
                if !inner.busy && inner.pending.is_some() && inner.status.status != "error" {
                    inner.status.status = "waiting_for_idle".into();
                }
            });
            return self.status();
        };
        let pending = {
            let mut inner = self
                .inner
                .lock()
                .map_err(|_| "O estado do atualizador está indisponível.".to_string())?;
            if inner.busy || inner.pending.is_none() {
                return Ok(inner.status.clone());
            }
            inner.busy = true;
            inner.status.status = "installing".into();
            inner.status.error = None;
            inner
                .pending
                .take()
                .expect("pending update checked under lock")
        };
        self.emit(app);
        match pending.update.install(&pending.bytes) {
            Ok(()) => {
                // Windows does not return on success. Keep the other platforms
                // consistent if the module is reused there in the future.
                app.restart();
            }
            Err(error) => {
                self.change(app, |inner| inner.pending = Some(pending));
                Err(self.fail(
                    app,
                    format!("Não foi possível instalar a atualização: {error}"),
                ))
            }
        }
    }

    pub fn retry_install(
        &self,
        app: &AppHandle,
        acquire_permit: &AcquireInstallPermit,
    ) -> Result<UpdateStatus, String> {
        self.change(app, |inner| {
            if !inner.busy && inner.pending.is_some() {
                inner.status.status = "ready".into();
                inner.status.error = None;
            }
        });
        self.install_pending(app, acquire_permit)
    }
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// NSIS installs its uninstaller alongside the application executable. Only
/// an installed Windows release participates in automatic updates; cargo run,
/// preview/example binaries, and a copied standalone exe remain untouched.
pub fn is_installed_release(executable: &Path) -> bool {
    cfg!(all(target_os = "windows", not(debug_assertions))) && is_installed_path(executable)
}

fn is_installed_path(executable: &Path) -> bool {
    let canonical_name = executable
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("IvestFII.exe"));
    canonical_name
        && executable
            .parent()
            .is_some_and(|parent| parent.join("uninstall.exe").is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_build_reports_disabled_without_a_check() {
        let manager = UpdateManager::new("0.1.0", false);
        let status = manager.status().unwrap();
        assert_eq!(status.status, "disabled");
        assert_eq!(status.last_checked_at, None);
        assert!(!manager.enabled());
    }

    #[test]
    fn progress_is_shared_and_serializes_for_frontend() {
        let manager = UpdateManager::new("0.1.0", true);
        let clone = manager.clone();
        {
            let mut inner = manager.inner.lock().unwrap();
            inner.status.available_version = Some("0.2.0".into());
            inner.status.downloaded_bytes = 120;
            inner.status.total_bytes = Some(240);
        }
        let json = serde_json::to_value(clone.status().unwrap()).unwrap();
        assert_eq!(json["currentVersion"], "0.1.0");
        assert_eq!(json["availableVersion"], "0.2.0");
        assert_eq!(json["downloadedBytes"], 120);
        assert_eq!(json["totalBytes"], 240);
        assert!(json.get("current_version").is_none());
    }

    #[test]
    fn failed_checks_are_never_reported_as_current() {
        let mut status = UpdateStatus::new("0.1.0".into(), true);
        status.begin_check();
        status.last_checked_at = Some(now());
        status.fail("Manifesto indisponível: HTTP 404".into());
        assert_eq!(status.status, "error");
        assert!(status.error.unwrap().contains("404"));
        assert!(status.last_checked_at.is_some());
        assert!(status.available_version.is_none());
    }

    #[test]
    fn retry_resets_partial_progress_and_errors() {
        let mut status = UpdateStatus::new("0.1.0".into(), true);
        status.available_version = Some("0.2.0".into());
        status.downloaded_bytes = 100;
        status.total_bytes = Some(200);
        status.fail("Assinatura inválida".into());
        status.begin_check();
        assert_eq!(status.status, "checking");
        assert_eq!(status.downloaded_bytes, 0);
        assert_eq!(status.total_bytes, None);
        assert_eq!(status.available_version, None);
        assert_eq!(status.error, None);
    }

    #[test]
    fn explicit_check_replaces_stale_pending_packages_including_ready_and_waiting() {
        let mut status = UpdateStatus::new("0.1.0".into(), true);
        status.fail("Não foi possível iniciar o instalador.".into());
        assert!(status.can_check(true, false, true));
        for state in ["ready", "waiting_for_idle"] {
            status.status = state.into();
            assert!(status.can_check(true, false, true), "{state}");
        }
    }

    #[test]
    fn failed_install_retry_still_respects_disabled_and_in_progress_guards() {
        let mut status = UpdateStatus::new("0.1.0".into(), true);
        status.fail("Falha no instalador.".into());
        assert!(!status.can_check(false, false, true));
        assert!(!status.can_check(true, true, true));
        assert!(status.can_check(true, false, false));
    }

    #[test]
    fn only_canonical_nsis_install_path_is_eligible() {
        let directory = tempfile::tempdir().unwrap();
        let installed = directory.path().join("IvestFII.exe");
        assert!(!is_installed_path(&installed));
        std::fs::write(directory.path().join("uninstall.exe"), b"fixture").unwrap();
        assert!(is_installed_path(&installed));
        assert!(!is_installed_path(
            &directory.path().join("ivestfii-preview.exe")
        ));
        assert!(!is_installed_path(&directory.path().join("other.exe")));
    }

    #[test]
    fn debug_build_cannot_replace_itself_even_in_an_install_folder() {
        if cfg!(debug_assertions) {
            let directory = tempfile::tempdir().unwrap();
            std::fs::write(directory.path().join("uninstall.exe"), b"fixture").unwrap();
            assert!(!is_installed_release(
                &directory.path().join("IvestFII.exe")
            ));
        }
    }
}
