//! The activity trace: thirteen buckets of what a terminal has been doing.
//!
//! Two series and a set of commit marks, per terminal, over a window that snaps
//! to roughly an hour, six hours or a day. Up is code touched, down is output to
//! the person, commits sit on the axis. The drawing is specified in the glance
//! spec's §04; this module is the arithmetic behind it and nothing else — it
//! holds no clock, opens no file and knows nothing about tmux or git.
//!
//! # The one property that must never be lost
//!
//! **Bucket boundaries are absolute wall-clock, never relative to now.**
//!
//! The whole point is that two polls landing inside the same bucket produce
//! byte-identical output. Clients compare snapshots to decide whether anything
//! is worth sending on — `agentsSayTheSame(as:)` in AgentKit exists so a watch
//! is not handed a Bluetooth write every three seconds — and a window measured
//! backwards from "now" shifts on every single poll. Every bar would move a
//! fraction of a bucket to the left, every byte would differ, and that guard
//! would never fire again for as long as the fleet ran.
//!
//! So a bucket is `[k*width, (k+1)*width)` in Unix seconds, for integer `k`.
//! `width` divides 86400, so buckets also land on wall-clock minutes and hours.
//! The array changes when a boundary is crossed, or when a count inside the
//! newest bucket moves. Never otherwise.
//!
//! If you are here to make the window "smoother", or to slide it so the newest
//! bar is always exactly now: that change is the bug. It is invisible in every
//! screenshot and it costs a battery.
//!
//! # Why the window is 13 x 5min / 30min / 2h and not exactly 1h / 6h / 24h
//!
//! The spec asks for thirteen buckets over a window snapping to 1h, 6h or 24h.
//! 3600 does not divide by 13, so an exactly-one-hour window of thirteen
//! buckets cannot be aligned to anything — which is the property above, and it
//! outranks the round number. The widths chosen are the nearest round ones that
//! divide a day: 13 x 5min = 65min, 13 x 30min = 6h30m, 13 x 2h = 26h. Clients
//! still print `1h`, `6h` and `24h` beside the trace, because that is what the
//! span means to a person, and nobody can see the extra five minutes on a bar
//! 12pt wide.

/// Buckets in a trace. Fixed at every size, per the spec: bars are flex, the
/// count is not.
pub const BUCKETS: usize = 13;

/// The finest bucket the daemon stores, in seconds.
///
/// Everything else is a whole multiple of this, so a coarser window is summed
/// out of these rather than sampled on its own clock — one ring, three views.
pub const BASE_WIDTH: i64 = 300;

/// The three window widths, in seconds per bucket, shortest first.
///
/// Each divides 86400, so every boundary is a wall-clock time. Each is a whole
/// multiple of `BASE_WIDTH`, so every one is summable out of the same ring.
pub const WIDTHS: [i64; 3] = [300, 1800, 7200];

/// Slots in the ring: enough base buckets to fill the widest window.
///
/// `13 * (7200 / 300)`. At 12 bytes a slot this is 3.7KB per terminal, which is
/// the daemon's memory and not a phone's — the wire carries the 66-byte
/// rendering below, never this.
pub const SLOTS: usize = BUCKETS * (7200 / 300) as usize;

/// The wire encoding's version, in the high nibble of byte 0.
///
/// A client that does not know a version must draw nothing rather than
/// misread the bytes as heights. The low nibble is the width code.
pub const ENCODING_VERSION: u8 = 1;

/// Bytes in one encoded trace. See `Trace::encode`.
pub const ENCODED_LEN: usize = 1 + BUCKETS * 2 + BUCKETS * 2 + BUCKETS;

/// What one bucket accumulated.
///
/// `u32` because this is the daemon's copy and nothing here has to be small;
/// the wire saturates these into `u16` and `u8`, which is where the size
/// argument lives.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sample {
    /// Lines the terminal put in front of the person. The lower half.
    pub output: u32,
    /// Lines of code touched. The upper half.
    pub code: u32,
    /// Commits landed. Marks on the axis, not a bar.
    pub commits: u32,
}

