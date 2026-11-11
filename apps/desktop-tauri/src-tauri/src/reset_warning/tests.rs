use super::*;

fn time(hour: u32, minute: u32) -> DateTime<Utc> {
    use chrono::TimeZone;
    Utc.with_ymd_and_hms(2026, 5, 1, hour, minute, 0).unwrap()
}

#[test]
fn warns_at_the_lead_deadline_between_refreshes() {
    let reset_at = Some(time(15, 0));
    let deadline = time(14, 30);
    assert_eq!(
        schedule(reset_at, None, 30, deadline - TimeDelta::milliseconds(1)),
        Schedule::At(deadline)
    );
    assert_eq!(schedule(reset_at, None, 30, deadline), Schedule::Warn);
}

#[test]
fn warns_once_and_rearms_when_the_reset_advances() {
    let reset_at = Some(time(15, 0));
    assert_eq!(schedule(reset_at, None, 30, time(14, 30)), Schedule::Warn);
    assert_eq!(
        schedule(reset_at, reset_at, 30, time(14, 35)),
        Schedule::Idle
    );
    assert_eq!(
        schedule(Some(time(20, 0)), reset_at, 30, time(15, 5)),
        Schedule::At(time(19, 30))
    );
    assert_eq!(
        schedule(Some(time(20, 0)), reset_at, 30, time(19, 30)),
        Schedule::Warn
    );
}

#[test]
fn ignores_missing_expired_and_older_resets() {
    let now = time(14, 30);
    assert_eq!(schedule(None, None, 30, now), Schedule::Idle);
    assert_eq!(schedule(Some(time(14, 29)), None, 30, now), Schedule::Idle);
    assert_eq!(schedule(Some(now), None, 30, now), Schedule::Idle);
    assert_eq!(
        schedule(Some(time(14, 55)), Some(time(15, 0)), 30, now),
        Schedule::Idle
    );
}

#[test]
fn moves_the_deadline_when_the_reset_or_lead_time_shifts() {
    let now = time(14, 25);
    assert_eq!(
        schedule(Some(time(15, 0)), None, 30, now),
        Schedule::At(time(14, 30))
    );
    assert_eq!(
        schedule(Some(time(15, 10)), None, 30, time(14, 30)),
        Schedule::At(time(14, 40))
    );
    assert_eq!(
        schedule(Some(time(15, 10)), None, 15, now),
        Schedule::At(time(14, 55))
    );
    assert_eq!(schedule(Some(time(14, 50)), None, 30, now), Schedule::Warn);
}

#[test]
fn startup_inside_lead_window_consults_the_persisted_receipt() {
    let directory =
        std::env::temp_dir().join(format!("codexbar-reset-warning-{}", uuid::Uuid::new_v4()));
    let path = directory.join("receipt");
    let reset_at = time(15, 0);
    let now = time(14, 45);
    assert_eq!(
        schedule(Some(reset_at), read_receipt(&path), 30, now),
        Schedule::Warn
    );
    write_receipt(&path, reset_at).unwrap();
    assert_eq!(read_receipt(&path), Some(reset_at));
    assert_eq!(
        schedule(Some(reset_at), read_receipt(&path), 30, now),
        Schedule::Idle
    );
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(directory).unwrap();
}

#[test]
fn uses_only_claudes_real_five_hour_primary_lane_and_normalizes_to_utc() {
    let mut snapshot = ProviderUsageSnapshot {
        provider_id: "claude".into(),
        ..ProviderUsageSnapshot::default()
    };
    snapshot.primary.window_minutes = Some(300);
    snapshot.primary.resets_at = Some("2026-05-01T10:00:00-05:00".into());
    snapshot.primary.remaining_percent = 3.0;
    let window = CLAUDE_SESSION.read(&snapshot).unwrap();
    assert_eq!(window.reset_at, time(15, 0));
    assert_eq!(window.remaining_percent, Some(3.0));

    snapshot.primary.resets_at = Some("sometime today".into());
    assert!(CLAUDE_SESSION.read(&snapshot).is_none());
    snapshot.primary.resets_at = Some("2026-05-01T15:00:00Z".into());
    snapshot.primary.is_informational = true;
    assert!(CLAUDE_SESSION.read(&snapshot).is_none());
    snapshot.primary.is_informational = false;
    snapshot.primary.window_minutes = Some(10080);
    assert!(CLAUDE_SESSION.read(&snapshot).is_none());
    snapshot.primary.window_minutes = Some(300);
    snapshot.provider_id = "codex".into();
    assert!(CLAUDE_SESSION.read(&snapshot).is_none());
}
