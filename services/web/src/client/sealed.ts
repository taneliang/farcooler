/// A phone enrolls this browser without a camera (ov-771): the sealed reply
/// over the relay, the code to compare, and this browser's registration with
/// the account's device directory.
///
/// Three rules, each held in one place here and tested (test/sealed.test.ts):
///
/// 1. **Only the earliest reply is opened.** Anyone who saw the offer and may
///    post can post a reply that opens, so the browser never picks among them
///    (crates/web `enroll::earliest` takes the first the relay lists).
/// 2. **The code is this browser's own**, from its own open of that reply. A
///    reply sealed by anyone but the person's phone opens, and shows another
///    code than the phone's: that difference is the only defense there is.
/// 3. **Nothing is kept until the person says the codes match**
///    ([`confirmCodes`] is the only way an opened reply reaches `keep`).
///
/// Every browser and wasm call is passed in, so a test plays them.

import type { Granted, Signed } from './device'

/// How often the relay is asked for the reply, and the longest this waits: the
/// ceremony's window (crates/ceremony `FRESHNESS`, 120 s), by this browser's clock.
export const POLL_MS = 2000
export const WAIT_MS = 120_000

/// A reply opened but not yet trusted: what it grants, and its code.
export interface Opened {
  code: string
  granted_by: string | null
  runners: Granted[]
}

export interface Answer {
  status: number
  body: any
}

export interface PollDeps {
  /// `POST /api/relay/ceremony/reply`.
  read: (ceremony: string) => Promise<Answer>
  /// crates/web `openReply`, given the relay's answer as JSON. Throws an Error
  /// whose message is the ceremony's word.
  open: (relayAnswer: string) => Opened
  now: () => number
  sleep: (ms: number) => Promise<void>
  /// The offer on screen was retired or replaced: stop.
  cancelled: () => boolean
  /// A reply that did not open or was refused, by the ceremony's word. The
  /// wait goes on: a forger's junk must not end the real reply's wait.
  refused: (word: string) => void
}

export type Waited = { kind: 'opened'; opened: Opened } | { kind: 'ended'; word: 'stale' | 'already_taken' } | { kind: 'timeout' } | { kind: 'cancelled' }

/// Ask the relay for the reply to `ceremony` every two seconds, for up to
/// 120 s, and open the earliest one it hands over.
///
/// A read takes every reply there is from the relay, so a reply that doesn't
/// open is gone: the wait goes on for the next, and says so through `refused`.
export async function waitForReply(deps: PollDeps, ceremony: string): Promise<Waited> {
  const started = deps.now()
  while (deps.now() - started < WAIT_MS) {
    if (deps.cancelled()) return { kind: 'cancelled' }
    let answer: Answer | null = null
    try {
      answer = await deps.read(ceremony)
    } catch {
      // A dropped request is the next poll's to answer.
    }
    if (deps.cancelled()) return { kind: 'cancelled' }
    if (answer?.status === 200) {
      try {
        return { kind: 'opened', opened: deps.open(JSON.stringify(answer.body)) }
      } catch (e) {
        const word = (e as Error).message
        if (word === 'stale' || word === 'already_taken') return { kind: 'ended', word }
        deps.refused(word)
      }
    }
    await deps.sleep(POLL_MS)
  }
  return { kind: 'timeout' }
}

/// The person's answer to "do the two codes match?". Only a yes keeps the
/// reply; a no does nothing at all, so it neither saves a runner nor counts
/// the offer as taken.
export async function confirmCodes(opened: Opened, match: boolean, keep: (opened: Opened) => Promise<void>): Promise<boolean> {
  if (!match) return false
  await keep(opened)
  return true
}

/// What the two buttons under the code do. "They Match" keeps the reply;
/// "They Don't Match" keeps nothing and calls `mismatched`, which puts the
/// still-unanswered code back on screen.
export function comparisonButtons(opened: Opened, keep: (opened: Opened) => Promise<void>, mismatched: () => void) {
  return {
    yes: () => void confirmCodes(opened, true, keep),
    no: async () => {
      await confirmCodes(opened, false, keep)
      mismatched()
    },
  }
}

export interface RegisterDeps {
  /// `POST /api/relay/<route>`.
  call: (route: string, body: unknown) => Promise<Answer>
  /// The passkey signs `data` as its challenge: one touch. Throws `declined`.
  sign: (data: Uint8Array) => Promise<Signed>
  /// crates/web `registrationData` and `registrationProof`.
  data: (account: string, challenge: string, keyLine: string) => Uint8Array
  proof: (account: string, challenge: string, keyLine: string, signed: Signed) => string
}

export type Registered = { ok: true } | { ok: false; word: 'declined' | 'unreachable' | 'claimed' | 'revoked' | 'refused' }

/// Register this browser's passkey with the directory as `label`, with one
/// touch. The relay mints a challenge, wasm builds what the passkey signs and
/// the proof from its answer, and `devices/register` checks the proof. The
/// device then waits on the directory as pending until the person's phone
/// certifies it.
export async function registerDevice(deps: RegisterDeps, account: string, keyLine: string, label: string): Promise<Registered> {
  // The relay is given the key as a bare two-token line, as the phones give it.
  const bare = keyLine.trim().split(/\s+/).slice(0, 2).join(' ')
  const minted = await deps.call('directory/devices/challenge', {}).catch(() => null)
  const challenge = minted?.status === 200 ? minted.body?.challenge : undefined
  if (typeof challenge !== 'string') return { ok: false, word: 'unreachable' }
  let signed: Signed
  try {
    signed = await deps.sign(deps.data(account, challenge, bare))
  } catch (e) {
    return { ok: false, word: (e as Error).message === 'declined' ? 'declined' : 'refused' }
  }
  const proofA = deps.proof(account, challenge, bare, signed)
  const answer = await deps.call('directory/devices/register', { label, challenge, keyA: bare, proofA }).catch(() => null)
  if (!answer || answer.status >= 500 || answer.status === 429) return { ok: false, word: 'unreachable' }
  if (answer.status === 200) return { ok: true }
  const error = answer.body?.error
  return { ok: false, word: error === 'claimed' ? 'claimed' : error === 'revoked' ? 'revoked' : 'refused' }
}
