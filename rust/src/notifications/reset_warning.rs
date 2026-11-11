use std::time::Duration;

use chrono::{DateTime, Local, Utc};

use super::NotificationManager;
use crate::core::ProviderId;
use crate::locale::{self, LocaleKey};
use crate::settings::Language;

pub const DEFAULT_SESSION_RESET_WARNING_LEAD_MINUTES: u32 = 30;

pub fn session_reset_warning_lead_minutes(minutes: u32) -> u32 {
    match minutes {
        5 | 10 | 15 | 30 | 60 => minutes,
        _ => DEFAULT_SESSION_RESET_WARNING_LEAD_MINUTES,
    }
}

/// The same localized message goes to Windows and to the configured ntfy topic.
pub struct SessionResetWarning {
    pub title: String,
    pub body: String,
}

impl SessionResetWarning {
    pub fn new(
        provider: ProviderId,
        now: DateTime<Utc>,
        reset_at: DateTime<Utc>,
        remaining_percent: Option<f64>,
        language: Language,
    ) -> Self {
        let seconds = (reset_at - now).num_seconds();
        let minutes = ((seconds + 59) / 60).max(1).to_string();
        let local_time = reset_at
            .with_timezone(&Local)
            .format("%-I:%M %p")
            .to_string();
        let provider = provider.display_name();
        let title =
            locale::format_locale(language, LocaleKey::SessionResetWarningTitle, &[provider]);
        let body = match remaining_percent.filter(|percent| percent.is_finite()) {
            Some(percent) => locale::format_locale(
                language,
                LocaleKey::SessionResetWarningBodyWithQuota,
                &[
                    provider,
                    &minutes,
                    &local_time,
                    &format!("{:.0}", percent.clamp(0.0, 100.0)),
                ],
            ),
            None => locale::format_locale(
                language,
                LocaleKey::SessionResetWarningBody,
                &[provider, &minutes, &local_time],
            ),
        };
        Self { title, body }
    }

    pub fn test(language: Language) -> Self {
        Self {
            title: locale::get_text(language, LocaleKey::SessionResetWarningTestTitle),
            body: locale::get_text(language, LocaleKey::SessionResetWarningTestBody),
        }
    }
}

impl NotificationManager {
    pub fn show_session_reset_warning(&self, warning: &SessionResetWarning) -> bool {
        self.show_toast(&warning.title, &warning.body)
    }
}

/// Accept one ntfy topic name as a single path segment.
pub fn validate_reset_warning_topic(topic: &str) -> Result<(), &'static str> {
    if topic.len() > 64
        || !topic
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("Use up to 64 letters, numbers, hyphens, or underscores for the ntfy topic.");
    }
    Ok(())
}

pub async fn send_reset_warning_ntfy(
    topic: &str,
    warning: &SessionResetWarning,
) -> Result<(), String> {
    let topic = topic.trim();
    validate_reset_warning_topic(topic)?;
    if topic.is_empty() {
        return Err("Enter an ntfy topic before sending a test.".into());
    }

    let client = crate::core::credentialed_http_client_builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| "Could not initialize the ntfy connection.")?;
    let response = client
        .post(format!("https://ntfy.sh/{topic}"))
        .header(reqwest::header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header("Title", &warning.title)
        .body(warning.body.clone())
        .send()
        .await
        .map_err(|_| "Could not reach ntfy.sh.")?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("ntfy.sh returned HTTP {}.", status.as_u16()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warning_uses_remaining_minutes_and_omits_unknown_quota() {
        let now = Utc::now();
        let reset_at = now + chrono::TimeDelta::minutes(27);
        let warning = SessionResetWarning::new(
            ProviderId::Claude,
            now,
            reset_at,
            Some(3.0),
            Language::English,
        );
        assert!(
            warning
                .body
                .starts_with("Claude session resets in 27 min (at ")
        );
        assert!(warning.body.ends_with("3% left."));
        let warning =
            SessionResetWarning::new(ProviderId::Claude, now, reset_at, None, Language::English);
        assert!(warning.body.ends_with(")."));
    }

    #[test]
    fn topic_validation_accepts_only_one_ntfy_topic_name() {
        assert!(validate_reset_warning_topic("").is_ok());
        assert!(validate_reset_warning_topic("Claude-session_123").is_ok());
        assert!(validate_reset_warning_topic("a/b").is_err());
        assert!(validate_reset_warning_topic("a?x=y").is_err());
        assert!(validate_reset_warning_topic(&"a".repeat(65)).is_err());
    }
}
