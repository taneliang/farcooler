/// The client shell (ov-764, plan-763 slice 1): enroll this browser with the
/// owner's phone, then open one of a runner's terminals.
///
/// 1. Both wasm modules load at boot — never lazily, because a deploy replaces
///    every hashed file and an open tab must not ask for one that's gone.
/// 2. A browser with no runners shows today's v2 offer as a QR code. The phone
///    runs its existing ceremony on it, and the browser reads the phone's reply
///    sealed through the relay. crates/web accepts it with the ceremony's own
///    rules and keeps each runner's host key.
/// 3. A browser with runners dials the first one it can reach through the
///    tunnel, SSHes in with that host key pinned, lists its terminals and
///    attaches the one picked.
///
/// The SSH key is a passkey (ov-767). Safari shows a passkey prompt only
/// from a click, so every prompt here follows a button: Make Passkey before
/// the code, Connect before each sign-in, and Use Passkey if the browser
/// refused to prompt because that click was too long ago (the dial and the
/// SSH handshake come between). Each connection costs one touch.
///
/// Every value from the network goes into the page through `textContent`,
/// never markup: the CSP's Trusted Types (`require-trusted-types-for
/// 'script'`) refuse an HTML string sink outright.

import { openWorkflow, installWorkflowPort } from './workflow-view'
import { WorkflowSessionPort } from './workflow-session'
import init, { connect, fingerprint, freshnessMs, makeOffer, newReplySecret, offerIsStale, needsYouView, openReply, passkeyKeyLine, planView, recoveryView, registrationData, registrationProof, rpId, workFailure, workView, workspaceLayout, workspaceOrchestrator, type WebRunner } from 'fc-web'

import { compareWords, connectFailure, joinDetail, joinTitle, linkNotice, MISMATCH, needsYouFailure, registerFailure, restoreFailure, runnerStopped, saveFailure, sealedRefusal, unreachable } from './copy'
import { askForCopy, assertWith, canUsePasskeys, userHandle, type Device, forgetDevice, type Granted, loadDevice, makePasskey, type Passkey, saveDevice, type Signed } from './device'
import { Fleet, type RunnerDeps } from './fleet'
import { type BoardView, buttonsFor, countLabel, countOf, isCounted, pageTitle } from './needs'
import { renderNeeds } from './needs-view'
import { type PlanTab, startPlanTab } from './plan-tab'
import { PlanList } from './plan-list'
import { scopeBoard, type Space, waitingIn } from './scope'
import { linkedRunner, mountWork, type WorkTerminal, type WorkView } from './work-screen'
import { drawPicker, saveSpace, savedSpace } from './space-picker'
import { DetailBoard, type Subject } from './detail'
import { type Answer, DetailSheet } from './detail-view'
import { corePhrases } from './core-phrases'
import { useNeedsYouPresenter } from './needs-present'
import { useConversationWords } from './chat-copy'
import { askToUsePasskey } from './passkey-prompt'
import { drawCode } from './qr'
import { type Budget, MAX_LIVE_PANES } from './panes'
import { readLocal, WorkspaceLayout, writeLocal, type Tab } from './workspace-layout'
import { WorkspacePanes } from './workspace-panes'
import { WorkspaceHistory, type Route } from './workspace-history'
import { comparisonButtons, type Opened, registerDevice, waitForReply } from './sealed'
import { type LinkView, Reconnector } from './reconnect'
import { installSignOut } from './sign-out'
import { relayCopy, relayRaw as requestRelay } from './relay-client'
import { type RecoveryPresenter, type RecoveryView, SavedCopies } from './saved-copies'
import { StartWatch } from './start'
import { loadTailcat, type Tailcat, type TunnelConn } from './tunnel'
import { b64url, deriveKeys, needsSaving, prfUse, restore, syncCopy, type Saved, type Vault } from './vault'

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T

let isSigningOut = () => false

function relayRaw(route: string, body: unknown = {}) {
  return requestRelay(route, body, () => !isSigningOut())
}

async function relay(route: string, body: unknown = {}): Promise<any> {
  const answer = await relayRaw(route, body)
  return answer.status >= 200 && answer.status < 300 ? answer.body : null
}

/// Never reloads on its own: a reload mid-keystroke would lose what was typed.
async function checkVersion(): Promise<void> {
  const response = await fetch('/version.json', { cache: 'no-store' }).catch(() => null)
  const deployed = response?.ok ? await response.json().catch(() => null) : null
  if (typeof deployed?.build !== 'string' || deployed.build === __BUILD__) return
  $('update').removeAttribute('hidden')
}

const SCREENS = ['status', 'enroll', 'compare', 'join', 'work']

function show(screen: string): void {
  for (const id of SCREENS) $(id).toggleAttribute('hidden', id !== screen)
}

/// A full-page message, with at most one button, and a quiet second one
/// (Cancel unless `cancelLabel` says otherwise) when `cancel` is given.
function status(title: string, detail = '', action?: { label: string; run: () => void }, cancel?: () => void, cancelLabel = 'Cancel'): void {
  $('status-title').textContent = title
  $('status-detail').textContent = detail
  const button = $<HTMLButtonElement>('status-action')
  button.hidden = !action
  button.onclick = action ? () => action.run() : null
  if (action) button.textContent = action.label
  const quit = $<HTMLButtonElement>('status-cancel')
  quit.hidden = !cancel
  quit.onclick = cancel ? () => cancel() : null
  quit.textContent = cancelLabel
  show('status')
}

/// What this browser calls itself on the phone's confirmation screen.
function deviceName(): string {
  const ua = navigator.userAgent
  const browser = /Edg\//.test(ua) ? 'Edge' : /Firefox\//.test(ua) ? 'Firefox' : /Chrome\//.test(ua) ? 'Chrome' : /Safari\//.test(ua) ? 'Safari' : 'Browser'
  const os = /Windows/.test(ua) ? 'Windows' : /Mac OS X/.test(ua) ? 'Mac' : /CrOS/.test(ua) ? 'ChromeOS' : /Linux/.test(ua) ? 'Linux' : 'Computer'
  return `${browser} on ${os}`
}

interface Session {
  device: Device
  /// The passkey's OpenSSH line, once there is a passkey.
  keyLine: string
  email: string
  /// The passkey's user handle for this account (`userHandle`).
  user: Uint8Array
  tailcat: Tailcat
  /// The sealed copy's key, once an assertion in this tab gave the passkey's
  /// PRF output (vault.ts), and whether that output came verified. In memory
  /// only: the passkey gives it again.
  vault?: { key: CryptoKey; uv: boolean }
  copyWork?: Promise<void>
  signingOut?: boolean
}

