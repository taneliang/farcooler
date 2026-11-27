/// The screens that wait for a runner or another device to add this browser:
/// Add Runner, Approve This Browser From Another Device, the code both sides
/// compare, and a teammate's join (ov-771, ov-1490). What they own is the
/// offer on screen: its reply key, its expiry and the wait for its answer.

import { fingerprint, freshnessMs, joinCommand, makeOffer, mergeRunnerGrants, newReplySecret, offerIsStale, openCliReply, openReply, unenrolledRunners as unenrolledIn } from 'fc-web'

import { drawUnenrolled, joinCommandFor, unenrolledRunners } from './add-runner'
import { compareWords, JOIN_MISMATCH, joinDetail, joinTitle, MISMATCH, type ReplyFrom, sealedRefusal } from './copy'
import { type Device, type Granted, saveDevice } from './device'
import { drawCode } from './qr'
import { say } from './say'
import { comparisonButtons, type Opened, waitForReply } from './sealed'
import type { ShellScreen } from './shell/store'

export interface EnrollingSession {
  device: Device
  keyLine: string
}

/// What these screens need of the page around them.
export interface EnrollingPage<S extends EnrollingSession> {
  $: <T extends HTMLElement = HTMLElement>(id: string) => T
  show: (screen: ShellScreen) => void
  relayRaw: (route: string, body?: unknown) => Promise<{ status: number; body: any }>
  deviceName: () => string
  register: (session: S) => Promise<void>
  work: (session: S) => Promise<void>
  keepCopy: (session: S) => Promise<void>
}