impl Sample {
    pub fn output(lines: u32) -> Self {
        Self { output: lines, ..Self::default() }
    }

    pub fn code(lines: u32) -> Self {
        Self { code: lines, ..Self::default() }
    }

    pub fn commits(count: u32) -> Self {
        Self { commits: count, ..Self::default() }
    }

    fn is_empty(&self) -> bool {
        self.output == 0 && self.code == 0 && self.commits == 0
    }

    fn add(&mut self, other: Sample) {
        self.output = self.output.saturating_add(other.output);
        self.code = self.code.saturating_add(other.code);
        self.commits = self.commits.saturating_add(other.commits);
    }
}

/// One terminal's history, as a ring of absolute five-minute slots.
///
/// Indexed by `slot mod SLOTS`, where `slot` is `unix_seconds / BASE_WIDTH` —
/// so a slot's identity is a wall-clock fact and not a position in a queue.
/// That is what makes advancing the ring free: nothing is copied, the slots a
/// gap skipped are simply cleared as they are reached.
#[derive(Debug, Clone)]
pub struct Trace {
    slots: [Sample; SLOTS],
    /// The newest slot index this ring has been advanced to. `None` until the
    /// first record — an untouched trace has no "now" of its own and must not
    /// invent one, or a daemon that just started would report a wall of zeroes
    /// as observed silence.
    newest: Option<i64>,
}

impl Default for Trace {
    fn default() -> Self {
        Self { slots: [Sample::default(); SLOTS], newest: None }
    }
}

impl Trace {
    pub fn new() -> Self {
        Self::default()
    }

    /// The absolute base-slot a moment falls in.
    fn slot_of(now: i64) -> i64 {
        now.div_euclid(BASE_WIDTH)
    }

    /// Move the ring forward to `now`, clearing everything the gap skipped.
    ///
    /// Clearing rather than leaving stale counts is the whole correctness of a
    /// ring: slot `k mod SLOTS` was last written 26 hours ago, and reading it
    /// as if it were this hour's would show a burst of activity that happened
    /// yesterday.
    fn advance_to(&mut self, slot: i64) {
        let Some(newest) = self.newest else {
            self.newest = Some(slot);
            self.slots[slot.rem_euclid(SLOTS as i64) as usize] = Sample::default();
            return;
        };
        if slot <= newest {
            return;
        }
        // A gap wider than the ring means every slot is stale; clearing them
        // one at a time would be up to `slot - newest` iterations, which for a
        // laptop that was closed for a week is tens of thousands.
        let stale = (slot - newest).min(SLOTS as i64);
        for step in 1..=stale {
            let index = (newest + step).rem_euclid(SLOTS as i64) as usize;
            self.slots[index] = Sample::default();
        }
        self.newest = Some(slot);
    }

    /// Add to the bucket `now` falls in.
    ///
    /// A sample that arrives late — a probe that took longer than a tick — is
    /// added to the bucket it belongs to, not to the newest one, as long as it
    /// is still inside the ring. Later than that it is dropped: there is
    /// nowhere honest to put it, and putting it in the oldest live bucket
    /// would draw yesterday's work as this morning's.
    pub fn record(&mut self, now: i64, sample: Sample) {
        if sample.is_empty() {
            // Still advance: a quiet minute is a real fact about the ring, and
            // failing to advance would leave the gap uncleared until the next
            // busy tick, which is when it would be read.
            self.advance_to(Self::slot_of(now));
            return;
        }
        let slot = Self::slot_of(now);
        self.advance_to(slot);
        let newest = self.newest.unwrap_or(slot);
        if slot > newest || newest - slot >= SLOTS as i64 {
            return;
        }
        self.slots[slot.rem_euclid(SLOTS as i64) as usize].add(sample);
    }

    /// Move the ring forward with nothing to add. See `record`.
    pub fn tick(&mut self, now: i64) {
        self.advance_to(Self::slot_of(now));
    }