// MARK: The sealed copy

/// Sign with the passkey, and take the PRF output the same touch gave
/// (vault.ts) before the signature goes on: the node key a first offer names
/// must be the derived one.
async function signed(session: Session, signing: Promise<Signed>): Promise<Signed> {
  const answer = await signing
  await notePrf(session, answer).catch(e => console.error('Far Cooler couldn’t use the passkey’s PRF output', e))
  return answer
}

/// What one assertion's PRF output gives this tab: the sealed copy's key, and
/// for a browser that hasn't yet named a node key in an offer, a node key
/// derived from the passkey. An output whose verification flag differs from
/// the one this device uses may be another output, so it is ignored.
async function notePrf(session: Session, answer: Signed): Promise<void> {
  const prf = answer.prf
  const { device } = session
  const use = prfUse(device, prf ?? null)
  if (!prf || !use.use) return
  const keys = await deriveKeys(prf.output)
  if (use.deriveNode) {
    const node = session.tailcat.nodeKeyFromSeed(keys.nodeSeed)
    if (node) {
      device.nodePriv = node.priv
      device.nodePub = node.pub
      device.nodeDerived = true
    }
  }
  device.prfUv = prf.uv
  session.vault = { key: keys.vaultKey, uv: prf.uv }
  await saveDevice(device)
  // Not awaited: this runs inside an SSH login, which mustn't wait on the relay.
  void keepCopy(session, true).catch(e => console.error('Far Cooler couldn’t keep the sealed copy', e))
}

/// Bring the relay's sealed copy up to date with this device, when this tab
/// has its key and the device changed since the last copy. A browser without
/// PRF has no copy: its storage is asked to persist instead, so the browser
/// is less likely to clear it.
///
/// After a touch, repair a missing, unreadable, or incomplete copy. Merge
/// the relay's authenticated runners before saving (ov-1009).
function keepCopy(session: Session, recheck = false): Promise<void> {
  if (session.signingOut) return Promise.resolve()
  const work = (session.copyWork ?? Promise.resolve()).catch(() => {}).then(() => {
    if (!session.signingOut) return keepCopyNow(session, recheck)
  })
  session.copyWork = work
  return work
}

async function keepCopyNow(session: Session, recheck: boolean): Promise<void> {
  const { device } = session
  if (!session.vault) {
    if (device.runners?.length) void navigator.storage?.persist?.().catch(() => {})
    return
  }
  if (!recheck && !(await needsSaving(device))) return
  const saved = await syncCopy(relayCopy, session.vault.key, device, session.vault.uv)
  if (saved === 'saved') await saveDevice(device)
  copyNote(saved)
}

/// A copy the relay turned down is said once, in the banner under the header,
/// since this browser's runners then go when its storage does. A relay that
/// didn't answer isn't: the next touch tries again.
function copyNote(saved: Saved): void {
  if (saved === 'unreachable') return
  const note = $('copy-note')
  note.hidden = saved === 'saved'
  if (saved !== 'saved') $('copy-note-text').textContent = saveFailure(saved)
}

/// A browser with nothing kept, on an account with sealed copies: one touch
/// of the passkey brings the runners back, with no enrollment. Make a New
/// Passkey is the way to add the browser from the phone instead.
function restoreScreen(session: Session, vaults: Vault[], migrated: boolean, notice = ''): void {
  status(
    'Reconnect with your passkey',
    notice || 'Far Cooler keeps your runners sealed with your passkey. Touch your passkey once to use them in this browser, with no need to add it from your phone.',
    { label: 'Use Passkey', run: () => void restoreWith(session, vaults, migrated) },
    () => passkeyScreen(session, migrated),
    'Make a New Passkey',
  )
}

async function restoreWith(session: Session, vaults: Vault[], migrated: boolean): Promise<void> {
  const done = await restore(
    { ask: (ids, uv) => askForCopy(rpId(), ids, uv), nodeKey: seed => session.tailcat.nodeKeyFromSeed(seed) },
    session.device.account,
    session.user,
    vaults,
  )
  if (!done.ok) return restoreScreen(session, vaults, migrated, restoreFailure(done.word))
  const { device } = done
  session.device = device
  session.keyLine = keyLineOf(device.passkey as Passkey)
  session.vault = { key: done.vaultKey, uv: !!device.prfUv }
  await saveDevice(device)
  if (device.runners?.length) void work(session)
  else void enroll(session)
}

/// The account's sealed copies, or none when the relay has none or can't say.
async function listCopies(): Promise<Vault[]> {
  const answer = await relayRaw('web/vault').catch(() => null)
  const vaults = answer?.status === 200 ? answer.body?.vaults : null
  return Array.isArray(vaults) ? vaults.filter(v => typeof v?.credential === 'string' && typeof v?.sealed === 'string' && typeof v?.uv === 'boolean') : []
}

// MARK: The passkey

/// Ask for this browser's passkey, from a button.
function passkeyScreen(session: Session, migrated: boolean, notice = ''): void {
  const why = migrated ? 'Far Cooler now signs in to your runners with a passkey instead of a key kept in this site’s storage, so this browser needs adding again. ' : ''
  status(
    'Make a passkey for this browser',
    notice ||
      `${why}Your passkey stays in this device’s password manager or security key, and Far Cooler never sees it. You’ll touch it once each time this browser connects to a runner.`,
    {
      label: 'Make Passkey',
      run: () =>
        void makePasskey(rpId(), session.user, session.email).then(
          async passkey => {
            session.device.passkey = passkey
            await saveDevice(session.device)
            session.keyLine = keyLineOf(passkey)
            void enroll(session)
          },
          e =>
            passkeyScreen(
              session,
              migrated,
              (e as Error).message === 'declined'
                ? 'Far Cooler didn’t get a passkey. Select Make Passkey and follow your browser’s prompt.'
                : 'This browser couldn’t make a passkey Far Cooler can use. Use a current version of Chrome, Safari, Edge or Firefox.',
            ),
        ),
    },
  )
}

function keyLineOf(passkey: Passkey): string {
  return passkeyKeyLine(passkey.spki, 'farcooler-web')
}

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

// MARK: Registering

