/// The start screen while the page waits (ov-869): what it is waiting on, how
/// long it has waited, and a way forward once that is longer than a start
/// should take.
///
/// The owner's first start after a deploy sat on "Starting Far Cooler…" long
/// enough to look stuck, with nothing to say why. Every byte the page needs is
/// new after a deploy (each file's name is its hash), so the first start
/// fetches the account through the relay and downloads about 4.8 MB of wasm
/// (assets.ts) before it can show anything, over whatever link the browser
/// has. None of those waits has a deadline of its own, so the screen keeps the
/// count instead: the step, then the seconds once they are worth showing, and
/// at `SLOW_MS` a sentence and Reload or Sign Out and Start Again. The start
/// keeps going behind it, and replaces the screen if it finishes.

/// When the seconds start to show: a normal start is done well before this.
export const SHOW_SECONDS_MS = 3_000
/// When the screen says the start is slow and offers a way forward.
export const SLOW_MS = 10_000

export type StartStep = 'account' | 'load' | 'settings'

const TITLES: Record<StartStep, string> = {
  account: 'Checking your account…',
  load: 'Loading Far Cooler…',
  settings: 'Opening this browser’s settings…',
}

/// What a step says while it waits.
export const STEP_DETAIL: Record<StartStep, string> = {
  account: 'Far Cooler is asking which account is signed in.',
  load: 'Far Cooler is downloading the parts that run in your browser. The first start after an update takes longest.',
  settings: 'Far Cooler is reading the runners this browser keeps.',
}

export interface StartScreen {
  title: string
  detail: string
  /// Past `SLOW_MS`: offer Reload, and Sign Out and Start Again.
  slow: boolean
}

/// The screen for `step` after `elapsedMs` since the page began to start.
export function startScreen(step: StartStep, elapsedMs: number): StartScreen {
  const seconds = Math.floor(elapsedMs / 1000)
  const title = TITLES[step]
  if (elapsedMs >= SLOW_MS)
    return {
      title,
      detail: `${seconds} s so far, which is longer than a start usually takes. Your connection may be slow. Reload to try again, or sign out and start again.`,
      slow: true,
    }
  if (elapsedMs >= SHOW_SECONDS_MS) return { title, detail: `${STEP_DETAIL[step]} ${seconds} s so far.`, slow: false }
  return { title, detail: STEP_DETAIL[step], slow: false }
}

export interface Ticker {
  now(): number
  every(run: () => void, ms: number): unknown
  stop(handle: unknown): void
}

const browserTicker: Ticker = {
  now: () => Date.now(),
  every: (run, ms) => setInterval(run, ms),
  stop: handle => clearInterval(handle as ReturnType<typeof setInterval>),
}

/// Draws the start screen through `draw` each second until `done`. The time
/// counts from construction, so a step that follows a slow one is slow too:
/// the person has waited the whole while.
export class StartWatch {
  private step: StartStep = 'account'
  private readonly began: number
  private readonly handle: unknown
  private ended = false

  constructor(
    private readonly draw: (screen: StartScreen) => void,
    private readonly ticker: Ticker = browserTicker,
  ) {
    this.began = ticker.now()
    this.handle = ticker.every(() => this.redraw(), 1000)
    this.redraw()
  }

  /// The page has moved on to `step`.
  at(step: StartStep): void {
    this.step = step
    this.redraw()
  }

  /// The start has finished, or failed with a screen of its own: draw nothing
  /// more.
  done(): void {
    if (this.ended) return
    this.ended = true
    this.ticker.stop(this.handle)
  }

  get finished(): boolean {
    return this.ended
  }

  private redraw(): void {
    if (!this.ended) this.draw(startScreen(this.step, this.ticker.now() - this.began))
  }
}
