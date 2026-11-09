//! `next_run` calculation from a five-field cron in an IANA zone. On a DST change a run from the skipped hour happens right
//! after it, and a run from the repeated hour happens once (`croner` behaviour, covered by the `cron_*` tests).

use chrono::{DateTime, TimeZone, Utc};
use chrono_tz::Tz;
use croner::parser::{CronParser, Seconds, Year};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CronError {
    /// The expression failed to parse or does not have five fields.
    Pattern,
    /// The zone is not in the IANA database.
    Zone,
}

fn parser() -> CronParser {
    CronParser::builder()
        .seconds(Seconds::Disallowed)
        .year(Year::Disallowed)
        .build()
}

/// Check of the expression and zone without calculating.
pub fn cron_check(expr: &str, tz: &str) -> Result<(), CronError> {
    tz.parse::<Tz>().map_err(|_| CronError::Zone)?;
    parser().parse(expr).map_err(|_| CronError::Pattern)?;
    Ok(())
}

/// Next run strictly after `after` (unix seconds, UTC); `Ok(None)` means no more runs.
pub fn cron_next(expr: &str, tz: &str, after: i64) -> Result<Option<i64>, CronError> {
    let zone: Tz = tz.parse().map_err(|_| CronError::Zone)?;
    let cron = parser().parse(expr).map_err(|_| CronError::Pattern)?;
    let start: DateTime<Tz> = Utc
        .timestamp_opt(after, 0)
        .single()
        .ok_or(CronError::Pattern)?
        .with_timezone(&zone);
    Ok(cron
        .find_next_occurrence(&start, false)
        .ok()
        .map(|t| t.timestamp()))
}