export function enrollingScreens<S extends EnrollingSession>(page: EnrollingPage<S>) {
  const { $, show, relayRaw, deviceName, register, work, keepCopy } = page

  /// The timer that retires the code on screen when it expires.
  let retiring = 0

  /// The secret half of the reply key behind the code on screen. In memory, and
  /// in this tab's `sessionStorage` so a reload keeps the code answerable; never
  /// in IndexedDB, which outlives the tab. A code whose secret is gone is retired.
  const SECRET = 'fc-reply-secret'
  let replySecret: string | null = null
  function keepSecret(secret: string): void {
    replySecret = secret
    try {
      sessionStorage.setItem(SECRET, secret)
    } catch {}
  }
  function loadSecret(): string | null {
    if (replySecret) return replySecret
    try {
      replySecret = sessionStorage.getItem(SECRET)
    } catch {}
    return replySecret
  }
  function dropSecret(): void {
    replySecret = null
    try {
      sessionStorage.removeItem(SECRET)
    } catch {}
  }

  /// Which wait for a sealed reply is current; a new count cancels the one before.
  let polling = 0
  const stopPolling = () => void ++polling


  /// Which of the two waiting screens this tab is on, so a reload stays there.
  const WAITING = 'fc-waiting-on'
  function waitingOn(screen?: 'device' | 'runner'): 'device' | 'runner' {
    try {
      if (screen) sessionStorage.setItem(WAITING, screen)
      return sessionStorage.getItem(WAITING) === 'device' ? 'device' : 'runner'
    } catch {
      return screen ?? 'runner'
    }
  }

  /// A browser with no runner starts at Add Runner (ov-1490), unless this tab
  /// was waiting for another device's approval.
  function firstScreen(session: S): Promise<void> {
    return waitingOn() === 'device' ? enroll(session) : addRunner(session)
  }

  /// This browser's offer, still answerable: a stored one past the window, or
  /// one whose secret is gone, is replaced.
  async function freshOffer(session: S): Promise<string> {
    const { device } = session
    if (device.offer && (offerIsStale(device.offerShownAt ?? 0) || !loadSecret())) {
      delete device.offer
      delete device.offerShownAt
    }
    if (!device.offer) {
      const secret = newReplySecret()
      keepSecret(secret)
      device.offer = makeOffer(deviceName(), device.account, session.keyLine, device.nodePub, secret)
      device.offerShownAt = Date.now()
      await saveDevice(device)
    }
    return device.offer
  }

  /// Replace the code on screen when it expires.
  function retireWith(session: S, screen: string, again: () => void): void {
    retiring = window.setTimeout(() => {
      if (!$(screen).hidden) again()
    }, Math.max(0, (session.device.offerShownAt ?? 0) + freshnessMs() - Date.now()) + 500)
  }

  /// The account's runners this browser doesn't open, or null when unreadable.
  function listUnenrolled(session: S) {
    return unenrolledRunners({ call: relayRaw, unenrolled: unenrolledIn }, session.device.account, session.device.runners ?? [])
  }

  /// Which render of Add Runner is current, so a slow answer for an old one
  /// draws nothing.
  let adding = 0

  /// Add Runner: the command to run on a computer, then wait for that runner's
  /// answer, which the person compares before it is kept.
  async function addRunner(session: S, notice = ''): Promise<void> {
    const { device } = session
    clearTimeout(retiring)
    stopPolling()
    waitingOn('runner')
    if (!device.registered) {
      await register(session)
      return
    }
    const mine = ++adding
    const offer = await freshOffer(session)
    const note = $('join-note')
    const tell = (words: string) => {
      note.textContent = words
      note.hidden = words === ''
    }
    tell(notice)
    const command = $('join-command')
    const copy = $<HTMLButtonElement>('join-copy')
    command.textContent = say({ say: 'getting_command' })
    copy.disabled = true
    $('join-fingerprint').textContent = fingerprint(session.keyLine) ?? ''
    const back = $<HTMLButtonElement>('join-back')
    back.hidden = !device.runners?.length
    back.onclick = () => {
      stopPolling()
      clearTimeout(retiring)
      void work(session)
    }
    $<HTMLButtonElement>('join-from-device').onclick = () => void enroll(session)
    show('runner-add')
    retireWith(session, 'runner-add', () => void addRunner(session, say({ say: 'command_expired' })))
    void waitForSealedReply(session, note, 'runner')

    void listUnenrolled(session).then(listed => {
      if (mine !== adding) return
      $('unenrolled-failed').hidden = listed !== null
      $('unenrolled').hidden = !listed?.length
      drawUnenrolled($('unenrolled-list'), listed ?? [], null, Date.now())
    })
    const made = await joinCommandFor({ call: relayRaw, command: joinCommand }, offer).catch(() => null)
    if (mine !== adding) return
    if (!made) {
      command.textContent = ''
      tell(say({ say: 'command_unavailable' }))
      return
    }
    command.textContent = made
    copy.disabled = false
    copy.textContent = say({ say: 'copy_command' })
    copy.onclick = () => void copyText(made, copy)
  }

  async function copyText(text: string, button: HTMLButtonElement): Promise<void> {
    const was = button.textContent
    const ok = await navigator.clipboard.writeText(text).then(() => true, () => false)
    button.textContent = ok ? 'Copied' : say({ say: 'copy_by_hand' })
    window.setTimeout(() => (button.textContent = was), 2000)
  }

  /// Show this browser's code, and read another device's reply to it.
  ///
  /// A code is answerable for the ceremony's window (two minutes) from when it
  /// went up, as on every other device: the reply leg carries no proof of who
  /// wrote it, and everything a forged reply must echo is printed in the code. So
  /// a stored code past the window is discarded for a new one, a code on screen
  /// is replaced when it expires, and a reply that comes later is refused in
  /// wasm (`openReply`).
  async function enroll(session: S, notice = ''): Promise<void> {
    const { device } = session
    clearTimeout(retiring)
    stopPolling()
    waitingOn('device')
    adding++
    if (!device.registered) {
      // Tried without a click first; a refusal to prompt becomes a button.
      await register(session)
      return
    }
    const offer = await freshOffer(session)

    drawCode($<HTMLCanvasElement>('offer-code'), offer)
    $('offer-fingerprint').textContent = say({ say: 'own_passkey', fingerprint: fingerprint(session.keyLine) ?? '' })
    const copy = $<HTMLButtonElement>('offer-copy')
    copy.onclick = () => void copyText(offer, copy)
    const note = $('enroll-note')
    note.textContent = notice
    note.hidden = notice === ''
    $('offer-account').textContent = device.account
    $<HTMLTextAreaElement>('cli-offer').value = offer
    const reply = $<HTMLTextAreaElement>('cli-reply')
    reply.value = ''
    $('cli-compare').onclick = () => {
      try {
        const opened = JSON.parse(openCliReply(reply.value, device.offer ?? '', loadSecret() ?? '', device.offerShownAt ?? 0))
        reply.value = ''
        compare(session, opened, [], 'cli')
      } catch (error) {
        note.textContent = sealedRefusal((error as Error).message)
        note.hidden = false
      }
    }
    show('enroll')
    // A reply to the retired code is refused by the same freshness window.
    retireWith(session, 'enroll', () => void enroll(session, say({ say: 'code_expired' })))

    void waitForSealedReply(session, note, 'device')

    const back = $<HTMLButtonElement>('enroll-back')
    back.hidden = !device.runners?.length
    back.onclick = () => {
      stopPolling()
      clearTimeout(retiring)
      void work(session)
    }
    $<HTMLButtonElement>('enroll-add-runner').onclick = () => void addRunner(session)

    $<HTMLButtonElement>('start-over').onclick = async () => {
      dropSecret()
      delete device.offer
      delete device.offerShownAt
      await saveDevice(device)
      void enroll(session)
    }
  }

  /// Wait for the sealed reply to the code on screen. When one opens, the
  /// person compares its code; nothing is kept before they say it matches.
  function waitForSealedReply(session: S, note: HTMLElement, from: ReplyFrom): void {
    const { device } = session
    const offer = device.offer
    if (!offer) return
    const ceremony = (JSON.parse(offer) as { ceremony: string }).ceremony
    const mine = ++polling
    void waitForReply(
      {
        read: id => relayRaw('ceremony/reply', { ceremony: id }),
        open: answer => JSON.parse(openReply(answer, device.offer ?? '', loadSecret() ?? '', !device.offer, device.offerShownAt ?? 0)) as Opened[],
        now: () => Date.now(),
        sleep: ms => new Promise(resolve => setTimeout(resolve, ms)),
        cancelled: () => mine !== polling,
        refused: word => {
          note.textContent = sealedRefusal(word)
          note.hidden = false
        },
      },
      ceremony,
    ).then(async waited => {
      if (waited.kind === 'opened') compare(session, waited.opened, waited.others, from)
      else if (waited.kind === 'ended' && mine === polling) {
        // Too late for this code, and so for any reply to it: a new one.
        dropSecret()
        delete device.offer
        delete device.offerShownAt
        await saveDevice(device)
        void (from === 'runner' ? addRunner : enroll)(session, sealedRefusal(waited.word))
      }
    })
  }

  /// The reply's own code, to match against the phone's before it is kept.
  function compare(session: S, opened: Opened, others: Opened[] = [], from: ReplyFrom = 'device'): void {
    stopPolling()
    clearTimeout(retiring)
    adding++
    const words = compareWords(opened.granted_by, from)
    $('compare-title').textContent = words.title
    $('compare-detail').textContent = words.detail
    $('compare-code').textContent = opened.code
    show('compare')
    // Nothing was kept on a mismatch and the code is still unanswered: the QR
    // reply and a later sealed one still work.
    // Another reply from the same read may be the real one (ov-778 R-113): show
    // it before starting over.
    const next = () =>
      others.length ? compare(session, others[0], others.slice(1), from) : void (from === 'runner' ? addRunner(session, JOIN_MISMATCH) : enroll(session, MISMATCH))
    const buttons = comparisonButtons(opened, taken => keep(session, taken, from), next)
    const yes = $<HTMLButtonElement>('compare-yes')
    const no = $<HTMLButtonElement>('compare-no')
    yes.disabled = false
    no.disabled = false
    // The first tap answers, so both buttons go dark at once: no second save.
    yes.onclick = () => {
      yes.disabled = true
      no.disabled = true
      void buttons.yes()
    }
    no.onclick = () => {
      yes.disabled = true
      no.disabled = true
      void buttons.no()
    }
  }

  /// The runner to connect to next, once, after it was just added.
  let justAdded: string | null = null

  /// Keep what a confirmed reply grants. For a runner that ran the join command,
  /// the match is also the person's approval of it: the relay adds it to the
  /// account only now. A reply that didn't come through a ticket has none to
  /// approve, which the relay says with a 404.
  async function keep(session: S, opened: Opened, from: ReplyFrom = 'device'): Promise<void> {
    if (from === 'runner' && !opened.granted_by && session.device.offer) {
      const ceremony = (JSON.parse(session.device.offer) as { ceremony: string }).ceremony
      const answer = await relayRaw('join/approve', { ceremony }).catch(() => null)
      if (!answer || (answer.status !== 200 && answer.status !== 404)) {
        $('copy-note-text').textContent = say({ say: 'not_in_account' })
        $('copy-note').hidden = false
      }
    }
    justAdded = opened.runners.find(r => r.why === '')?.id ?? null
    await finishTaken(session, { granted_by: opened.granted_by, runners: opened.runners })
  }

  /// A sealed reply whose code the person matched spends the offer.
  async function finishTaken(session: S, taken: { granted_by: string | null; runners: Granted[] }): Promise<void> {
    const { device } = session
    stopPolling()
    clearTimeout(retiring)
    dropSecret()
    delete device.offer
    delete device.offerShownAt
    await saveDevice(device)
    if (taken.granted_by) {
      confirmJoin(session, taken.granted_by, taken.runners)
      return
    }
    device.runners = mergeRunners(device.runners ?? [], taken.runners)
    await saveDevice(device)
    await keepCopy(session)
    void work(session)
  }

  function mergeRunners(known: Granted[], added: Granted[]): Granted[] {
    return JSON.parse(mergeRunnerGrants(JSON.stringify(known), JSON.stringify(added)))
  }

  /// Another account's runners: asked about, never joined silently, as a phone
  /// asks (apps/ios/FarCooler/Ceremony/JoinView.swift, TeammateJoinPrompt).
  function confirmJoin(session: S, from: string, runners: Granted[]): void {
    $('join-title').textContent = joinTitle(runners.map(r => r.label))
    $('join-detail').textContent = joinDetail(from)
    const list = $('join-runners')
    list.replaceChildren(
      ...(runners.length > 1
        ? runners.map(r => {
            const item = document.createElement('li')
            item.textContent = r.label
            return item
          })
        : []),
    )
    show('join')
    $<HTMLButtonElement>('join-yes').onclick = async () => {
      session.device.runners = mergeRunners(session.device.runners ?? [], runners)
      await saveDevice(session.device)
      await keepCopy(session)
      void work(session)
    }
    $<HTMLButtonElement>('join-no').onclick = () => void firstScreen(session)
  }

  /// The runner just added, once: the one to offer to connect to next.
  function takeAdded(): string | null {
    const added = justAdded
    justAdded = null
    return added
  }

  return { firstScreen, addRunner, enroll, listUnenrolled, dropSecret, takeAdded }
}