/// Put this browser's passkey on the account's device list, so the phone can
/// certify it and it shows there as a device (ov-771). One touch. Tried as soon
/// as it is needed; if the browser refuses to prompt without a click (Safari),
/// a button asks for the click.
async function register(session: Session): Promise<void> {
  const { device } = session
  const passkey = device.passkey as Passkey
  const rp = rpId()
  status('Adding this browser to your account…', 'Touch your passkey if your browser asks.')
  const done = await registerDevice(
    {
      call: relayRaw,
      sign: data => signed(session, assertWith(passkey, rp, data)),
      data: registrationData,
      proof: (account, challenge, keyLine, signed) =>
        registrationProof(account, challenge, keyLine, new Uint8Array(signed.authenticatorData), new Uint8Array(signed.clientDataJSON), new Uint8Array(signed.signature)),
    },
    device.account,
    session.keyLine,
    deviceName(),
  )
  if (!done.ok) {
    status('Add this browser to your account', registerFailure(done.word), { label: 'Register This Browser', run: () => void register(session) })
    return
  }
  device.registered = true
  await saveDevice(device)
  await keepCopy(session)
  void enroll(session)
}

// MARK: Enrolling

/// Show this browser's code, and read the phone's reply to it.
///
/// A code is answerable for the ceremony's window (two minutes) from when it
/// went up, as on every other device: the reply leg carries no proof of who
/// wrote it, and everything a forged reply must echo is printed in the code. So
/// a stored code past the window is discarded for a new one, a code on screen
/// is replaced when it expires, and a reply that comes later is refused in
/// wasm (`openReply`).
async function enroll(session: Session, notice = ''): Promise<void> {
  const { device } = session
  clearTimeout(retiring)
  stopPolling()
  if (!device.registered) {
    // Tried without a click first; a refusal to prompt becomes a button.
    await register(session)
    return
  }
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

  drawCode($<HTMLCanvasElement>('offer-code'), device.offer)
  $('offer-fingerprint').textContent = `This browser’s passkey: ${fingerprint(session.keyLine) ?? ''}`
  const note = $('enroll-note')
  note.textContent = notice
  note.hidden = notice === ''
  show('enroll')
  // A reply to the retired code is refused by the same freshness window.
  retiring = window.setTimeout(() => {
    if ($('enroll').hidden) return
    void enroll(session, 'That code expired, so here’s a new one. Scan it with your phone, then compare the code it sends back.')
  }, Math.max(0, (device.offerShownAt ?? 0) + freshnessMs() - Date.now()) + 500)

  void waitForSealedReply(session, note)

  $<HTMLButtonElement>('start-over').onclick = async () => {
    dropSecret()
    delete device.offer
    delete device.offerShownAt
    await saveDevice(device)
    void enroll(session)
  }

}

/// Wait for the phone's sealed reply to the code on screen. When one opens, the
/// person compares its code; nothing is kept before they say it matches.
function waitForSealedReply(session: Session, note: HTMLElement): void {
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
    if (waited.kind === 'opened') compare(session, waited.opened, waited.others)
    else if (waited.kind === 'ended' && mine === polling) {
      // Too late for this code, and so for any reply to it: a new one.
      dropSecret()
      delete device.offer
      delete device.offerShownAt
      await saveDevice(device)
      void enroll(session, sealedRefusal(waited.word))
    }
  })
}

/// The reply's own code, to match against the phone's before it is kept.
function compare(session: Session, opened: Opened, others: Opened[] = []): void {
  stopPolling()
  clearTimeout(retiring)
  const words = compareWords(opened.granted_by)
  $('compare-title').textContent = words.title
  $('compare-detail').textContent = words.detail
  $('compare-code').textContent = opened.code
  show('compare')
  // Nothing was kept on a mismatch and the code is still unanswered: the QR
  // reply and a later sealed one still work.
  // Another reply from the same read may be the real one (ov-778 R-113): show
  // it before starting over.
  const next = () => (others.length ? compare(session, others[0], others.slice(1)) : void enroll(session, MISMATCH))
  const buttons = comparisonButtons(opened, taken => keep(session, taken), next)
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

/// Keep what a confirmed reply grants.
async function keep(session: Session, opened: Opened): Promise<void> {
  await finishTaken(session, { granted_by: opened.granted_by, runners: opened.runners })
}

/// A sealed reply whose code the person matched spends the offer.
async function finishTaken(session: Session, taken: { granted_by: string | null; runners: Granted[] }): Promise<void> {
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
  device.runners = taken.runners
  await saveDevice(device)
  await keepCopy(session)
  void work(session)
}

/// Another account's runners: asked about, never joined silently, as a phone
/// asks (apps/ios/FarCooler/Ceremony/JoinView.swift, TeammateJoinPrompt).
function confirmJoin(session: Session, from: string, runners: Granted[]): void {
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
    session.device.runners = runners
    await saveDevice(session.device)
    await keepCopy(session)
    void work(session)
  }
  $<HTMLButtonElement>('join-no').onclick = () => void enroll(session)
}

// MARK: Working

/// The terminals on the page, so a second visit to `work` replaces them.
let panes: WorkspacePanes | null = null
let layout: WorkspaceLayout | null = null
let navigation: WorkspaceHistory | null = null
/// What each runner's sshd has shown it will hold, kept for the session.
const paneBudgets = new Map<string, Budget>()

// MARK: Needs You

/// How often the list is read while the tab is in view. The runner pushes
/// nothing a browser listens to yet, so this is what shows an answer made
/// on another device.
const NEEDS_POLL_MS = 5000

/// Every enrolled runner's Needs You on the page, read again every few seconds.
let fleet: Fleet | null = null
let boardTimer = 0
/// The connections made to the runners other than the one in front, which
/// carry Needs You and nothing else (ov-861).
let others: Reconnector<WebRunner, TunnelConn>[] = []
/// What a runner's note button does, by runner id.
const retries = new Map<string, () => void>()
let drawn = ''

/// The terminal buttons by id, which is what an Open on an item can go to.
let workScreen: ReturnType<typeof mountWork> | null = null

/// The workspace workflow sheet, preserving the panes behind it.
let workflowSheet: ReturnType<typeof openWorkflow> | null = null
/// The plan tab, while a runner is connected.
let planTab: PlanTab | null = null
let planList: PlanList | null = null

/// What the plan does to the panes of the runner in front: open a terminal,
/// and leave words in the orchestrator's message box.
let paneControls: { open(terminal: string): boolean } | null = null

function showTab(tab: Tab): void {
  navigation?.navigate({ space: chosenSpace, page: tab, tab })
}

/// The workspaces of the runner in front, the one showing, and the last view
/// the board gave (so a pick redraws without waiting for a poll).
let spaces: Space[] = []
let chosenSpace: string | null = null
let frontRunner = ''
let lastView: BoardView | null = null
let pickerDrawn = ''

