//! A moment in this machine's local time, for a sentence the runner writes
//! into a task's record ("Held until Oct 5, 9:00 AM.").
//!
//! The runner's own zone, because the runner is the owner's machine and its
//! notes are read on it; a client showing a time it was sent as a number
//! formats it in its own zone instead.

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// `Oct 5, 9:00 AM`: Unix milliseconds in local time, on a 12-hour clock.
pub fn moment(millis: i64) -> String {
    let tm = broken_down(millis);
    let hour = match tm.hour % 12 {
        0 => 12,
        h => h,
    };
    let half = if tm.hour < 12 { "AM" } else { "PM" };
    format!("{} {}, {hour}:{:02} {half}", MONTHS[tm.month0.rem_euclid(12) as usize], tm.day, tm.minute)
}

/// The hour of the day, 0 to 23, in this machine's local time: what a rule
/// like "never between 22:00 and 08:00" is asked of.
pub fn hour(millis: i64) -> u32 {
    broken_down(millis).hour.rem_euclid(24) as u32
}

/// The fields `moment` and `hour` read, in this machine's zone.
struct Fields {
    /// 0 for January.
    month0: i32,
    day: i32,
    hour: i32,
    minute: i32,
}

// The libc crate marks `time_t` deprecated on musl only, warning that it will
// follow musl 1.2's move to 64 bits. On the 64-bit targets we ship it is
// `c_long`, already 64 bits, so there is nothing to act on.
#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(target_env = "musl", allow(deprecated))]
fn broken_down(millis: i64) -> Fields {
    let seconds: libc::time_t = millis.div_euclid(1000) as libc::time_t;
    // SAFETY: `localtime_r` writes only the `tm` it is given and reads only
    // `seconds`; both outlive the call, and a zeroed `tm` is a valid one.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&seconds, &mut tm);
        tm
    };
    Fields { month0: tm.tm_mon, day: tm.tm_mday, hour: tm.tm_hour, minute: tm.tm_min }
}

/// A browser (ov-768) has no `localtime_r` and no zone database this crate
/// can read, so it reads UTC. Nothing a browser runs formats a runner's note
/// today; this exists so the crate builds there, and says UTC rather than
/// guessing a zone.
#[cfg(target_arch = "wasm32")]
fn broken_down(millis: i64) -> Fields {
    let seconds = millis.div_euclid(1000);
    let (days, of_day) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    // Howard Hinnant's days-to-civil, from 1970-01-01.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as i32;
    let month0 = if mp < 10 { mp + 2 } else { mp - 10 } as i32;
    Fields { month0, day, hour: (of_day / 3600) as i32, minute: (of_day % 3600 / 60) as i32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hour_is_a_clock_hour_that_moves_with_the_time() {
        let t = 1_759_654_800_000;
        assert!(hour(t) < 24);
        // An hour later is the next hour on the clock, in any zone without a change in between.
        assert_eq!((hour(t) + 1) % 24, hour(t + 3_600_000));
    }

    /// The shape, in whatever zone the test runs in: a month, a day, and a
    /// 12-hour time with its half.
    #[test]
    fn a_moment_reads_as_month_day_and_time() {
        let said = moment(1_759_654_800_000);
        let (date, time) = said.split_once(", ").expect(&said);
        let (month, day) = date.split_once(' ').expect(&said);
        assert!(MONTHS.contains(&month), "{said}");
        assert!((1..=31).contains(&day.parse::<u32>().unwrap()), "{said}");
        assert!(time.ends_with(" AM") || time.ends_with(" PM"), "{said}");
        let (h, m) = time[..time.len() - 3].split_once(':').expect(&said);
        assert!((1..=12).contains(&h.parse::<u32>().unwrap()) && m.len() == 2, "{said}");
    }
}
