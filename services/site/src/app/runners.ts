/// Runners, their keys, the account's device keys and a team's audit log
/// (ov-562, team directory phase 3 lane B).
///
/// Read-only: nothing here posts, revokes or grants. Every page is a link away
/// from the team page, and every value that came from the directory (runner
/// names, key comments, device labels, emails, audit details) goes through
/// `text()`. Wording is plain: a code the relay sends (a key's label, a reason,
/// an audit action) is looked up here and never shown as it came, so a code
/// this page doesn't know reads as a generic sentence rather than raw text.

import { CHEVRON, document, email, message, roleName, text, type Viewer } from './pages'

export interface TeamRef {
  id: string
  name: string
  personal: boolean
  role: string
}

export interface RunnerRow {
  id: string
  name: string
  lastSeenAt: number | null
  version: string | null
  unclaimed: boolean
  access: { user: string; email: string | null; role: string; grantedAt: number }[]
}

export interface KeyRow {
  fingerprint: string
  comment: string
  inside: boolean
  scope: string
  file: string
  firstSeenAt: number
  label: string
  reason: string
  user: string | null
}

export interface KeyReport {
  reportedAt: number | null
  lastSeenAt: number | null
  error: string | null
  total: number
  unparsedSinceBaseline: number
  keys: KeyRow[]
}

export interface AuditRow {
  id: number
  actor: string | null
  action: string
  subject: string | null
  detail: any
  at: number
}

export interface DeviceRow {
  id: string
  label: string
  keyAFingerprint: string
  state: string
  createdAt: number | null
  revokedAt: number | null
  /// When it was trusted on first use; later than `createdAt` for one trusted
  /// again because none of the account's devices was confirmed (R-75).
  trustedAt?: number | null
  /// Whether that trust was R-75's reset, a new device's included, rather than
  /// the account's first device.
  trustReset?: boolean
}

const teamPath = (team: TeamRef) => `/app/teams/${encodeURIComponent(team.id)}`
const runnersPath = (team: TeamRef) => `${teamPath(team)}/runners`
const runnerPath = (team: TeamRef, runner: string) => `${runnersPath(team)}/${encodeURIComponent(runner)}`

const stamp = new Intl.DateTimeFormat('en-US', { month: 'short', day: 'numeric', year: 'numeric', hour: 'numeric', minute: '2-digit', timeZone: 'UTC', timeZoneName: 'short' })
const day = new Intl.DateTimeFormat('en-US', { month: 'short', day: 'numeric', year: 'numeric', timeZone: 'UTC' })

/// A time that might not be one (a value past what Date holds would throw).
function when(at: unknown, format: Intl.DateTimeFormat): string {
  if (typeof at !== 'number') return ''
  const date = new Date(at)
  return Number.isNaN(date.getTime()) ? '' : format.format(date)
}
export const dayOf = (at: unknown) => when(at, day)
export const stampOf = (at: unknown) => when(at, stamp)

const RUNNER_ROLES: Record<string, string> = { owner: 'Owner', admin: 'Admin', read: 'Read', control: 'Control', shell: 'Shell' }
const RUNNER_ROLE_CAN: Record<string, string> = {
  owner: 'Everything an admin can, and the runner is theirs.',
  admin: 'Manages who else has access.',
  shell: 'A full shell on the runner.',
  control: 'Starts and steers agents.',
  read: 'Looks, but doesn’t change anything.',
}
const runnerRole = (role: string) => (Object.hasOwn(RUNNER_ROLES, role) ? RUNNER_ROLES[role] : 'Access')