/// Put the board's view on the page: the list for the workspace showing, the
/// count on the button, the bar and the tab's title. The bar and the title
/// count everything waiting, in every workspace and on every runner; the
/// button counts what the list shows, and the picker says how many wait in
/// each workspace.
function drawNeeds(view: BoardView): void {
  lastView = view
  const scoped = scopeBoard(view, spaces, chosenSpace, frontRunner)
  planTab?.setNeeds(scoped.items.filter(i => isCounted(i) && !i.elsewhere))
  const total = countOf(view.items)
  $('show-needs').textContent = countLabel(countOf(scoped.items))
  const bar = $<HTMLButtonElement>('bar-count')
  bar.hidden = total === 0
  bar.textContent = `${total} Waiting`
  document.title = pageTitle(total)
  pickerDrawn = drawPicker($<HTMLSelectElement>('space-picker'), spaces, chosenSpace, waitingIn(view, spaces, frontRunner), pickerDrawn)
  drawn = renderNeeds(
    $('needs'),
    scoped,
    Date.now(),
    {
      canOpen: (terminal, item) => fleet?.canOpenItem(terminal, item) ?? false,
      press: (item, press) => {
        void fleet?.press(item, press)
      },
      retry: runner => retries.get(runner)?.(),
      detail: item => {
        if (item.task) openCard(item.runner ?? frontRunner, item.task)
      },
    },
    drawn,
  )
  if (sheet?.isOpen) sheet.redraw()
}

/// The card sheet, made the first time a card is opened.
let sheet: DetailSheet | null = null

/// Open a task's detail, read from the runner it is on. The buttons that
/// answer it are the Needs You item's own, while one waits on the person.
function openCard(runner: string, task: { id: string; key: string; title: string }): void {
  if (fleet?.cardsOf(runner)?.deps?.task && navigation) {
    navigation.navigate({ ...navigation.route, page: 'ticket', ticket: { runner, ...task } }, true)
  }
}

function showCard(ticket: NonNullable<Route['ticket']>, scope: string, afterDraw: () => void): void {
  const member = fleet?.cardsOf(ticket.runner)
  const deps = member?.deps
  if (!member || !deps?.task || !deps.note) return
  const read = deps.task
  const write = deps.note
  sheet ??= new DetailSheet($('detail') as HTMLDialogElement, $, () => corePhrases(planView).waitingOnYou, () => layout?.run('close_page'))
  const subject: Subject = ticket
  const board = new DetailBoard({ runner: member.label, task: id => read(id), addNote: (id, body) => write(id, body), show: view => { sheet?.draw(view); afterDraw() } }, subject)
  const key = `${scope}:ticket:${JSON.stringify([ticket.runner, ticket.id])}:draft`
  sheet.open(board, () => answersFor(ticket.runner, ticket.id), readLocal(key), text => writeLocal(key, text))
}

/// The buttons a waiting item on this task offers (not Open: the sheet covers
/// the terminal), each pressed on the runner the item is on.
function answersFor(runner: string, task: string): Answer[] {
  const item = lastView?.items.find(i => i.runner === runner && i.task?.id === task && i.kind !== 'fyi')
  if (!item) return []
  return buttonsFor(item, () => false).map(({ action, press }) => ({
    label: action.title,
    primary: action.primary,
    destructive: action.destructive,
    run: () => void fleet?.press(item, press),
  }))
}

function stopNeeds(): void {
  workScreen?.dispose()
  workScreen = null
  navigation?.stop()
  navigation = null
  layout?.stop()
  layout = null
  sheet?.close()
  workflowSheet?.close()
  workflowSheet = null
  installWorkflowPort(() => null)
  planList?.close()
  planTab?.stop()
  planTab = null
  paneControls = null
  clearInterval(boardTimer)
  fleet?.stop()
  fleet = null
  for (const rec of others) rec.current?.free()
  others = []
  retries.clear()
  drawn = ''
  pickerDrawn = ''
  lastView = null
  spaces = []
  chosenSpace = null
  document.title = pageTitle(0)
  $('bar-count').hidden = true
}

async function work(session: Session): Promise<void> {
  const runners = session.device.runners ?? []
  const wanted = linkedRunner()
  const target = wanted ? runners.find(r => r.id === wanted && r.why === '') : runners.find(r => r.why === '')
  if (wanted && !target) {
    status('This runner is unavailable', 'Open this link in a browser paired with the runner.',
      { label: 'Add Again', run: () => void addAgain(session) })
    return
  }
  if (!target) {
    const first = runners[0]
    status(
      'No runner here can be opened from a browser yet',
      first ? unreachable(first.why, first.label, first.cause) : 'Your phone granted no runners.',
      { label: 'Add Again', run: () => void addAgain(session) },
    )
    return
  }

  // A button first: the passkey prompt this leads to needs a click (Safari).
  status(`Connect to ${target.label}`, 'You’ll touch your passkey to sign in.', { label: 'Connect', run: () => void connectTo(session, target) })
}

/// The passkey's signature over the SSH login's data: asked for straight
/// away, which Chrome, Edge and Firefox allow whenever, and Safari while the
/// Connect click still counts. If the browser refused to prompt (Safari, once
/// the dial and the handshake outlasted that click) or the person dismissed
/// the prompt, a Use Passkey button asks again, with the call made inside its
/// click, until Cancel or its window closes (`askToUsePasskey`). Either way
/// one touch signs.
function signIn(session: Session, label: string): (challenge: Uint8Array) => Promise<Signed> {
  const passkey = session.device.passkey as Passkey
  const rp = rpId()
  return async challenge => {
    try {
      return await signed(session, assertWith(passkey, rp, challenge))
    } catch (e) {
      if ((e as Error).message !== 'declined') throw e
    }
    return await askToUsePasskey(
      (use, cancel) =>
        status(
          `Sign in to ${label}`,
          'Select Use Passkey, then touch your passkey.',
          {
            label: 'Use Passkey',
            run: () => {
              status(`Connecting to ${label}…`)
              use()
            },
          },
          () => {
            status(`Connecting to ${label}…`)
            cancel()
          },
        ),
      () => signed(session, assertWith(passkey, rp, challenge)),
    )
  }
}

/// The connection on the page, so a tab coming back or the browser coming
/// online can ask whether it still answers (ov-772).
let link: Reconnector<WebRunner, TunnelConn> | null = null

