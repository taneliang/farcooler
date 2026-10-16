//! A moment in this machine's local time, for a sentence the runner writes
//! into a task's record ("Held until Oct 5, 9:00 AM.").
//!
//! The runner's own zone, because the runner is the owner's machine and its
//! notes are read on it; a client showing a time it was sent as a number
//! formats it in its own zone instead.

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// `Oct 5, 9:00 AM`: Unix milliseconds in local time, on a 12-hour clock.
// The libc crate marks `time_t` deprecated on musl only, warning that it will
// follow musl 1.2's move to 64 bits. On the 64-bit targets we ship it is
// `c_long`, already 64 bits, so there is nothing to act on.
#[cfg_attr(target_env = "musl", allow(deprecated))]
pub fn moment(millis: i64) -> String {
    let seconds: libc::time_t = millis.div_euclid(1000) as libc::time_t;
    // SAFETY: `localtime_r` writes only the `tm` it is given and reads only
    // `seconds`; both outlive the call, and a zeroed `tm` is a valid one.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&seconds, &mut tm);
        tm
    };
    let hour = match tm.tm_hour % 12 {
        0 => 12,
        h => h,
    };
    let half = if tm.tm_hour < 12 { "AM" } else { "PM" };
    format!("{} {}, {hour}:{:02} {half}", MONTHS[tm.tm_mon.rem_euclid(12) as usize], tm.tm_mday, tm.tm_min)
}

/// The hour of the day, 0 to 23, in this machine's local time: what a rule
/// like "never between 22:00 and 08:00" is asked of.
#[cfg_attr(target_env = "musl", allow(deprecated))]
pub fn hour(millis: i64) -> u32 {
    let seconds: libc::time_t = millis.div_euclid(1000) as libc::time_t;
    // SAFETY: as in `moment`.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&seconds, &mut tm);
        tm
    };
    tm.tm_hour.rem_euclid(24) as u32
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
