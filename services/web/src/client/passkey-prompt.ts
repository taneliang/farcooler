/// The Use Passkey screen's rule (ov-767 fix round 1): one answer, then
/// nothing.
///
/// The screen appears when the browser refused to prompt (Safari, once the
/// dial and the handshake outlasted the Connect click) or the person dismissed
/// the prompt. The SSH connection waits on it, and the runner's sshd drops a
/// login that hasn't authenticated after LoginGraceTime, 120 s by default. So
/// the screen ends, as declined, on Cancel or after `USE_PASSKEY_WINDOW_MS`,
/// well inside that; and once it has ended, a late click asks nothing of the
/// passkey, so no touch is spent on a connection that is already gone.

/// How long Use Passkey waits for a click: half of sshd's default
/// LoginGraceTime, leaving the other half for the dial and the handshake that
/// came before it.
export const USE_PASSKEY_WINDOW_MS = 60_000

export interface Timers {
  set(run: () => void, ms: number): unknown
  clear(handle: unknown): void
}

const browserTimers: Timers = {
  set: (run, ms) => setTimeout(run, ms),
  clear: handle => clearTimeout(handle as ReturnType<typeof setTimeout>),
}

/// Show the screen through `show(use, cancel)`, and answer `sign()`'s result
/// once Use Passkey is clicked. Rejects with `declined` on Cancel or when the
/// window closes. `sign` is called at most once, and only from `use`, so it
/// runs inside the click (what Safari requires of a passkey prompt).
export function askToUsePasskey<T>(
  show: (use: () => void, cancel: () => void) => void,
  sign: () => Promise<T>,
  windowMs = USE_PASSKEY_WINDOW_MS,
  timers: Timers = browserTimers,
): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    let open = true
    const close = (): boolean => {
      if (!open) return false
      open = false
      timers.clear(timer)
      return true
    }
    const timer = timers.set(() => {
      if (close()) reject(new Error('declined'))
    }, windowMs)
    show(
      () => {
        if (close()) sign().then(resolve, reject)
      },
      () => {
        if (close()) reject(new Error('declined'))
      },
    )
  })
}