/// What the page says about the connection: nothing while it is up, else a
/// notice above the terminal, with Reconnect when a click is wanted.
function showLink(view: LinkView, label: string): void {
  // The first connection is still listing terminals; the screen is not up yet.
  if (view.kind !== 'up' || panes) show('work')
  const notice = linkNotice(view, label)
  $('link-note').hidden = !notice
  if (!notice) return
  $('link-title').textContent = notice.title
  $('link-detail').textContent = notice.detail
  const action = $<HTMLButtonElement>('link-action')
  action.hidden = !notice.action
  if (notice.action) action.textContent = notice.action
  action.onclick = () => void link?.reconnect()
}

async function connectTo(session: Session, target: Granted): Promise<void> {
  status(`Connecting to ${target.label}…`)
  let runner: WebRunner
  let screen: ReturnType<typeof mountWork> | undefined
  let mineFleet: Fleet | null = null
  const reconnector = new Reconnector<WebRunner, TunnelConn>({
    dial: () => session.tailcat.dial(target.token, session.device.nodePriv),
    discard: conn => conn.close(),
    connect: (conn, onLost) => connect(conn, target.host_key, target.user, session.keyLine, signIn(session, target.label), onLost),
    resume: async next => {
      await panes?.resume(next)
      await screen?.refresh()
    },
    show: view => {
      showLink(view, target.label)
      workflowSheet?.connection(view.kind === 'up')
      // Its list is read as soon as the connection is up, and set aside while
      // it is not.
      if (view.kind === 'up') mineFleet?.up(target.id)
      else mineFleet?.unreachable(target.id, needsYouFailure('closed', target.label))
    },
    now: () => Date.now(),
  })
  /// A pane's stream ended: said so unless the whole connection did, which
  /// the notice above the terminal already says.
  const ended = (from: WebRunner, id: string) => {
    // Asked a moment later: the loss reaches the page as the pane's end and as
    // `onLost`, in either order. The reconnector remembers it; the wasm object
    // cannot be asked once it is freed.
    setTimeout(() => {
      if (!reconnector.hasLost(from) && reconnector.current === from) {
        panes?.problem('This terminal’s stream ended. Select it to open it again.', id)
        screen?.streamEnded(id)
      }
    }, 100)
  }
  const workflowPort = new WorkflowSessionPort(() => {
    const current = reconnector.current
    return current ? { account: session.device.account, host: target.id, session: current } : null
  }, localStorage)
  link = reconnector
  let dialed: TunnelConn | null = null
  try {
    const conn = await session.tailcat.dial(target.token, session.device.nodePriv)
    dialed = conn
    runner = await reconnector.open(conn)
  } catch (e) {
    dialed?.close()
    const word = (e as Error).message
    const action =
      word === 'host_key_changed' || word === 'host_key_unknown' || word === 'key_rejected'
        ? { label: 'Add Again', run: () => void addAgain(session) }
        : { label: 'Connect', run: () => void connectTo(session, target) }
    status(`Can’t connect to ${target.label}`, connectFailure(word, target.label), action)
    return
  }

  let inventory: string
  let initial: WorkView
  const params = new URLSearchParams(location.search)
  const linkedSpace = params.get('workspace')
  const linkedTerminal = params.get('terminal') ?? undefined
  try {
    inventory = await runner.work_inventory()
    initial = JSON.parse(workView(inventory, JSON.stringify({
      workspace: linkedSpace, terminal: linkedTerminal, preferredWorkspace: savedSpace(target.id),
    })))
  } catch (e) {
    status(`Can’t list ${target.label}’s terminals`, connectFailure((e as Error).message, target.label), {
      label: 'Connect',
      run: () => void connectTo(session, target),
    })
    return
  }

  $('runner-name').textContent = target.label
  const list = $('terminal-list')
  list.replaceChildren()
  show('work')

  stopNeeds()
  installWorkflowPort(id => id === target.id && reconnector.current ? workflowPort : null)
  frontRunner = target.id
  spaces = initial.workspaces
  const fallbackSpace = initial.workspace ?? linkedSpace
  chosenSpace = fallbackSpace
  const scope = `fc-workspace:${JSON.stringify([session.device.account, target.id])}`
  const returning = history.state?.workspaceScope === scope
  const mine = new Fleet(
    drawNeeds,
    (runner, terminal) => {
      if (runner === target.id) paneControls?.open(terminal)
    },
    (runner, terminal) => runner === target.id && !!screen?.hasTerminal(terminal),
  )
  mineFleet = mine
  fleet = mine
  for (const r of session.device.runners ?? []) {
    if (r.id === target.id) mine.add(r.id, r.label, depsOf(reconnector))
    else if (r.why !== '') mine.addUnreachable(r.id, r.label, unreachable(r.why, r.label, r.cause))
    else joinRunner(session, r, mine, !returning)
  }
  const liveRunner = () => {
    const current = reconnector.current
    return current && !reconnector.hasLost(current) ? current : null
  }
  planTab = startPlanTab($('plan'), target.label, liveRunner, chosenSpace, {
    openCard: card => openCard(target.id, { id: card.task, key: card.key, title: '' }),
    quote: async text => {
      if (!orchestrator) return false
      layout?.run('orchestrator')
      layout?.run('conversation', 'side')
      return !!await panes?.quote(text)
    },
    openAgent: agent => !!agent.terminal && !!paneControls?.open(agent.terminal),
    openList: kind => {
      const found = planTab?.listing(kind)
      if (!found) return
      planList ??= new PlanList($('plan-list') as HTMLDialogElement, $, card => openCard(target.id, { id: card.task, key: card.key, title: '' }))
      planList.open(found.plan, found.view, kind, found.count)
    },
  }, planView, raw => {
    if (navigation?.route.page !== 'plan' || !raw) return
    setOrchestrator(workspaceOrchestrator(JSON.stringify(raw)) ?? null)
    restoreScroll('plan')
  })

  panes?.dispose()
  const budget = paneBudgets.get(target.id) ?? { max: MAX_LIVE_PANES }
  paneBudgets.set(target.id, budget)
  const mounted = new WorkspacePanes(scope, target.label, budget, () => reconnector.current, ended)
  panes = mounted

  let shown: WorkTerminal[] = []
  let inventoryWorkspace: string | null | undefined
  let firstRender = true
  let focusedSlot: 'main' | 'side' = 'main'
  let pendingTerminal: { id: string; slot?: 'main' | 'side' } | undefined
  let orchestrator: string | null = null
  let discovery = 0
  let pendingScroll: { page: keyof typeof scrollIds; position: number } | null = null
  let previous: Route | null = null
  let opener: { element: Element | null; task: string | null } | null = null
  const scrollIds = { plan: 'plan', needs: 'needs', ticket: 'detail-body' } as const
  const scrollKey = (page: keyof typeof scrollIds) => `${scope}:scroll:${JSON.stringify([chosenSpace, page, navigation?.route.ticket?.id])}`
  const restoreScroll = (page: keyof typeof scrollIds) => {
    if (pendingScroll?.page !== page || navigation?.route.page !== page) return
    const element = $(scrollIds[page])
    if (element.scrollHeight <= element.clientHeight) return
    element.scrollTop = pendingScroll.position
    pendingScroll = null
  }
  for (const [page, id] of Object.entries(scrollIds)) {
    const element = $(id)
    element.onscroll = () => {
      if (pendingScroll?.page === page || navigation?.route.page !== page || element.scrollHeight <= element.clientHeight) return
      writeLocal(scrollKey(page as keyof typeof scrollIds), String(element.scrollTop))
    }
  }

  const controller = new WorkspaceLayout(scope, target.label, workspaceLayout, (view, intent) => {
    mounted.render(view, intent === 'open' || view.effect.kind === 'refresh_pane')
    for (const [id, action] of [
      ['show-orchestrator', 'orchestrator'], ['show-workflow', 'workflow'], ['show-commands', 'commands'],
      ['view-conversation', 'conversation'], ['side-conversation', 'conversation'],
      ['view-terminal', 'raw_terminal'], ['side-terminal', 'raw_terminal'], ['close-split', 'close_split'],
    ] as const) {
      const command = view.actions.find(a => a.id === action)!
      const button = $<HTMLButtonElement>(id)
      button.textContent = command.label
      if (id.startsWith('show-')) button.disabled = !command.enabled
      if (command.key) button.setAttribute('aria-keyshortcuts', view.shortcut_prefix + command.key.toUpperCase())
      else button.removeAttribute('aria-keyshortcuts')
    }
    list.hidden = navigation?.route.page !== 'terminals' && !view.side
    focusedSlot = view.focus
    const selected = view[view.focus]?.terminal
    if (selected && !pendingTerminal) screen?.focusTerminal(selected)
    if (!selected) $('terminal-manage').hidden = true
  }, effect => {
    if (effect.kind === 'focus') mounted.focus(effect.slot, false)
    else if (effect.kind === 'navigate') {
      if (effect.page === 'workflow') {
        if (navigation && navigation.route.page !== 'workflow') {
          navigation.navigate({ space: chosenSpace, page: 'workflow', tab: navigation.route.tab }, true)
        }
      } else showTab(effect.page)
    } else if (effect.kind === 'close_page') navigation?.leave()
  })
  layout = controller
  $('show-orchestrator').onclick = () => controller.run('orchestrator')
  $('show-workflow').onclick = () => controller.run('workflow')
  $('show-commands').onclick = () => controller.run('commands')
  $('close-split').onclick = () => controller.run('close_split')
  for (const [id, action, slot] of [
    ['view-conversation', 'conversation', 'main'], ['view-terminal', 'raw_terminal', 'main'],
    ['side-conversation', 'conversation', 'side'], ['side-terminal', 'raw_terminal', 'side'],
  ] as const) {
    $(id).onclick = () => { controller.run(action, slot); mounted.focus(slot, true) }
  }

  const openTerminal = (id: string, fromPage: boolean) => {
    if (!screen?.hasTerminal(id) || !navigation) return false
    const route = navigation.route
    const slot = fromPage ? 'main' : focusedSlot
    const next = slot === 'main'
      ? { space: chosenSpace, page: 'terminals', tab: 'terminals', terminal: id, slot } as const
      : { ...route, terminal: id, slot }
    if (route.page === next.page && route.terminal === id && route.slot === slot) {
      pendingTerminal = { id, slot }
      void screen.show(chosenSpace, id)
    } else navigation.navigate(next)
    return true
  }
  paneControls = { open: id => openTerminal(id, true) }

  const presentLayout = (page: Route['page']) => {
    const workspaceName = spaces.find(space => space.id === chosenSpace)?.name ?? null
    controller.contextFor(chosenSpace, workspaceName, spaces.length, page, shown, orchestrator)
  }

  screen = mountWork({
    runner: target.id,
    current: () => reconnector.current,
    inventory,
    project: workView,
    prefer: () => savedSpace(target.id),
    navigation: {
      workspace: space => navigation?.navigate({ space, page: 'terminals', tab: 'terminals' }),
      terminal: id => { openTerminal(id, false) },
    },
    failure: e => workFailure(e.message, e.code ?? '', e.what ?? ''),
    started: () => {},
    note: words => { if (words) mounted.problem(words) },
    select: (view, terminal, open, replace, historyMode) => {
      const route = navigation?.route
      if (!route) return
      spaces = view.workspaces
      if (replace) mounted.forget(replace)
      if (historyMode && (view.workspace !== route.space || open && terminal)) {
        navigation?.navigate({
          space: view.workspace, page: 'terminals', tab: 'terminals', terminal: open ? terminal?.id : undefined,
          slot: open && historyMode === 'replaceState' ? focusedSlot : 'main',
        }, false, historyMode === 'replaceState')
        return
      }
      shown = view.terminals
      inventoryWorkspace = route.space
      presentLayout(route.page)
      const pending = pendingTerminal
      pendingTerminal = undefined
      if (pending) {
        controller.open(pending.id, pending.slot)
        if (route.page === 'terminals') mounted.focus(focusedSlot, true)
      } else if (open && terminal) controller.open(terminal.id, focusedSlot)
      if (firstRender) {
        firstRender = false
        if (route.page === 'terminals') mounted.restoreFocus(true)
      }
      if (view.notice) mounted.problem(view.notice)
      if (lastView) drawNeeds(lastView)
      else pickerDrawn = drawPicker($<HTMLSelectElement>('space-picker'), spaces, chosenSpace, new Map())
    },
  })
  workScreen = screen

  const setOrchestrator = (id: string | null) => {
    if (id === orchestrator) return
    orchestrator = id
    if (inventoryWorkspace === chosenSpace && navigation) presentLayout(navigation.route.page)
  }

  const discover = async () => {
    const current = liveRunner()
    const space = chosenSpace
    if (!current || !space || layout !== controller || navigation?.route.page === 'plan') return
    const mine = ++discovery
    try {
      const id = workspaceOrchestrator(await current.plan(space)) ?? null
      if (mine !== discovery || layout !== controller || space !== chosenSpace || current !== liveRunner()) return
      setOrchestrator(id)
    } catch {}
  }

  const resolve = (route: Route): Route => {
    const unavailable = route.page === 'workflow' && !spaces.some(space => space.id === route.space) ||
      route.page === 'ticket' && (!route.ticket || !fleet?.cardsOf(route.ticket.runner)?.deps?.task)
    return unavailable ? { ...route, page: route.tab, ticket: undefined } : route
  }
  const renderRoute = (route: Route) => {
    const changedSpace = previous === null || chosenSpace !== route.space
    const cameFrom = previous
    if (route.page === 'ticket' && cameFrom?.page !== 'ticket') {
      const element = document.activeElement
      opener = { element, task: element?.closest('[data-task]')?.getAttribute('data-task') ?? null }
    }
    previous = route
    sheet?.close()
    workflowSheet?.close()
    workflowSheet = null
    planList?.close()
    chosenSpace = route.space
    const scrollPage = route.page in scrollIds ? route.page as keyof typeof scrollIds : null
    const position = scrollPage ? Number(readLocal(scrollKey(scrollPage))) : 0
    pendingScroll = scrollPage && position ? { page: scrollPage, position } : null
    if (changedSpace) {
      orchestrator = null
      if (spaces.some(space => space.id === chosenSpace)) saveSpace(target.id, chosenSpace!)
      planTab?.choose(chosenSpace)
    }
    const changedTerminal = route.terminal !== cameFrom?.terminal || route.slot !== cameFrom?.slot
    pendingTerminal = route.terminal && (changedSpace || changedTerminal) && !(cameFrom === null && returning)
      ? { id: route.terminal, slot: route.slot ?? 'main' } : undefined
    $('needs').hidden = route.page !== 'needs'
    $('plan').hidden = route.page !== 'plan'
    $('pane').hidden = route.page !== 'terminals'
    if (scrollPage && scrollPage !== 'ticket') $(scrollIds[scrollPage]).scrollTop = 0
    for (const tab of ['terminals', 'needs', 'plan'] as const) {
      $(`show-${tab}`).setAttribute('aria-pressed', String(tab === route.page))
    }
    if (changedSpace || pendingTerminal) void screen?.show(route.space, pendingTerminal?.id)
    else if (inventoryWorkspace === route.space) presentLayout(route.page)
    planTab?.show(route.page === 'plan')
    if (lastView) drawNeeds(lastView)
    else pickerDrawn = drawPicker($<HTMLSelectElement>('space-picker'), spaces, chosenSpace, new Map())
    if (route.page === 'needs') void mine.refresh()
    if (route.page === 'workflow' && chosenSpace) {
      workflowSheet = openWorkflow($<HTMLDialogElement>('workflow-runs'), target.id, chosenSpace,
        `${scope}:workflow:${JSON.stringify(chosenSpace)}`, () => controller.run('close_page'))
    }
    if (route.page === 'ticket' && route.ticket) showCard(route.ticket, scope, () => restoreScroll('ticket'))
    if (scrollPage) restoreScroll(scrollPage)
    if (cameFrom?.page === 'ticket' && route.page !== 'ticket') {
      const task = opener?.task ? `[data-task="${CSS.escape(opener.task)}"]` : null
      const sameCard = task ? [...document.querySelectorAll<HTMLElement>(`button${task},${task} button`)].find(el => el.getClientRects().length) : null
      const back = opener?.element instanceof HTMLElement && opener.element.getClientRects().length ? opener.element : sameCard ?? $(`show-${route.tab}`)
      back.focus({ preventScroll: true })
      opener = null
    }
    if (changedSpace) void discover()
  }
  navigation = new WorkspaceHistory(scope, target.id, { space: fallbackSpace, page: 'terminals', tab: 'terminals', terminal: linkedTerminal }, resolve, renderRoute)
  renderRoute(navigation.route)
  $<HTMLSelectElement>('space-picker').onchange = event => {
    const tab = navigation?.route.tab ?? 'terminals'
    navigation?.navigate({ space: (event.target as HTMLSelectElement).value, page: tab, tab })
  }
  mine.up(target.id)
  boardTimer = window.setInterval(() => {
    if (document.visibilityState !== 'visible') return
    void mine.refresh()
    void discover()
    void screen?.refresh()
    restoreScroll('ticket')
    restoreScroll('needs')
  }, NEEDS_POLL_MS)
}

