use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, TimeDelta, Utc};
use codexbar::core::ProviderId;
use codexbar::notifications::{SessionResetWarning, send_reset_warning_ntfy};
use codexbar::{secure_file, settings::Settings};
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Manager};

use crate::commands::ProviderUsageSnapshot;
use crate::state::AppState;

struct WarningTarget {
    provider: ProviderId,
    session_minutes: u32,
}

const CLAUDE_SESSION: WarningTarget = WarningTarget {
    provider: ProviderId::Claude,
    session_minutes: 300,
};

struct Window {
    reset_at: DateTime<Utc>,
    remaining_percent: Option<f64>,
}

impl WarningTarget {
    fn read(&self, snapshot: &ProviderUsageSnapshot) -> Option<Window> {
        let lane = &snapshot.primary;
        if snapshot.provider_id != self.provider.cli_name()
            || snapshot.error.is_some()
            || lane.is_informational
            || lane.window_minutes != Some(self.session_minutes)
        {
            return None;
        }
        let reset_at = DateTime::parse_from_rfc3339(lane.resets_at.as_deref()?)
            .ok()?
            .with_timezone(&Utc);
        Some(Window {
            reset_at,
            remaining_percent: lane
                .remaining_percent
                .is_finite()
                .then_some(lane.remaining_percent),
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Schedule {
    Idle,
    At(DateTime<Utc>),
    Warn,
}

fn schedule(
    reset_at: Option<DateTime<Utc>>,
    last_notified: Option<DateTime<Utc>>,
    lead_minutes: u32,
    now: DateTime<Utc>,
) -> Schedule {
    let Some(reset_at) =
        reset_at.filter(|reset| *reset > now && last_notified.is_none_or(|last| *reset > last))
    else {
        return Schedule::Idle;
    };
    let deadline = reset_at - TimeDelta::minutes(i64::from(lead_minutes));
    if deadline > now {
        Schedule::At(deadline)
    } else {
        Schedule::Warn
    }
}

struct PendingWarning {
    deadline: DateTime<Utc>,
    task: JoinHandle<()>,
}

pub(crate) struct ResetWarningService {
    receipt_path: Option<PathBuf>,
    last_notified: Option<DateTime<Utc>>,
    pending: Option<PendingWarning>,
    clock: fn() -> DateTime<Utc>,
}

impl ResetWarningService {
    pub(crate) fn load() -> Self {
        let receipt_path = Settings::settings_path().and_then(|path| {
            Some(
                path.parent()?
                    .join("notifications/claude-session-warning-v1"),
            )
        });
        let last_notified = receipt_path.as_deref().and_then(read_receipt);
        Self {
            receipt_path,
            last_notified,
            pending: None,
            clock: Utc::now,
        }
    }

    fn cancel_pending(&mut self) {
        if let Some(pending) = self.pending.take() {
            pending.task.abort();
        }
    }

    fn update(&mut self, app: &AppHandle) {
        let settings = Settings::load();
        let window = cached_window(app, &settings);
        let now = (self.clock)();
        let decision = schedule(
            window.as_ref().map(|window| window.reset_at),
            self.last_notified,
            settings.session_reset_warning_lead_minutes,
            now,
        );
        if let Schedule::At(deadline) = decision
            && self
                .pending
                .as_ref()
                .is_some_and(|pending| pending.deadline == deadline)
        {
            return;
        }
        self.cancel_pending();
        match decision {
            Schedule::Idle => {}
            Schedule::At(deadline) => {
                let delay = (deadline - now).to_std().unwrap_or_default();
                let wake_at = tokio::time::Instant::now() + delay;
                let app = app.clone();
                let task = tauri::async_runtime::spawn(async move {
                    tokio::time::sleep_until(wake_at).await;
                    timer_elapsed(&app, deadline);
                });
                self.pending = Some(PendingWarning { deadline, task });
            }
            Schedule::Warn => {
                let Some(window) = window else { return };
                if self.record_warning(window.reset_at) {
                    dispatch(app, &settings, now, window);
                }
            }
        }
    }

    fn record_warning(&mut self, reset_at: DateTime<Utc>) -> bool {
        // Commit the receipt while holding the service lock, before handing off
        // either channel. A refresh racing the timer sees the same receipt.
        if let Some(path) = &self.receipt_path
            && let Err(error) = write_receipt(path, reset_at)
        {
            tracing::warn!(%error, "could not save session reset warning receipt");
            return false;
        }
        self.last_notified = Some(reset_at);
        true
    }
}

fn write_receipt(path: &Path, reset_at: DateTime<Utc>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    secure_file::write_string(path, &reset_at.to_rfc3339())
}

fn read_receipt(path: &Path) -> Option<DateTime<Utc>> {
    match secure_file::read_string(path) {
        Ok(receipt) => match DateTime::parse_from_rfc3339(receipt.trim()) {
            Ok(reset) => Some(reset.with_timezone(&Utc)),
            Err(error) => {
                tracing::warn!(%error, "invalid session reset warning receipt");
                None
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            tracing::warn!(%error, "could not read session reset warning receipt");
            None
        }
    }
}

fn cached_window(app: &AppHandle, settings: &Settings) -> Option<Window> {
    if !settings.session_reset_warning_enabled
        || !settings
            .get_enabled_provider_ids()
            .contains(&CLAUDE_SESSION.provider)
    {
        return None;
    }
    let state = app.state::<Mutex<AppState>>();
    let state = state.lock().ok()?;
    if state.seeded_provider_ids.contains(&CLAUDE_SESSION.provider) {
        return None;
    }
    state
        .provider_cache
        .iter()
        .find_map(|snapshot| CLAUDE_SESSION.read(snapshot))
}

pub(crate) fn provider_refreshed(app: &AppHandle, provider: ProviderId) {
    if provider == CLAUDE_SESSION.provider {
        reconcile(app);
    }
}

pub(crate) fn reconcile(app: &AppHandle) {
    if let Ok(mut service) = app.state::<Mutex<ResetWarningService>>().lock() {
        service.update(app);
    }
}

fn timer_elapsed(app: &AppHandle, deadline: DateTime<Utc>) {
    if let Ok(mut service) = app.state::<Mutex<ResetWarningService>>().lock()
        && service
            .pending
            .as_ref()
            .is_some_and(|pending| pending.deadline == deadline)
    {
        service.pending = None;
        service.update(app);
    }
}

fn dispatch(app: &AppHandle, settings: &Settings, now: DateTime<Utc>, window: Window) {
    let warning = SessionResetWarning::new(
        CLAUDE_SESSION.provider,
        now,
        window.reset_at,
        window.remaining_percent,
        settings.ui_language,
    );
    if let Err(error) = show_toast(app, &warning) {
        tracing::warn!(%error, "session reset warning toast failed");
    }
    let topic = settings.session_reset_warning_ntfy_topic.trim().to_string();
    if !topic.is_empty() {
        tauri::async_runtime::spawn(async move {
            if let Err(error) = send_reset_warning_ntfy(&topic, &warning).await {
                tracing::warn!(%error, "session reset warning push failed");
            }
        });
    }
}

fn show_toast(app: &AppHandle, warning: &SessionResetWarning) -> Result<(), String> {
    let state = app.state::<Mutex<AppState>>();
    let state = state.lock().map_err(|error| error.to_string())?;
    if state
        .notification_manager
        .show_session_reset_warning(warning)
    {
        Ok(())
    } else {
        Err("Could not dispatch the desktop toast.".into())
    }
}

#[tauri::command]
pub(crate) async fn test_session_reset_warning(
    app: AppHandle,
    topic: Option<String>,
) -> Result<(), String> {
    let warning = SessionResetWarning::test(Settings::load().ui_language);
    match topic {
        Some(topic) => send_reset_warning_ntfy(&topic, &warning).await,
        None => show_toast(&app, &warning),
    }
}

#[cfg(test)]
mod tests;
