use crate::domain::{CalendarException, CalendarProfile, DayAffinity, DayKind, ScheduleEvaluation};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Datelike, Duration, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;

pub fn validate_profile(timezone: &str, weekly_pattern: &str) -> Result<()> {
    timezone
        .parse::<Tz>()
        .with_context(|| format!("unknown IANA timezone: {timezone}"))?;
    let bytes = weekly_pattern.as_bytes();
    if bytes.len() != 7 || !bytes.iter().all(|value| matches!(value, b'W' | b'O')) {
        bail!("weekly pattern must contain exactly seven W/O characters in Monday-Sunday order");
    }
    Ok(())
}

pub fn evaluate(
    profile: &CalendarProfile,
    exceptions: &[CalendarException],
    affinity: DayAffinity,
    now: DateTime<Utc>,
) -> Result<ScheduleEvaluation> {
    validate_profile(&profile.timezone, &profile.weekly_pattern)?;
    let timezone: Tz = profile.timezone.parse()?;
    let local_date = now.with_timezone(&timezone).date_naive();
    let (day_kind, day_source) = resolve_day(profile, exceptions, local_date)?;
    let eligible = affinity_matches(affinity, day_kind);
    let reason_code = match (eligible, affinity, day_kind) {
        (true, DayAffinity::Both, _) => "schedule.eligible_both",
        (true, DayAffinity::Work, DayKind::Work) => "schedule.eligible_workday",
        (true, DayAffinity::Off, DayKind::Off) => "schedule.eligible_off_day",
        (false, DayAffinity::Work, DayKind::Off) => "schedule.ineligible_off_day",
        (false, DayAffinity::Off, DayKind::Work) => "schedule.ineligible_workday",
        _ => unreachable!("all affinity and day combinations are covered"),
    }
    .to_owned();
    let next_eligible_at = if eligible {
        None
    } else {
        next_eligible(profile, exceptions, affinity, timezone, local_date)?
    };
    Ok(ScheduleEvaluation {
        profile_id: profile.id.clone(),
        profile_version: profile.version,
        timezone: profile.timezone.clone(),
        evaluated_at: now,
        local_date,
        day_kind,
        day_source,
        affinity,
        eligible,
        reason_code,
        next_eligible_at,
    })
}

fn resolve_day(
    profile: &CalendarProfile,
    exceptions: &[CalendarException],
    local_date: NaiveDate,
) -> Result<(DayKind, String)> {
    if let Some(exception) = exceptions
        .iter()
        .find(|exception| exception.local_date == local_date)
    {
        return Ok((
            exception.day_kind,
            format!("exception:{}", exception.reason),
        ));
    }
    let index = local_date.weekday().num_days_from_monday() as usize;
    let day_kind = match profile.weekly_pattern.as_bytes()[index] {
        b'W' => DayKind::Work,
        b'O' => DayKind::Off,
        _ => bail!("weekly pattern contains an invalid day kind"),
    };
    Ok((day_kind, format!("weekly_pattern:{index}")))
}

fn affinity_matches(affinity: DayAffinity, day_kind: DayKind) -> bool {
    matches!(
        (affinity, day_kind),
        (DayAffinity::Both, _)
            | (DayAffinity::Work, DayKind::Work)
            | (DayAffinity::Off, DayKind::Off)
    )
}

fn next_eligible(
    profile: &CalendarProfile,
    exceptions: &[CalendarException],
    affinity: DayAffinity,
    timezone: Tz,
    local_date: NaiveDate,
) -> Result<Option<DateTime<Utc>>> {
    for days in 1..=370 {
        let candidate = local_date
            .checked_add_signed(Duration::days(days))
            .context("calendar search exceeded supported date range")?;
        let (day_kind, _) = resolve_day(profile, exceptions, candidate)?;
        if affinity_matches(affinity, day_kind) {
            return Ok(Some(local_day_start_utc(timezone, candidate)?));
        }
    }
    Ok(None)
}

fn local_day_start_utc(timezone: Tz, date: NaiveDate) -> Result<DateTime<Utc>> {
    let midnight = date
        .and_hms_opt(0, 0, 0)
        .context("constructing local midnight")?;
    for minutes in 0..=180 {
        let candidate: NaiveDateTime = midnight + Duration::minutes(minutes);
        match timezone.from_local_datetime(&candidate) {
            LocalResult::Single(value) => return Ok(value.with_timezone(&Utc)),
            LocalResult::Ambiguous(first, second) => {
                return Ok(first.min(second).with_timezone(&Utc));
            }
            LocalResult::None => {}
        }
    }
    bail!("could not resolve the start of local date {date} in {timezone}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(pattern: &str) -> CalendarProfile {
        CalendarProfile {
            id: "calendar-default".into(),
            slug: "default".into(),
            timezone: "Europe/London".into(),
            weekly_pattern: pattern.into(),
            version: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn all_weekdays_and_affinities_follow_work_off_both_contract() {
        let calendar = profile("WWWWWOO");
        let monday = Utc.with_ymd_and_hms(2026, 7, 20, 12, 0, 0).unwrap();
        let mut assertions = 0;
        for offset in 0..7 {
            let now = monday + Duration::days(offset);
            let expected_kind = if offset < 5 {
                DayKind::Work
            } else {
                DayKind::Off
            };
            for affinity in [DayAffinity::Work, DayAffinity::Off, DayAffinity::Both] {
                let result = evaluate(&calendar, &[], affinity, now).unwrap();
                assert_eq!(result.day_kind, expected_kind);
                assert_eq!(
                    result.eligible,
                    affinity == DayAffinity::Both
                        || (affinity == DayAffinity::Work && expected_kind == DayKind::Work)
                        || (affinity == DayAffinity::Off && expected_kind == DayKind::Off)
                );
                assertions += 1;
            }
        }
        assert_eq!(assertions, 21);
    }

    #[test]
    fn exception_and_dst_use_local_date_and_correct_next_midnight() {
        let calendar = profile("WWWWWOO");
        let dst_sunday = Utc.with_ymd_and_hms(2026, 3, 29, 10, 0, 0).unwrap();
        let normal = evaluate(&calendar, &[], DayAffinity::Work, dst_sunday).unwrap();
        assert!(!normal.eligible);
        assert_eq!(
            normal.next_eligible_at,
            Some(Utc.with_ymd_and_hms(2026, 3, 29, 23, 0, 0).unwrap())
        );
        let exception = CalendarException {
            profile_id: calendar.id.clone(),
            local_date: NaiveDate::from_ymd_opt(2026, 3, 29).unwrap(),
            day_kind: DayKind::Work,
            reason: "release day".into(),
            created_at: Utc::now(),
        };
        let overridden = evaluate(&calendar, &[exception], DayAffinity::Work, dst_sunday).unwrap();
        assert!(overridden.eligible);
        assert_eq!(overridden.day_source, "exception:release day");
    }

    #[test]
    fn invalid_timezone_and_pattern_fail_closed() {
        assert!(validate_profile("Mars/Olympus", "WWWWWOO").is_err());
        assert!(validate_profile("Etc/UTC", "WWWWWBQ").is_err());
        assert!(validate_profile("Etc/UTC", "WWWWW").is_err());
    }
}