/// What reads and answers for the runner `rec` is connected to.
function depsOf(rec: Reconnector<WebRunner, TunnelConn>): RunnerDeps {
  const live = (): WebRunner => {
    const current = rec.current
    if (!current) throw new Error('closed')
    return current
  }
  return {
    list: async () => await live().needsYou(),
    ask: async (terminal, ask, option) => await live().answer(terminal, ask, option),
    decide: async (task, body) => await live().answerDecision(task, body),
    task: async id => await live().task(id),
    note: async (id, body) => await live().addNote(id, body),
  }
}

/// Passkey touches are asked one at a time: the connections to the runners
/// behind the one in front wait their turn.
let turn: Promise<void> = Promise.resolve()

/// Connect to a runner behind the one in front, for its Needs You alone: its
/// panes and plan stay with the runner in front. If it can't be reached the
/// list says so and offers Connect (or Reconnect, once it has been up), whose
/// click is also what Safari wants before it prompts for a passkey.
function joinRunner(session: Session, target: Granted, into: Fleet, automatically: boolean): void {
  let everUp = false
  const passkey = session.device.passkey as Passkey
  const rp = rpId()
  const rec = new Reconnector<WebRunner, TunnelConn>({
    dial: () => session.tailcat.dial(target.token, session.device.nodePriv),
    discard: conn => conn.close(),
    connect: (conn, onLost) =>
      connect(
        conn,
        target.host_key,
        target.user,
        session.keyLine,
        async challenge => {
          try {
            return await signed(session, assertWith(passkey, rp, challenge))
          } catch (e) {
            throw new Error((e as Error).message === 'declined' ? 'passkey_declined' : (e as Error).message)
          }
        },
        onLost,
      ),
    resume: async () => {},
    show: view => {
      if (view.kind === 'up') {
        everUp = true
        into.up(target.id)
      } else if (view.kind === 'connecting') into.connecting(target.id)
      else if (view.kind === 'failed') into.unreachable(target.id, connectFailure(view.word, target.label), 'Reconnect')
      else into.unreachable(target.id, runnerStopped(target.label), 'Reconnect')
    },
    now: () => Date.now(),
  })
  into.add(target.id, target.label, depsOf(rec))
  others.push(rec)
  const attempt = async () => {
    into.connecting(target.id)
    let dialed: TunnelConn | null = null
    try {
      dialed = await session.tailcat.dial(target.token, session.device.nodePriv)
      await rec.open(dialed)
    } catch (e) {
      dialed?.close()
      into.unreachable(target.id, connectFailure((e as Error).message, target.label), 'Connect')
    }
  }
  retries.set(target.id, () => void (everUp ? rec.reconnect() : attempt()))
  if (automatically) turn = turn.then(attempt)
  else into.unreachable(target.id, runnerStopped(target.label), 'Connect')
}

