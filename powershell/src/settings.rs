//! The clock typed predicates read and what this build of trex has.
//!
//! The clock is trex's own, one per process: `\T{age<24h}` compares a
//! timestamp with its `now`, a timestamp written with no zone is read at its
//! offset, and a slash date is read day first or month first by its order.
//! Set here, it holds for every later scan in this PowerShell process until
//! set again.

use pwrs::prelude::*;

use crate::common::arg_err;

/// Which field of an all-numeric slash date such as `05/09/2026` is the day.
#[psenum(name = "Trex.DateOrder")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DateOrder {
    /// `05/09/2026` is the 5th of September.
    #[default]
    DayFirst,
    /// `05/09/2026` is the 9th of May.
    MonthFirst,
}

/// The clock typed predicates read.
#[psclass(name = "Trex.Clock")]
#[derive(Clone, Default)]
pub struct TrexClock {
    /// The instant `now` reads as.
    pub now: PsDateTimeOffset,
    /// Whether `now` is fixed by Set-TrexClock rather than read from the
    /// system clock at each scan.
    pub fixed: bool,
    /// The offset a timestamp written with no zone is read at.
    pub time_zone_offset: PsTimeSpan,
    /// Which field of an all-numeric slash date is the day.
    pub date_order: DateOrder,
}

/// The ticks between 0001-01-01 and the Unix epoch.
const EPOCH_TICKS: i64 = 621_355_968_000_000_000;

fn clock_now() -> TrexClock {
    let c = trex::Clock::current();
    TrexClock {
        now: PsDateTimeOffset::new(c.now_secs.saturating_mul(10_000_000).saturating_add(EPOCH_TICKS), 0),
        fixed: trex::now_override().is_some(),
        time_zone_offset: PsTimeSpan::from_ticks(i64::from(c.tz_offset) * 10_000_000),
        date_order: if trex::date_order_day_first() { DateOrder::DayFirst } else { DateOrder::MonthFirst },
    }
}

/// Writes the clock typed predicates read: the instant `now` is, whether it
/// is fixed, the offset a timestamp with no zone is read at, and the order
/// of a slash date's fields.
///
/// # Examples
/// Get-TrexClock
#[cmdlet(verb = "Get", noun = "TrexClock", alias = "Get-TxClock", output = ["Trex.Clock"])]
#[derive(Default)]
pub struct GetTrexClock {}

impl Cmdlet for GetTrexClock {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        ps.write(clock_now())
    }
}

/// Sets the clock typed predicates read, for every later scan in this
/// PowerShell process.
///
/// -Now fixes the instant `\T{age<24h}` and `\T{<now}` compare against,
/// and -SystemClock returns to reading the system clock. -TimeZoneOffset is
/// the offset a timestamp written with no zone is read at, UTC until set.
/// -DateOrder says which field of an all-numeric slash date is the day.
///
/// # Examples
/// Set-TrexClock -Now '2026-09-27T00:00:00Z'
/// Set-TrexClock -TimeZoneOffset '-04:00' -DateOrder MonthFirst
/// Set-TrexClock -SystemClock
#[cmdlet(verb = "Set", noun = "TrexClock", alias = "Set-TxClock", supports_should_process, output = ["Trex.Clock"])]
#[derive(Default)]
pub struct SetTrexClock {
    /// The instant `now` reads as from here on.
    #[param]
    pub now: Option<PsDateTimeOffset>,
    /// Returns `now` to the system clock.
    #[param]
    pub system_clock: bool,
    /// The offset a timestamp written with no zone is read at.
    #[param]
    pub time_zone_offset: Option<PsTimeSpan>,
    /// Which field of an all-numeric slash date is the day.
    #[param]
    pub date_order: Option<DateOrder>,
    /// Writes the clock as it stands after the change.
    #[param]
    pub pass_thru: bool,
}

impl Cmdlet for SetTrexClock {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        if self.now.is_some() && self.system_clock {
            return Err(arg_err("TrexClock", "-Now fixes the instant and -SystemClock releases it; give one"));
        }
        if !ps.should_process("the trex clock", "Set")? {
            return Ok(());
        }
        if let Some(now) = self.now {
            let secs = (now.to_utc_ticks() - EPOCH_TICKS).div_euclid(10_000_000);
            trex::set_now(Some(secs));
        }
        if self.system_clock {
            trex::set_now(None);
        }
        if let Some(offset) = self.time_zone_offset {
            let secs = offset.ticks / 10_000_000;
            let secs = i32::try_from(secs)
                .map_err(|e| arg_err("TrexClock", format!("-TimeZoneOffset of {secs} seconds is out of range: {e}")))?;
            trex::set_tz_offset(secs);
        }
        if let Some(order) = self.date_order {
            trex::set_date_order_day_first(order == DateOrder::DayFirst);
        }
        if self.pass_thru {
            ps.write(clock_now())?;
        }
        Ok(())
    }
}

/// What this build of trex is.
#[psclass(name = "Trex.Info")]
#[derive(Clone, Default)]
pub struct TrexInfo {
    /// The trex engine's version.
    pub version: String,
    /// Whether a CUDA device is present for the scans that can use one.
    pub device_available: bool,
}

/// Writes the trex engine's version and whether a CUDA device is present.
///
/// # Examples
/// Get-TrexInfo
#[cmdlet(verb = "Get", noun = "TrexInfo", alias = "Get-TxInfo", output = ["Trex.Info"])]
#[derive(Default)]
pub struct GetTrexInfo {}

impl Cmdlet for GetTrexInfo {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        ps.write(TrexInfo { version: trex::version().to_string(), device_available: trex::device_available() })
    }
}