    /// Add another terminal's ring into this one, slot for slot.
    ///
    /// This is what makes the fleet trace possible, and it is only legitimate
    /// because slots are ABSOLUTE: slot `k` means the same five minutes of the
    /// same afternoon in every ring on the runner, so adding them is adding
    /// like to like. Two rings indexed from their own start would be added out
    /// of phase by however long apart the terminals were created.
    ///
    /// Summing within a channel keeps its units — the spec's own justification
    /// for the fleet trace. Nothing here ever adds the two channels together.
    pub fn absorb(&mut self, other: &Trace, now: i64) {
        let Some(other_newest) = other.newest else { return };
        self.advance_to(Self::slot_of(now));
        let Some(newest) = self.newest else { return };
        for back in 0..SLOTS as i64 {
            let slot = other_newest - back;
            if slot > newest || newest - slot >= SLOTS as i64 {
                continue;
            }
            let index = slot.rem_euclid(SLOTS as i64) as usize;
            let sample = other.slots[index];
            if sample.is_empty() {
                continue;
            }
            self.slots[index].add(sample);
        }
    }

    /// The base slot of the oldest non-empty bucket still in the ring.
    ///
    /// Bounded by `self.newest` on BOTH sides, not just by the ring's length,
    /// and that is not belt and braces. Slots between `newest` and `now` have
    /// not been cleared yet — clearing is `advance_to`'s, and it runs when
    /// something is recorded or ticked, not when something is read. Every one
    /// of those uncleared slots aliases onto a slot from the previous time
    /// round the ring, so a trace read long after it was last written would
    /// find yesterday's counts sitting in tomorrow's slots and snap its window
    /// to activity that has not happened.
    ///
    /// In the daemon the sampling loop ticks every ring once a second, so
    /// `newest` is never far behind — but a read is not allowed to depend on a
    /// writer having run recently, and this is the one place that could.
    fn oldest_activity(&self, now: i64) -> Option<i64> {
        let newest = self.newest?;
        let now_slot = Self::slot_of(now);
        (0..SLOTS as i64).rev().map(|back| now_slot - back).find(|slot| {
            *slot <= newest
                && newest - *slot < SLOTS as i64
                && !self.slots[slot.rem_euclid(SLOTS as i64) as usize].is_empty()
        })
    }

    /// Which of `WIDTHS` this trace should be drawn at.
    ///
    /// "The shortest window containing the activity", per the spec. Monotone in
    /// the age of the oldest activity, so it cannot oscillate between two
    /// widths while nothing is happening — which would change the array on
    /// alternate polls and cost exactly what wall-clock alignment buys.
    fn width(&self, now: i64) -> Option<i64> {
        let oldest = self.oldest_activity(now)?;
        let newest = Self::slot_of(now);
        let span = (newest - oldest + 1) * BASE_WIDTH;
        Some(*WIDTHS.iter().find(|w| span <= *w * BUCKETS as i64).unwrap_or(&WIDTHS[2]))
    }

    /// The thirteen buckets, oldest first, at the given width.
    ///
    /// The newest bucket is the one `now` is in, and its left edge is
    /// `(now / width) * width` — an absolute wall-clock second. Everything in
    /// this module exists to make that sentence true; see the module header.
    pub fn buckets(&self, now: i64, width: i64) -> [Sample; BUCKETS] {
        let per = (width / BASE_WIDTH).max(1);
        let newest_bucket = now.div_euclid(width);
        let mut out = [Sample::default(); BUCKETS];
        for (position, bucket) in out.iter_mut().enumerate() {
            // Oldest first: position 0 is twelve buckets back.
            let index = newest_bucket - (BUCKETS as i64 - 1 - position as i64);
            let first = index * per;
            for offset in 0..per {
                let slot = first + offset;
                // Outside the ring is not zero, it is unknown — but a bucket
                // the ring never covered is also a bucket nothing was recorded
                // into, so zero is the honest reading either way.
                if let Some(newest) = self.newest {
                    if slot > newest || newest - slot >= SLOTS as i64 {
                        continue;
                    }
                } else {
                    continue;
                }
                bucket.add(self.slots[slot.rem_euclid(SLOTS as i64) as usize]);
            }
        }
        out
    }

