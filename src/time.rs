//! Minimal RFC 3339 (UTC) handling so we do not need a date-time crate.

use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Format unix seconds as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn rfc3339(secs: u64) -> String {
    rfc3339_millis(secs * 1000)
}

/// Format unix milliseconds as `YYYY-MM-DDTHH:MM:SS[.mmm]Z`. The fraction is
/// omitted when it is zero, so whole-second timestamps round-trip unchanged.
pub fn rfc3339_millis(millis: u64) -> String {
    let secs = millis / 1000;
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    let fraction = match millis % 1000 {
        0 => String::new(),
        ms => format!(".{ms:03}"),
    };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}{fraction}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Parse an RFC 3339 timestamp into unix seconds.
pub fn parse_rfc3339(s: &str) -> Option<u64> {
    parse_rfc3339_millis(s).map(|ms| ms / 1000)
}

/// Parse `YYYY-MM-DDTHH:MM:SS[.frac](Z|±HH:MM)` into unix milliseconds.
pub fn parse_rfc3339_millis(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || (b[10] != b'T' && b[10] != b't' && b[10] != b' ') {
        return None;
    }
    if b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let mut rest = s.get(19..)?;
    let mut millis = 0i64;
    if let Some(frac) = rest.strip_prefix('.') {
        let digits = frac.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        // Keep millisecond precision: pad or truncate to three digits.
        millis = format!("{:0<3.3}", &frac[..digits]).parse().ok()?;
        rest = &frac[digits..];
    }
    let offset = match rest {
        "Z" | "z" => 0,
        _ => {
            let rb = rest.as_bytes();
            if rb.len() != 6 || rb[3] != b':' {
                return None;
            }
            let sign = match rb[0] {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let oh: i64 = rest.get(1..3)?.parse().ok()?;
            let om: i64 = rest.get(4..6)?.parse().ok()?;
            sign * (oh * 3600 + om * 60)
        }
    };
    let total = days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + sec - offset;
    u64::try_from(total * 1000 + millis).ok()
}

/// Compact relative age such as `now`, `5m`, `3h`, `2d`.
pub fn age(now: u64, then: u64) -> String {
    let diff = now.saturating_sub(then);
    match diff {
        0..=59 => "now".to_string(),
        60..=3599 => format!("{}m", diff / 60),
        3600..=86_399 => format!("{}h", diff / 3600),
        _ => format!("{}d", diff / 86_400),
    }
}

// Howard Hinnant's civil date algorithms.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let s = "2026-11-24T20:00:00Z";
        let secs = parse_rfc3339(s).unwrap();
        assert_eq!(secs, 1_795_550_400);
        assert_eq!(rfc3339(secs), s);
    }

    #[test]
    fn offsets_and_fractions() {
        let utc = parse_rfc3339("2026-11-24T20:00:00Z").unwrap();
        assert_eq!(parse_rfc3339("2026-11-24T15:00:00-05:00"), Some(utc));
        assert_eq!(parse_rfc3339("2026-11-24T20:00:00.123Z"), Some(utc));
    }

    #[test]
    fn millisecond_precision() {
        let base = parse_rfc3339_millis("2026-11-24T20:00:00Z").unwrap();
        assert_eq!(parse_rfc3339_millis("2026-11-24T20:00:00.123Z"), Some(base + 123));
        assert_eq!(parse_rfc3339_millis("2026-11-24T20:00:00.5Z"), Some(base + 500));
        assert_eq!(parse_rfc3339_millis("2026-11-24T20:00:00.123456789Z"), Some(base + 123));
        assert_eq!(rfc3339_millis(base + 7), "2026-11-24T20:00:00.007Z");
        assert_eq!(rfc3339_millis(base), "2026-11-24T20:00:00Z", "no fraction for whole seconds");
    }

    #[test]
    fn rejects_garbage() {
        for bad in ["", "2026-11-24", "2026-13-01T00:00:00Z", "2026-11-24T20:00:00", "x026-11-24T20:00:00Z"] {
            assert_eq!(parse_rfc3339(bad), None, "{bad}");
        }
    }

    #[test]
    fn ages() {
        assert_eq!(age(100, 90), "now");
        assert_eq!(age(1000, 100), "15m");
        assert_eq!(age(10_000, 100), "2h");
        assert_eq!(age(200_000, 100), "2d");
        assert_eq!(age(5, 100), "now");
    }
}