/// A device label as a screen may quote it, the same rule as the apps' Device
/// Keys list (`DeviceKeyRow.shownLabel`) and the relay's notices
/// (`directory/common.ts` `withoutUnshown`), all four against one table,
/// `test/fixtures/device-labels.json` (ov-560 R1): no format or control
/// characters (bidi controls, zero-width characters, line breaks), no letters
/// that draw as nothing, no opening or closing quotes or double-quote
/// lookalikes, the apostrophe ’ only once between two letters, runs of space
/// made one, at most 40 characters, and "a device" when nothing is left. Any
/// session on the account chose the label, an impostor's included, so a row
/// never shows it without its key words and date.
export function shownLabel(label: unknown): string {
  const dropped = String(label ?? '').replace(
    /(?!\u2019)[\p{Cf}\p{Cc}\p{Zl}\p{Zp}\p{Pi}\p{Pf}"`\u02ee\u05f4\u115f\u201e\u2033\u2036\u2800\u3003\u3164\uff02\u301d-\u301f\u275d\u275e]/gu,
    '',
  )
  const letter = /^\p{L}$/u
  const plain = dropped
    .replace(/\u2019+/g, (run, at: number) =>
      letter.test(Array.from(dropped.slice(0, at)).pop() ?? '') && letter.test(Array.from(dropped.slice(at + run.length))[0] ?? '')
        ? '\u2019'
        : '',
    )
    .replace(/\s+/g, ' ')
    .trim()
  const cut = [...plain].slice(0, 40).join('').trim()
  return cut || 'a device'
}

/// A fingerprint as the apps spell the key words (`KeyFingerprint.short`):
/// `SHA256:` and the first and last eight characters.
export function keyWords(fingerprint: unknown): string {
  if (typeof fingerprint !== 'string' || !fingerprint) return ''
  const prefix = 'SHA256:'
  if (!fingerprint.startsWith(prefix)) return fingerprint
  const body = fingerprint.slice(prefix.length)
  return body.length > 20 ? `${prefix}${body.slice(0, 8)}…${body.slice(-8)}` : fingerprint
}

const crumb = (href: string, label: string) => `<a class="back" href="${text(href)}">‹ ${text(label)}</a>`
const mono = (value: string) => `<span class="mono">${text(value)}</span>`

/// The page for a failure to read one team's runners or log.
export function noAccessPage(what: 'runners' | 'audit', viewer: Viewer): string {
  return what === 'runners'
    ? message('Runners aren’t visible to you', 'Billing members can see a team, but not its runners. Ask a team admin if you need to.', '<a class="button" href="/app">My Teams</a>', undefined, viewer)
    : message('Only admins can see the audit log', 'Ask a team admin if you need to know what changed.', '<a class="button" href="/app">My Teams</a>', undefined, viewer)
}

export function runnerNotFoundPage(team: TeamRef, viewer: Viewer): string {
  return message('Runner not found', 'It may have been removed from the team, or the link is wrong.', `<a class="button" href="${text(runnersPath(team))}">All Runners</a>`, undefined, viewer)
}

function who(access: RunnerRow['access'], user: string | null): string {
  const found = access.find(a => a.user === user)
  return found?.email ? found.email : ''
}

export function runnersPage(team: TeamRef, runners: RunnerRow[], viewer: Viewer): string {
  const rows = runners
    .map(runner => {
      const owner = runner.access.find(a => a.role === 'owner')
      const people = runner.access.length === 1 ? '1 person has access' : `${runner.access.length} people have access`
      const seen = stampOf(runner.lastSeenAt)
      const meta = [
        runner.unclaimed ? 'Nobody has claimed it yet' : owner?.email ? `Owned by ${owner.email}` : 'Owned by someone without an email on file',
        people,
        seen ? `Last heard from ${seen}` : 'Hasn’t been heard from yet',
      ]
        .map(part => text(part))
        .join(' · ')
      return `<li><a class="row" href="${text(runnerPath(team, runner.id))}"><span class="main"><span class="name">${text(runner.name)}</span><span class="meta">${meta}</span></span>${CHEVRON}</a></li>`
    })
    .join('\n')
  return document(
    `Runners – ${team.name} – Far Cooler`,
    `<main class="wrap page">
${crumb(teamPath(team), team.name)}
<h1>Runners</h1>
<p class="lede">The machines ${text(team.name)}’s agents run on, and who can reach each one.</p>
${runners.length ? `<ul class="list">\n${rows}\n</ul>` : '<p class="note">This team has no runners yet. Add one from Far Cooler on your Mac.</p>'}
${team.role === 'admin' ? `<h2>History</h2>\n<ul class="list"><li><a class="row" href="${text(teamPath(team))}/audit"><span class="main"><span class="name">Audit Log</span><span class="meta">Who changed what, newest first.</span></span>${CHEVRON}</a></li></ul>` : ''}
</main>`,
    viewer,
  )
}

const KEY_LABELS: Record<string, string> = {
  known: 'Known',
  unknown: 'Unrecognized',
  orphaned: 'No longer allowed',
  planted: 'Added outside Far Cooler',
}
const KEY_REASONS: Record<string, string> = {
  granted: 'It belongs to a teammate’s device, and their role covers it.',
  no_device: 'No one’s device has registered this key.',
  revoked_device: 'The device it belonged to was removed.',
  uncertified_device: 'Its device hasn’t been confirmed yet.',
  not_in_team: 'Its owner isn’t in this team anymore.',
  no_grant: 'Its owner doesn’t hold a role on this runner.',
  outside_fence: 'Its owner holds a role here, but this line sits outside the part of the file Far Cooler manages.',
  exceeds_grant: 'The line allows more than its owner’s role does.',
  unmanaged_line: 'Its owner holds a role here, but Far Cooler didn’t write this line.',
  ca_key: 'It trusts every certificate the key signs, so it can let in people this page can’t list.',
}
const SCOPES: Record<string, string> = {
  read: 'Read',
  control: 'Control',
  host_admin: 'Admin',
  shell: 'Shell',
  ca: 'Certificate authority',
  other: 'Not a Far Cooler line',
}
const KEY_ERRORS: Record<string, string> = {
  damaged: 'The runner’s keys file is damaged, so the last good list is shown.',
  refused: 'The runner wasn’t allowed to read its keys file, so the last good list is shown.',
  too_large: 'The runner’s keys file is too large to read in full.',
  unreadable: 'The runner couldn’t read its keys file, so the last good list is shown.',
}

const label = (key: KeyRow) => (Object.hasOwn(KEY_LABELS, key.label) ? key.label : 'unknown')

/// One key: its comment, its key words, what it is and why, and when it appeared.
function keyRow(key: KeyRow, access: RunnerRow['access']): string {
  const kind = label(key)
  const owner = who(access, key.user)
  const reason = Object.hasOwn(KEY_REASONS, key.reason) ? KEY_REASONS[key.reason] : ''
  const scope = Object.hasOwn(SCOPES, key.scope) ? SCOPES[key.scope] : SCOPES.other
  const where = key.file === 'authorized_keys2' ? 'In authorized_keys2' : key.inside ? 'In Far Cooler’s part of the file' : 'Outside Far Cooler’s part of the file'
  const meta = [
    `<span class="${kind === 'known' ? '' : kind === 'planted' ? 'danger' : 'attn'}">${text(KEY_LABELS[kind])}</span>`,
    owner ? text(`Registered to ${owner}`) : '',
    text(scope),
    text(where),
    key.firstSeenAt ? text(`First seen ${dayOf(key.firstSeenAt)}`) : '',
  ].filter(Boolean)
  return `<li><div class="row"><span class="main"><span class="name">${key.comment ? text(key.comment) : '<span class="dim">No comment</span>'}</span><span class="meta">${meta.join(' · ')}</span><span class="meta">${mono(keyWords(key.fingerprint))}</span>${reason ? `<span class="meta">${text(reason)}</span>` : ''}</div></li>`
}

const ORDER = ['planted', 'orphaned', 'unknown', 'known']

function keysSection(report: KeyReport | null, access: RunnerRow['access']): string {
  if (!report) return '<p class="note">Couldn’t load this runner’s keys. Try again in a moment.</p>'
  const seen = report.lastSeenAt === null ? '' : stampOf(report.lastSeenAt)
  const error = report.error && Object.hasOwn(KEY_ERRORS, report.error) ? `<p class="banner bad" role="status">${text(KEY_ERRORS[report.error])}</p>` : ''
  if (report.reportedAt === null) {
    return `${error}<p class="note">This runner hasn’t reported its keys yet.${seen ? ` It last checked in ${text(seen)}.` : ''}</p>`
  }
  const sorted = [...report.keys].sort((a, b) => ORDER.indexOf(label(a)) - ORDER.indexOf(label(b)))
  const flagged = sorted.filter(k => label(k) !== 'known')
  const known = sorted.filter(k => label(k) === 'known')
  const cut = report.total > report.keys.length ? `<p class="note">The runner read ${report.total} keys. This list shows the ${report.keys.length} it sent.</p>` : ''
  const n = report.unparsedSinceBaseline
  const [lines, was, it] = n === 1 ? ['1 line', 'wasn’t', 'it'] : [`${n} lines`, 'weren’t', 'them']
  const unparsed = n > 0 ? `<p class="banner bad" role="status">${lines} in the keys file can’t be read, and ${was} there at the first report. sshd may still obey ${it}.</p>` : ''
  const summary = flagged.length
    ? `${flagged.length === 1 ? '1 key needs' : `${flagged.length} keys need`} a look.`
    : report.keys.length
      ? 'Every key matches a teammate.'
      : 'This runner has no keys that let anyone in.'
  return `${error}${unparsed}<p class="lede">${text(summary)} Reported ${text(stampOf(report.reportedAt))}.</p>
${flagged.length ? `<ul class="list keys">\n${flagged.map(k => keyRow(k, access)).join('\n')}\n</ul>` : ''}
${known.length ? `<details class="fold"><summary>${known.length === 1 ? '1 known key' : `${known.length} known keys`}</summary>\n<ul class="list keys">\n${known.map(k => keyRow(k, access)).join('\n')}\n</ul></details>` : ''}
${cut}
<p class="note">These are the keys the runner says it has. Anyone with a shell on the runner can change what it says, so this catches mistakes and drift, and doesn’t prove a runner is clean.</p>`
}

export interface Names {
  users: Map<string, string>
  runners: Map<string, string>
}

const nameOfUser = (names: Names, user: unknown) => (typeof user === 'string' && names.users.get(user)) || 'Someone'
const nameOfRunner = (names: Names, id: unknown) => (typeof id === 'string' && names.runners.get(id)) || 'a runner'
const str = (value: unknown, max = 80) => (typeof value === 'string' ? [...value].slice(0, max).join('') : '')
const roleOr = (value: unknown, fallback: string) => (typeof value === 'string' && Object.hasOwn(RUNNER_ROLES, value) ? RUNNER_ROLES[value] : Object.hasOwn(({ admin: 1, member: 1, billing: 1 }), String(value)) ? roleName(String(value)) : fallback)

/// What one audit line says, as a sentence in plain words. Returns plain
/// text; the caller escapes it. An action this page doesn't know reads as
/// "Something changed", never as the relay's code.
export function auditSentence(entry: AuditRow, names: Names): string {
  const actor = entry.actor?.startsWith('runner:') ? nameOfRunner(names, entry.actor.slice(7)) : nameOfUser(names, entry.actor)
  const subject = nameOfUser(names, entry.subject)
  const d = entry.detail && typeof entry.detail === 'object' ? entry.detail : {}
  switch (entry.action) {
    case 'team.created':
      return `${actor} made the team${str(d.name) ? ` ${str(d.name)}` : ''}.`
    case 'member.joined':
      return `${subject} joined as ${roleOr(d.role, 'a member').toLowerCase()}.`
    case 'member.left':
      return `${subject} left the team.`
    case 'member.removed':
      return `${actor} removed ${subject} from the team.`
    case 'member.readmitted':
      return `${actor} readmitted ${subject}.`
    case 'member.role_changed':
      return `${actor} changed ${subject} from ${roleOr(d.from, 'a role')} to ${roleOr(d.role, 'a role')}.`
    case 'invite.created':
      return `${actor} made an invite link.`
    case 'invite.revoked':
      return `${actor} revoked an invite link.`
    case 'runner.registered':
      return `${actor} added the runner ${str(d.name) || 'a runner'}.`
    case 'runner.removed':
      return `${actor} removed the runner ${str(d.name) || 'a runner'}.`
    case 'runner.claimed':
      return `${actor} moved ${str(d.name) || nameOfRunner(names, entry.subject)} into this team.`
    case 'runner.claimed_away':
      return `${actor} moved ${nameOfRunner(names, entry.subject)} to another team.`
    case 'runner.token_rotated':
      return `${actor} signed ${nameOfRunner(names, entry.subject)} in again.`
    case 'runner.role_removed':
      return `${actor} took ${subject}’s ${roleOr(d.role, 'access').toLowerCase()} access to ${nameOfRunner(names, d.runner)} away.`
    case 'runner.inventory_error':
      return `${nameOfRunner(names, entry.subject)} couldn’t read its keys file.`
    case 'runner.inventory_recovered':
      return `${nameOfRunner(names, entry.subject)} can read its keys file again.`
    case 'runner.key_appeared':
      return `A key appeared on ${nameOfRunner(names, d.runner)}${str(d.comment, 60) ? `: ${str(d.comment, 60)}` : ''}.`
    case 'runner.key_disappeared':
      return `A key disappeared from ${nameOfRunner(names, d.runner)}${str(d.comment, 60) ? `: ${str(d.comment, 60)}` : ''}.`
    default:
      return 'Something changed.'
  }
}

/// The audit entries about one runner: its own lines, and the key and role
/// lines that name it.
export function aboutRunner(entries: AuditRow[], runner: string): AuditRow[] {
  return entries.filter(e => e.subject === runner || (e.detail && typeof e.detail === 'object' && e.detail.runner === runner))
}

function entryRows(entries: AuditRow[], names: Names): string {
  return entries
    .map(e => {
      const actor = e.actor?.startsWith('runner:') ? nameOfRunner(names, e.actor.slice(7)) : nameOfUser(names, e.actor)
      return `<li><div class="row"><span class="main"><span class="name">${text(auditSentence(e, names))}</span><span class="meta">${text(actor)} · ${text(stampOf(e.at))}</span></span></div></li>`
    })
    .join('\n')
}

export interface RunnerView {
  team: TeamRef
  runner: RunnerRow
  keys: KeyReport | null
  /// Null when the viewer isn't a team admin: the log is theirs alone.
  activity: AuditRow[] | 'failed' | null
  names: Names
  viewer: Viewer
}

export function runnerPage(view: RunnerView): string {
  const { team, runner, viewer } = view
  const access = runner.access.length
    ? `<ul class="list">\n${runner.access
        .map(
          a =>
            `<li><div class="row"><span class="avatar" aria-hidden="true">${text((a.email ?? '?').trim().charAt(0) || '?')}</span><span class="main"><span class="name">${a.email ? email(a.email) : 'No email on file'}</span><span class="meta">${text(RUNNER_ROLE_CAN[a.role] ?? '')}${dayOf(a.grantedAt) ? text(`${RUNNER_ROLE_CAN[a.role] ? ' ' : ''}Since ${dayOf(a.grantedAt)}.`) : ''}</span></span><span class="end">${runnerRole(a.role)}</span></div></li>`,
        )
        .join('\n')}\n</ul>`
    : '<p class="note">No one holds a role on this runner yet.</p>'
  const unclaimed = runner.unclaimed ? '<p class="note">Nobody has claimed this runner, so no one can give anyone access to it yet. Someone with a shell on it can claim it from Far Cooler.</p>' : ''
  const seen = stampOf(runner.lastSeenAt)
  const more = `<p class="note"><a href="${text(teamPath(team))}/audit">See the whole audit log</a></p>`
  const activity =
    view.activity === null
      ? '<p class="note">Team admins can see what changed on this runner.</p>'
      : view.activity === 'failed'
        ? `<p class="note">Couldn’t load activity. Try again in a moment.</p>\n${more}`
        : (view.activity.length
        ? `<ul class="list">\n${entryRows(view.activity.slice(0, 20), view.names)}\n</ul>`
        : '<p class="note">Nothing about this runner is in the team’s latest 100 changes.</p>') + `\n${more}`
  return document(
    `${runner.name} – Far Cooler`,
    `<main class="wrap page">
${crumb(runnersPath(team), 'Runners')}
<h1>${text(runner.name)}</h1>
<p class="lede">${text([team.name, seen ? `Last heard from ${seen}` : 'Hasn’t been heard from yet', runner.version ? `Version ${runner.version}` : ''].filter(Boolean).join(' · '))}</p>
<h2>Access</h2>
${access}
${unclaimed}
<h2>Keys</h2>
${keysSection(view.keys, runner.access)}
<h2>Projects</h2>
<p class="note">A runner’s projects stay on the runner, so the web can’t list them yet. Open this runner in Far Cooler on your Mac or iPhone to see them.</p>
<h2>Activity</h2>
${activity}
</main>`,
    viewer,
  )
}

export function auditPage(team: TeamRef, entries: AuditRow[], names: Names, older: number | null, paged: boolean, viewer: Viewer): string {
  return document(
    `Audit Log – ${team.name} – Far Cooler`,
    `<main class="wrap page">
${crumb(teamPath(team), team.name)}
<h1>Audit Log</h1>
<p class="lede">What changed in ${text(team.name)}, newest first. Only admins can see this.</p>
${entries.length ? `<ul class="list">\n${entryRows(entries, names)}\n</ul>` : `<p class="note">${paged ? 'No older entries.' : 'Nothing has been recorded yet.'}</p>`}
${older === null ? '' : `<div class="actions left"><a class="button" href="${text(teamPath(team))}/audit?before=${String(Math.trunc(older))}">Older</a></div>`}
</main>`,
    viewer,
  )
}

const DEVICE_STATES: Record<string, string> = {
  trusted: 'Confirmed as the first device on your account',
  certified: 'Confirmed by another of your devices',
  pending: 'Not confirmed yet. Scan its code from one of your confirmed devices.',
  revoked: 'Removed. Its key can’t be added back.',
}

/// What a row says of its state. `anyConfirmed` is whether any of the
/// account's devices is: with none, the next to open Far Cooler or sign in is
/// confirmed (R-75, ov-561), the same copy as the apps' Device Keys list.
function deviceState(d: DeviceRow, anyConfirmed: boolean): string {
  if (d.state === 'trusted' && d.trustReset === true) {
    return 'Confirmed when it signed in, since none of your devices was confirmed'
  }
  if (d.state === 'pending' && !anyConfirmed) {
    return 'Not confirmed. None of your devices is confirmed, so whichever opens Far Cooler or signs in next becomes confirmed. Do that on the device you want to keep, then confirm the others from it.'
  }
  return Object.hasOwn(DEVICE_STATES, d.state) ? DEVICE_STATES[d.state] : DEVICE_STATES.pending
}

function deviceRow(d: DeviceRow, anyConfirmed: boolean): string {
  const words = keyWords(d.keyAFingerprint)
  const added = dayOf(d.createdAt) ? `Added ${dayOf(d.createdAt)}` : ''
  const removed = d.state === 'revoked' && dayOf(d.revokedAt) ? `Removed ${dayOf(d.revokedAt)}` : ''
  const state = deviceState(d, anyConfirmed)
  const meta = [words ? mono(words) : '', added ? text(added) : '', removed ? text(removed) : ''].filter(Boolean).join(' · ')
  return `<li><div class="row"><span class="main"><span class="name">${text(shownLabel(d.label))}</span><span class="meta">${meta}</span><span class="meta">${text(state)}</span></span></div></li>`
}

export function devicesPage(devices: DeviceRow[], viewer: Viewer): string {
  const live = devices.filter(d => d.state !== 'revoked')
  const gone = devices.filter(d => d.state === 'revoked')
  const anyConfirmed = devices.some(d => d.state === 'trusted' || d.state === 'certified')
  const section = (list: DeviceRow[]) => list.map(d => deviceRow(d, anyConfirmed)).join('\n')
  return document(
    'My Devices – Far Cooler',
    `<main class="wrap page">
${crumb('/app', 'My Teams')}
<h1>My Devices</h1>
<p class="lede">Each device that signs in to Far Cooler adds its key here, so your team’s admins can add it to runners as yours.</p>
${live.length ? `<ul class="list">\n${section(live)}\n</ul>` : '<p class="note">No device keys yet. Each device adds its key when it signs in.</p>'}
${gone.length ? `<h2>Removed</h2>\n<ul class="list">\n${section(gone)}\n</ul>` : ''}
<p class="note">To tell two devices apart, compare the words under each with the ones that device shows for itself in Far Cooler’s settings. To remove a device you don’t recognize, open those settings on your Mac or iPhone.</p>
<p class="note">If you think someone else has your session, <a href="/app/sign-out-everywhere">Sign Out Everywhere…</a> ends every session on your account.</p>
</main>`,
    viewer,
  )
}