    /// The trace as the wire carries it: 66 fixed bytes, or none at all.
    ///
    /// # Layout
    ///
    /// ```text
    ///  0        version << 4 | width code (0 = 5min, 1 = 30min, 2 = 2h)
    ///  1..27    13 x u16 little-endian, code lines, oldest first
    /// 27..53    13 x u16 little-endian, output lines, oldest first
    /// 53..66    13 x u8, commits, oldest first
    /// ```
    ///
    /// # Why fixed-width bytes and not `repeated uint32`
    ///
    /// The consumer that decides this is the iOS widget: `FleetWidget` holds a
    /// whole fleet snapshot per timeline entry, up to 13 entries, in an
    /// extension with a hard memory ceiling. So the arithmetic is
    /// `buckets x series x agents x entries`, and it is the DECODED size that
    /// is held, not the wire size.
    ///
    /// For a plausible fleet of 12 agents, in bytes rather than in rounded
    /// units, because the point of the table is the ratio:
    ///
    /// | encoding | per agent | x12 agents | x13 entries | allocations |
    /// | --- | --- | --- | --- | --- |
    /// | three `[UInt32]` of 13 | 3 x 96 = 288 | 3,456 | 44,928 | 468 |
    /// | one `Data` of 66 | 112 | 1,344 | 17,472 | 156 |
    ///
    /// A Swift array of 13 `UInt32` is a heap buffer of a 32-byte header plus 52
    /// bytes of payload, which malloc rounds to 96; a `Data` of 66 bytes is one
    /// buffer rounded to 112, since 66 is far past the 14 bytes `Data` keeps
    /// inline. So 2.6x the bytes and 3x the allocations — and the allocation
    /// count is the half that actually hurts an extension, 468 small heap
    /// objects churned on every timeline rebuild against 156. Three `repeated`
    /// fields also give three separate backing stores per agent, so a fleet's
    /// traces end up scattered across the heap rather than in twelve
    /// contiguous runs.
    ///
    /// `u16` for the two bar series: a bucket busier than 65535 lines saturates,
    /// which is invisible, because the bars are scaled per row per half and a
    /// saturated bucket is the tallest one either way. `u8` for commits, which
    /// is a mark and not a bar.
    ///
    /// # Empty is empty
    ///
    /// A trace with nothing in it encodes to no bytes at all rather than 66
    /// zeroes. A fleet at rest then costs nothing on the wire, and two polls of
    /// a quiet terminal are equal in the cheapest possible way.
    pub fn encode(&self, now: i64) -> Vec<u8> {
        let Some(width) = self.width(now) else { return Vec::new() };
        let code = WIDTHS.iter().position(|w| *w == width).unwrap_or(0) as u8;
        let buckets = self.buckets(now, width);

        let mut out = Vec::with_capacity(ENCODED_LEN);
        out.push((ENCODING_VERSION << 4) | code);
        for bucket in &buckets {
            out.extend_from_slice(&(bucket.code.min(u16::MAX as u32) as u16).to_le_bytes());
        }
        for bucket in &buckets {
            out.extend_from_slice(&(bucket.output.min(u16::MAX as u32) as u16).to_le_bytes());
        }
        for bucket in &buckets {
            out.push(bucket.commits.min(u8::MAX as u32) as u8);
        }
        out
    }
}

/// A screen reduced to what is needed to tell whether it scrolled.
///
/// One hash per visible row, plus where the last non-blank row is. Nothing else
/// survives, which is the point: this is held per terminal between ticks and a
/// pane's worth of text is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenShape {
    rows: Vec<u64>,
    /// Index of the last row with anything on it, if any.
    bottom: Option<usize>,
}

impl ScreenShape {
    /// Reduce a `capture-pane` result to its shape.
    pub fn of(screen: &str) -> Self {
        use std::hash::{Hash, Hasher};
        let mut rows = Vec::new();
        let mut bottom = None;
        for (index, line) in screen.lines().enumerate() {
            if !line.trim().is_empty() {
                bottom = Some(index);
            }
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            line.hash(&mut hasher);
            rows.push(hasher.finish());
        }
        Self { rows, bottom }
    }