/// The tab is visible again or the browser is online: ask the runner now
/// rather than wait out the probe interval.
function recheckLink(): void {
  void link?.recheck()
  for (const rec of others) void rec.recheck()
}

async function addAgain(session: Session): Promise<void> {
  link = null
  turn = Promise.resolve()
  stopNeeds()
  panes?.dispose()
  panes = null
  dropSecret()
  delete session.device.runners
  delete session.device.offer
  delete session.device.offerShownAt
  await saveDevice(session.device)
  await keepCopy(session)
  void enroll(session)
}

// MARK: Boot

/// Sign Out stops queued copies, forgets local runners, and reports an
/// unconfirmed shared-copy delete before leaving the page.
function signOutForgets(session: () => Session | null): void {
  const form = document.querySelector<HTMLFormElement>('form[action="/sign-out"]')
  if (!form) return
  isSigningOut = installSignOut(form, $<HTMLDialogElement>('sign-out'), {
    session,
    call: relayCopy,
    forget: forgetDevice,
    dropSecret,
  })
}

/// The session boot made, for Sign Out.
let signedIn: Session | null = null

/// Sign Out and Start Again, from the start screen: the header's Sign Out.
function signOut(): void {
  document.querySelector<HTMLFormElement>('form[action="/sign-out"]')?.requestSubmit()
}

