// Bounds and clocks the relay's modules share (ov-310): a leaf, so a module
// can import them without importing `index.ts`, which imports it back.

/// How long a row stays in an account's roster before it is forgotten.
///
/// Twenty-four hours, and the number is the design's own rather than a round
/// one. A row's trace snaps to the shortest window that contains its activity —
/// 1h, 6h or 24h — so past a day it cannot contribute to any window the card can
/// draw, and it has nothing left to say. Purging at the design's own maximum is
/// the smallest number that loses nothing visible.
///
/// Applied LAZILY, on write, and per account on the account's own notice, so
/// the work is proportional to what is actually running. The one cron trigger
/// (`sweepQuiet`) reaches it only through `readFleet`, for accounts with a card
/// up.
export const ROW_RETENTION_MS = 24 * 60 * 60 * 1000

/// The most of a workspace's name a row keeps, in bytes.
///
/// Sixteen characters of the widest script, and far more than a sidebar shows
/// of an ASCII name. Exported because it is payload arithmetic — see
/// `ROWS_SHOWN` in `index.ts`.
export const WORKSPACE_BUDGET = 48

/// `text`, cut to at most `bytes` of UTF-8, never mid-character.
///
/// Bytes and not characters, because the cap is bytes: a card carrying an
/// agent's own words can be three bytes a character, and cutting at a hundred
/// and twenty of those is nearly four hundred. `Intl.Segmenter` would be more
/// correct about grapheme clusters and is not worth it here — the worst a code
/// point boundary can do is separate an emoji from its modifier at the very end
/// of a line that was already too long to read.
///
/// Exported for the same reason the budgets above are: what it does is
/// arithmetic on a cap APNs enforces, and the only property anything ever
/// measured was the LENGTH of what came back. A `cut` that returned its input
/// REVERSED satisfied all three callers and the whole suite with them, which is
/// not a property anybody would have claimed for it.
export function cut(text: string, bytes: number): string {
  const encoder = new TextEncoder()
  if (encoder.encode(text).length <= bytes) return text
  let out = ''
  let size = 0
  // `for...of` iterates code points rather than UTF-16 units, so a surrogate
  // pair is never split in half — which would produce a lone surrogate, and
  // `JSON.stringify` writes that as an escape the app decodes to a replacement
  // character.
  for (const character of text) {
    const width = encoder.encode(character).length
    if (size + width > bytes) break
    out += character
    size += width
  }
  return out
}

/// How long a runner may be silent before it's quiet, in milliseconds: two
/// missed beats and five minutes' slack for a slow request. Fifteen minutes at
/// the shipped five-minute beat.
///
/// **A second statement of `RunnerPulse.quietAfter`** in AgentKit, which the
/// phone's widget and the watch judge `/v1/pulse` by. The card can't judge on
/// its own — it redraws only when the relay pushes it — so the relay has to
/// know when to push, and that is this. The two are pinned together by
/// `RunnerPulseTests` (15 minutes at 300 s) and by the 14- and 16-minute
/// tests in `a runner that stops beating, on the card`.
export function quietAfterMs(beatEvery: number): number {
  return (2 * beatEvery + 5 * 60) * 1000
}