    pub fn rows(&self) -> usize {
        self.rows.len()
    }
}

/// How many lines the pane put on screen between two captures.
///
/// # Why this and not the raw pty bytes
///
/// Counting `\n` in the byte stream is the obvious answer and it is not
/// available. The bytes reach a Rust process exactly once, in `fanout::serve_on`
/// — a separate `farcoolerd --fanout` process that tmux starts when a client
/// asks to watch a pane and that exits five seconds after the last one leaves.
/// Nothing pipes a pane that nobody is looking at, deliberately: the fanout's
/// own header says every byte written into a pipe with no reader is a cost paid
/// for nothing. So a newline counter there would count only the minutes somebody
/// happened to be watching, and the activity trace exists precisely to describe
/// the hours they were not.
///
/// tmux's own counters do not help either. `#{history_size}` is a saturating
/// gauge, not a monotonic total: measured on tmux 3.7c against a pane with
/// `history-limit 10`, it pins at 10 and stays there while output keeps
/// arriving, so its delta is zero for the entire life of any pane past its
/// scrollback limit — which is every long-running agent, since the managed
/// server sets no limit and takes tmux's default of 2000 lines.
/// `#{history_bytes}` plateaus the same way.
///
/// What IS always available is the rendered screen: the watcher captures one
/// per terminal per second to classify agent activity, whether or not any
/// client is connected. Two consecutive captures say how far the screen moved,
/// and tmux — which is the emulator — has already done the hard part.
///
/// # What it counts, and what it gets wrong
///
/// A line that left the top of the screen is a line that was genuinely produced
/// and pushed past, so this is screen-aware by construction: a TUI that repaints
/// its own frame in place scrolls nothing and correctly contributes nothing.
/// That is the failure mode a raw `\n` count has, and it is not present here.
///
/// Two named inaccuracies, so nobody files them as bugs later:
///
/// - **A burst faster than the pane is tall saturates.** More than `rows` lines
///   between two captures leaves no overlap to measure, and the answer is
///   `rows` — a floor, not the truth. A `cargo build` firehose reads as a solid
///   bar rather than a taller one. At thirteen buckets scaled per half that is
///   the same drawing.
/// - **A wholesale repaint reads as one screenful.** Entering or leaving the
///   alternate screen, or a `clear`, produces a screen with nothing in common
///   with the last one and is counted as `rows`. Once per invocation of `vim` or
///   `less`, against buckets five minutes wide.
///
/// This is a texture, not an accounting figure. The spec draws it 12pt wide and
/// scales it per row against that row's own past; it is never summed, never
/// compared between agents and never printed as a number.
///
/// # The method
///
/// Score every possible shift and take the best, rather than looking for an
/// exact suffix match. An exact match is far too brittle: one spinner glyph
/// changing in place on an otherwise still screen breaks every shift including
/// zero, and the fallback would report a full screenful every second forever.
///
/// `rows x rows` u64 comparisons — 1600 for a 40-row pane, once per terminal
/// per second — which is nothing next to the `capture-pane` that produced the
/// text.
pub fn lines_produced(previous: &ScreenShape, current: &ScreenShape) -> u32 {
    let rows = current.rows.len();
    if rows == 0 || previous.rows.len() != rows {
        // A resize. The two screens are not comparable and guessing across one
        // would report the resize itself as output.
        return 0;
    }

    let mut best_shift = 0usize;
    let mut best_score = 0usize;
    for shift in 0..rows {
        let overlap = rows - shift;
        let score = (0..overlap).filter(|i| previous.rows[shift + i] == current.rows[*i]).count();
        // Strictly greater, so ties go to the SMALLEST shift. A mostly blank
        // screen matches itself well at every shift, and "it did not scroll" is
        // the reading that does not invent output out of empty rows.
        if score > best_score {
            best_score = score;
            best_shift = shift;
        }
    }

    // Nothing lined up anywhere: the screen was replaced, not scrolled. See the
    // second named inaccuracy above.
    //
    // Two rows of agreement is the threshold. One can happen by chance between
    // two unrelated screens that both have a blank row in the same place, and
    // requiring more would misread a small pane whose content genuinely all
    // changed.
    if best_score < 2 {
        return rows as u32;
    }

    if best_shift > 0 {
        return best_shift as u32;
    }

    // It did not scroll, which for a screen that is not yet full does not mean
    // nothing was printed — the first `rows` lines of a fresh pane never push
    // anything off the top. Growth of the occupied region is those lines.
    match (previous.bottom, current.bottom) {
        (Some(was), Some(now)) => now.saturating_sub(was) as u32,
        (None, Some(now)) => now as u32 + 1,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ten past four in the afternoon, UTC, on an arbitrary day. Any absolute
    /// second does; what matters below is that the same one is used twice.
    const AFTERNOON: i64 = 1_788_000_000;

    fn shape(lines: &[&str]) -> ScreenShape {
        ScreenShape::of(&lines.join("\n"))
    }

    /// **Constraint: two polls inside one bucket must produce identical bytes.**
    ///
    /// This is the property `agentsSayTheSame(as:)` is built on. A window
    /// measured backwards from `now` changes on every poll and turns that guard
    /// into dead code, so this test is the one that must never be deleted or
    /// weakened into "roughly equal".
    #[test]
    fn two_polls_inside_one_bucket_encode_the_same_bytes() {
        let mut trace = Trace::new();
        trace.record(AFTERNOON, Sample { output: 40, code: 12, commits: 1 });

        // Two reads a minute and a half apart, both inside the same five-minute
        // wall-clock bucket. Nothing was recorded between them.
        let first = trace.encode(AFTERNOON + 10);
        let second = trace.encode(AFTERNOON + 100);

        assert!(!first.is_empty(), "a trace with activity must encode to something");
        assert_eq!(first, second, "the array shifted inside one bucket");
    }

    /// The same, across the widest window, where a sliding implementation would
    /// still look stable for minutes at a time and only betray itself here.
    #[test]
    fn two_polls_hours_apart_inside_one_wide_bucket_encode_the_same_bytes() {
        let mut trace = Trace::new();
        // Old enough to force the two-hour width.
        trace.record(AFTERNOON - 20 * 3600, Sample::output(9));
        trace.record(AFTERNOON, Sample::output(3));

        let base = (AFTERNOON / 7200) * 7200;
        let first = trace.encode(base + 60);
        let second = trace.encode(base + 7000);
        assert_eq!(first[0] & 0x0f, 2, "this trace should have snapped to the 2h width");
        assert_eq!(first, second, "the array shifted inside one two-hour bucket");
    }

    /// **Constraint: crossing a boundary shifts by exactly one and drops the
    /// oldest.**
    #[test]
    fn crossing_a_boundary_shifts_by_one_and_drops_the_oldest() {
        let mut trace = Trace::new();
        // One distinct value per bucket, twelve buckets back to now, so a shift
        // is visible rather than a coincidence.
        let now = (AFTERNOON / BASE_WIDTH) * BASE_WIDTH;
        for back in 0..BUCKETS as i64 {
            let at = now - back * BASE_WIDTH;
            trace.record(at, Sample::output(100 + (BUCKETS as u32 - 1 - back as u32)));
        }

        let before = trace.buckets(now, BASE_WIDTH);
        // One second past the boundary into the next bucket.
        let after = trace.buckets(now + BASE_WIDTH, BASE_WIDTH);

        assert_eq!(before[0].output, 100, "oldest bucket is not the one written oldest");
        assert_eq!(before[12].output, 112, "newest bucket is not the one written newest");

        // Everything moved left by exactly one, the oldest fell off, and the
        // newly exposed newest bucket is empty.
        for i in 0..BUCKETS - 1 {
            assert_eq!(
                after[i], before[i + 1],
                "bucket {i} after the crossing is not bucket {} from before",
                i + 1
            );
        }
        assert_eq!(after[12], Sample::default(), "the new bucket arrived non-empty");
    }

    /// A bucket boundary is a wall-clock second, not an offset from startup.
    #[test]
    fn bucket_edges_land_on_wall_clock_multiples() {
        let mut trace = Trace::new();
        trace.record(AFTERNOON, Sample::output(1));
        // Every second inside one five-minute wall-clock slot renders the same.
        let edge = (AFTERNOON / BASE_WIDTH) * BASE_WIDTH;
        let at_edge = trace.encode(edge);
        for offset in [1, 17, 149, BASE_WIDTH - 1] {
            assert_eq!(trace.encode(edge + offset), at_edge, "offset {offset} moved the window");
        }
        assert_ne!(trace.encode(edge + BASE_WIDTH), at_edge, "the boundary did not bite");
    }

    /// Nothing recorded means nothing sent, not 66 zero bytes.
    #[test]
    fn an_empty_trace_encodes_to_nothing() {
        let trace = Trace::new();
        assert!(trace.encode(AFTERNOON).is_empty());
    }

    #[test]
    fn an_encoded_trace_is_exactly_the_documented_length() {
        let mut trace = Trace::new();
        trace.record(AFTERNOON, Sample::output(1));
        assert_eq!(trace.encode(AFTERNOON).len(), ENCODED_LEN);
        assert_eq!(ENCODED_LEN, 66);
    }

    /// The window snaps to the shortest one that contains the activity.
    #[test]
    fn the_window_snaps_to_the_shortest_that_holds_the_activity() {
        let width = |ago: i64| {
            let mut trace = Trace::new();
            trace.record(AFTERNOON - ago, Sample::output(1));
            trace.tick(AFTERNOON);
            trace.encode(AFTERNOON)[0] & 0x0f
        };
        assert_eq!(width(60), 0, "a minute of history is the 5-minute width");
        assert_eq!(width(50 * 60), 0, "50 minutes still fits 13 x 5min");
        assert_eq!(width(3 * 3600), 1, "3 hours needs the 30-minute width");
        assert_eq!(width(20 * 3600), 2, "20 hours needs the 2-hour width");
    }

    /// A gap wider than the ring must not resurrect what was in those slots.
    #[test]
    fn a_ring_that_wrapped_does_not_report_yesterdays_work() {
        let mut trace = Trace::new();
        trace.record(AFTERNOON, Sample::output(500));
        // A closed laptop. Exactly one ring later, the same slot comes round.
        let later = AFTERNOON + SLOTS as i64 * BASE_WIDTH;
        trace.tick(later);
        let buckets = trace.buckets(later, BASE_WIDTH);
        assert!(
            buckets.iter().all(|b| *b == Sample::default()),
            "a stale slot was read as this hour's: {buckets:?}"
        );
    }

    /// A trace read long after it was last written must not find its own past
    /// aliased into its future.
    ///
    /// Clearing is the writer's job — `advance_to` — and a read is not allowed
    /// to depend on a writer having run recently. Without the bound in
    /// `oldest_activity`, the slots between `newest` and `now` still hold what
    /// was written into them the previous time round the ring, and this trace
    /// would report a full day of activity that stopped a day ago.
    #[test]
    fn a_trace_nobody_ticked_does_not_read_its_own_past_as_the_present() {
        let mut trace = Trace::new();
        trace.record(AFTERNOON, Sample::output(77));
        // Read a full ring later with no tick in between.
        let later = AFTERNOON + SLOTS as i64 * BASE_WIDTH + 600;
        assert!(
            trace.encode(later).is_empty(),
            "a ring read a day late reported stale slots as current activity"
        );
    }

    /// Values above what a byte or a short can hold saturate rather than wrap.
    /// Wrapping would draw a very busy bucket as an empty one.
    #[test]
    fn oversized_counts_saturate_instead_of_wrapping() {
        let mut trace = Trace::new();
        trace.record(AFTERNOON, Sample { output: 999_999, code: 70_000, commits: 300 });
        let bytes = trace.encode(AFTERNOON);
        let code = u16::from_le_bytes([bytes[1 + 24], bytes[1 + 25]]);
        let output = u16::from_le_bytes([bytes[27 + 24], bytes[27 + 25]]);
        assert_eq!(code, u16::MAX);
        assert_eq!(output, u16::MAX);
        assert_eq!(bytes[53 + 12], u8::MAX);
    }

    /// The fleet trace adds rings slot for slot, in phase.
    #[test]
    fn absorbing_adds_the_same_wall_clock_slot() {
        let mut one = Trace::new();
        one.record(AFTERNOON, Sample::output(10));
        one.record(AFTERNOON - 3 * BASE_WIDTH, Sample::output(4));

        let mut two = Trace::new();
        two.record(AFTERNOON, Sample::output(7));

        let mut fleet = Trace::new();
        fleet.absorb(&one, AFTERNOON);
        fleet.absorb(&two, AFTERNOON);

        let buckets = fleet.buckets(AFTERNOON, BASE_WIDTH);
        assert_eq!(buckets[12].output, 17, "the newest slot did not add up");
        assert_eq!(buckets[9].output, 4, "the older slot landed in the wrong bucket");
    }

    /// A ring that went stale must not be absorbed as if it were current.
    #[test]
    fn absorbing_ignores_slots_older_than_the_ring() {
        let mut old = Trace::new();
        old.record(AFTERNOON - SLOTS as i64 * BASE_WIDTH, Sample::output(99));

        let mut fleet = Trace::new();
        fleet.absorb(&old, AFTERNOON);
        assert!(fleet.encode(AFTERNOON).is_empty(), "a wrapped ring leaked into the fleet");
    }

    #[test]
    fn a_still_screen_produced_nothing() {
        let screen = shape(&["alpha", "beta", "gamma", "", "", ""]);
        assert_eq!(lines_produced(&screen, &screen), 0);
    }

    /// The case exact matching gets catastrophically wrong: one cell changing
    /// on an otherwise still screen must not read as a full screenful.
    #[test]
    fn a_spinner_on_a_still_screen_produces_nothing() {
        let before = shape(&["alpha", "beta", "| working", "", "", "", "", ""]);
        let after = shape(&["alpha", "beta", "/ working", "", "", "", "", ""]);
        assert_eq!(lines_produced(&before, &after), 0);
    }

    #[test]
    fn a_full_screen_scrolling_by_three_counts_three() {
        let before = shape(&["a", "b", "c", "d", "e", "f", "g", "h"]);
        let after = shape(&["d", "e", "f", "g", "h", "i", "j", "k"]);
        assert_eq!(lines_produced(&before, &after), 3);
    }

    /// A pane that is not yet full scrolls nothing, but lines were still
    /// printed. Losing these would report a quiet shell as silent forever.
    #[test]
    fn lines_appended_below_the_content_count_even_without_a_scroll() {
        let before = shape(&["a", "b", "", "", "", "", "", ""]);
        let after = shape(&["a", "b", "c", "d", "", "", "", ""]);
        assert_eq!(lines_produced(&before, &after), 2);
    }

    #[test]
    fn the_first_lines_on_a_blank_pane_count() {
        let before = shape(&["", "", "", "", "", "", "", ""]);
        let after = shape(&["a", "b", "c", "", "", "", "", ""]);
        assert_eq!(lines_produced(&before, &after), 3);
    }

    /// Documented behavior, asserted so it is a decision rather than an
    /// accident: a screen with nothing in common reads as one screenful.
    #[test]
    fn a_wholesale_repaint_reads_as_one_screenful() {
        let before = shape(&["a", "b", "c", "d"]);
        let after = shape(&["w", "x", "y", "z"]);
        assert_eq!(lines_produced(&before, &after), 4);
    }

    /// A resize is not output. Comparing across one would count the reflow.
    #[test]
    fn a_resize_produces_nothing() {
        let before = shape(&["a", "b", "c", "d"]);
        let after = shape(&["a", "b", "c", "d", "e", "f"]);
        assert_eq!(lines_produced(&before, &after), 0);
    }
}