const reload = { label: 'Reload', run: () => location.reload() }

let webLoading: ReturnType<typeof init> | undefined
const loadWeb = () => webLoading ??= init({ module_or_path: __WASM__.web })

async function boot(): Promise<void> {
  $('reload').addEventListener('click', () => location.reload())
  $('show-terminals').addEventListener('click', () => showTab('terminals'))
  $('show-needs').addEventListener('click', () => showTab('needs'))
  $('show-plan').addEventListener('click', () => showTab('plan'))
  $('bar-count').addEventListener('click', () => showTab('needs'))
  window.addEventListener('focus', () => { void checkVersion(); recheckLink() })
  window.addEventListener('pageshow', recheckLink)
  document.addEventListener('keydown', event => {
    if ($('work').hidden || event.key !== 'Escape' || event.defaultPrevented || document.querySelector('dialog:modal') ||
        event.target instanceof Element && event.target.closest('.xterm, .chat-input')) return
    layout?.run('close_page')
  })
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') recheckLink()
  })
  window.addEventListener('online', recheckLink)
  const presentRecovery: RecoveryPresenter = async (count, current) => {
    await loadWeb()
    return JSON.parse(recoveryView(count, current)) as RecoveryView
  }
  const copies = new SavedCopies($('saved-copies') as HTMLDialogElement, relayCopy, () => {
    const passkey = signedIn?.device.passkey
    return passkey ? b64url(passkey.id) : undefined
  }, presentRecovery)
  $('show-saved-copies').addEventListener('click', () => void copies.open())
  signOutForgets(() => signedIn)
  $('copy-note-close').addEventListener('click', () => ($('copy-note').hidden = true))

  // Nothing below has a deadline of its own, so the screen says which step
  // the start is on, counts the seconds, and offers a way forward when it is
  // slow (start.ts, ov-869).
  const watch = new StartWatch(screen => status(screen.title, screen.detail, screen.slow ? reload : undefined, screen.slow ? signOut : undefined, 'Sign Out and Start Again'))
  try {
    await start(watch)
  } catch (error) {
    console.error('Far Cooler couldn’t start', error)
    watch.done()
    status('Far Cooler couldn’t start in this browser', 'Reload the page to try again.', reload, signOut, 'Sign Out and Start Again')
  }
}

async function start(watch: StartWatch): Promise<void> {
  /// A screen of its own, which ends the start screen.
  const fail = (title: string, detail: string, action?: { label: string; run: () => void }) => {
    watch.done()
    status(title, detail, action)
  }
  // Both modules download while the account is checked: after a deploy they
  // are the longest wait, and neither depends on who is signed in.
  const loading = Promise.all([loadWeb(), loadTailcat(__WASM__.tailcat)])
  loading.catch(() => {})
  useNeedsYouPresenter(needsYouView)
  useConversationWords(planView)

  // Which account is signed in decides which device this browser may use, so
  // nothing starts without it.
  watch.at('account')
  const me = await relay('directory/me').catch(() => null)
  const account = typeof me?.user?.id === 'string' && me.user.id ? (me.user.id as string) : null
  if (typeof me?.user?.email === 'string') $('who').textContent = me.user.email
  if (!account) return fail('Far Cooler couldn’t check your account', 'Reload the page to try again.', reload)

  watch.at('load')
  let tailcat: Tailcat
  try {
    ;[, tailcat] = await loading
  } catch {
    return fail('Far Cooler couldn’t start in this browser', 'Reload the page to try again.', reload)
  }

  if (!canUsePasskeys()) return fail('This browser can’t use passkeys', 'Far Cooler signs in to your runners with a passkey. Use a current version of Chrome, Safari, Edge or Firefox.')
  watch.at('settings')
  let kept: { device: Device; migrated: boolean }
  try {
    kept = await loadDevice(account, () => tailcat.mintNodeKey())
  } catch {
    return fail('This browser can’t keep Far Cooler’s settings', 'Far Cooler keeps this browser’s runners in this site’s storage. Allow site data for this site, then reload.')
  }
  const { device, migrated } = kept
  const email = typeof me?.user?.email === 'string' ? (me.user.email as string) : ''
  const session: Session = { device, keyLine: device.passkey ? keyLineOf(device.passkey) : '', email, user: await userHandle(account), tailcat }
  signedIn = session
  // Nothing kept here, as after the browser cleared this site's storage: the
  // account's sealed copies, if it has any, bring it back with one touch.
  const vaults = device.passkey ? [] : await listCopies()
  watch.done()
  if (!device.passkey && vaults.length) restoreScreen(session, vaults, migrated)
  else if (!device.passkey) passkeyScreen(session, migrated)
  else if (device.runners?.length) void work(session)
  else void enroll(session)
}

void boot()
