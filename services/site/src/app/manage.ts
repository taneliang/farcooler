/// The team page and the pages an admin manages it from (ov-524).
///
/// Server-rendered like the rest: a link opens a confirm page, and the confirm
/// page's button is a form that POSTs (`forms.ts`). No script. Every value
/// that came from the directory goes through `text()`.

import type { NewInvite } from './session'
import { banner, CHEVRON, document, email, joinedOn, message, roleName, text, type Member, type Viewer } from './pages'

export interface TeamInfo {
  id: string
  name: string
  personal: boolean
}

export interface Invite {
  id: string
  role: string
  domain: string | null
  maxUses: number
  uses: number
  createdBy: string
  createdAt: number
  expiresAt: number
  revokedAt: number | null
}

export interface Removed {
  user: string
  email: string | null
  removedAt: number
}

export interface Runner {
  id: string
  name: string
  access: { user: string; email: string | null; role: string }[]
}

const day = new Intl.DateTimeFormat('en-US', { month: 'short', day: 'numeric', year: 'numeric', timeZone: 'UTC' })
function on(at: unknown): string {
  if (typeof at !== 'number') return ''
  const date = new Date(at)
  return Number.isNaN(date.getTime()) ? '' : day.format(date)
}

const csrf = (viewer: Viewer) => `<input type="hidden" name="csrf" value="${text(viewer.csrf)}">`
const post = (action: string, viewer: Viewer, inner: string, cls = '') =>
  `<form${cls ? ` class="${cls}"` : ''} method="post" action="${text(action)}">${csrf(viewer)}${inner}</form>`

const teamPath = (team: TeamInfo) => `/app/teams/${encodeURIComponent(team.id)}`
const memberPath = (team: TeamInfo, user: string) => `${teamPath(team)}/members/${encodeURIComponent(user)}`

const ROLE_CAN: Record<string, string> = {
  admin: 'Manages people, roles and invite links.',
  member: 'Sees the team and who’s in it.',
  billing: 'Sees the team, but not who’s in it.',
}
const ARTICLE: Record<string, string> = { admin: 'an admin', member: 'a member', billing: 'a billing member' }
export const article = (role: string) => (Object.hasOwn(ARTICLE, role) ? ARTICLE[role] : 'a member')

/// An invite that can still be used.
export function isOpen(invite: Invite, now: number): boolean {
  return invite.revokedAt === null && invite.expiresAt > now && invite.uses < invite.maxUses
}

function inviteRows(team: TeamInfo, invites: Invite[], viewer: Viewer, now: number): string {
  const open = invites.filter(i => isOpen(i, now))
  if (!open.length) return '<p class="note">No invite links are open.</p>'
  const rows = open
    .map(invite => {
      const left = invite.maxUses - invite.uses
      const meta = [`Expires ${on(invite.expiresAt)}`, invite.domain ? `Only @${invite.domain}` : '']
        .filter(Boolean)
        .map(part => text(part))
        .join(' · ')
      return `<li><div class="row"><span class="main"><span class="name">${invite.role === 'billing' ? 'Billing' : 'Member'} link · ${left} of ${invite.maxUses} left</span><span class="meta">${meta}</span></span>${post(`${teamPath(team)}/invites/${encodeURIComponent(invite.id)}/revoke`, viewer, '<button class="plain danger" type="submit">Revoke</button>', 'end')}</div></li>`
    })
    .join('\n')
  return `<ul class="list">\n${rows}\n</ul>`
}

function inviteForm(team: TeamInfo, viewer: Viewer): string {
  return `<details class="make">
<summary class="button">New Invite Link</summary>
${post(
  `${teamPath(team)}/invites`,
  viewer,
  `<div class="fields">
<label class="field"><span>Role</span><select name="role"><option value="member" selected>Member</option><option value="billing">Billing</option></select></label>
<label class="field"><span>Expires After</span><select name="days"><option value="1">1 day</option><option value="3">3 days</option><option value="7" selected>7 days</option><option value="14">14 days</option><option value="30">30 days</option></select></label>
<label class="field"><span>People Who Can Use It</span><input type="number" name="uses" value="50" min="1" max="500" inputmode="numeric" required></label>
<label class="field"><span>Only This Email Domain</span><input type="text" name="domain" placeholder="Optional, like example.com" autocomplete="off" autocapitalize="none" spellcheck="false"></label>
</div>
<div class="actions"><button class="button" type="submit">Make Link</button></div>`,
  'card',
)}
</details>`
}

export interface TeamView {
  team: TeamInfo
  members: Member[]
  removed: Removed[]
  invites: Invite[]
  self: string
  /// The viewer's role in this team.
  role: string
  viewer: Viewer
  notice: string | null
  now: number
}

/// Only an admin of a team with other people in it gets controls.
export const canManage = (view: Pick<TeamView, 'team' | 'role'>) => view.role === 'admin' && !view.team.personal

export function teamPage(view: TeamView): string {
  const { team, members, viewer } = view
  const manage = canManage(view)
  const rows = members
    .map(member => {
      const label = member.email === null ? 'No email on file' : email(member.email)
      const initial = (member.email ?? '?').trim().charAt(0) || '?'
      const meta = [member.user === view.self ? 'You' : '', joinedOn(member.joinedAt)].filter(Boolean).join(' · ')
      const inner = `<span class="avatar" aria-hidden="true">${text(initial)}</span><span class="main"><span class="name">${label}</span>${meta ? `<span class="meta">${meta}</span>` : ''}</span><span class="end">${roleName(member.role)}</span>`
      return manage
        ? `<li><a class="row" href="${text(memberPath(team, member.user))}">${inner}${CHEVRON}</a></li>`
        : `<li><div class="row">${inner}</div></li>`
    })
    .join('\n')
  const count = members.length === 1 ? '1 person' : `${members.length} people`
  const removed = manage && view.removed.length
    ? `<h2>Removed</h2>
<ul class="list members">
${view.removed
  .map(
    r =>
      `<li><div class="row"><span class="avatar" aria-hidden="true">${text((r.email ?? '?').trim().charAt(0) || '?')}</span><span class="main"><span class="name">${r.email === null ? 'No email on file' : email(r.email)}</span><span class="meta">Removed ${text(on(r.removedAt))}</span></span><a class="end" href="${text(memberPath(team, r.user))}/readmit">Readmit</a></div></li>`,
  )
  .join('\n')}
</ul>
<p class="note">Readmitting someone makes them a member again. An invite link can’t bring back a person an admin removed.</p>`
    : ''
  return document(
    `${team.name} – Far Cooler`,
    `<main class="wrap page">
<a class="back" href="/app">‹ My Teams</a>
<h1>${text(team.name)}</h1>
<p class="lede">${team.personal ? 'Your personal team. It holds your own runners, and only you are in it.' : count}</p>
${banner(view.notice)}
${
  manage
    ? `<h2>Invite Links</h2>
${inviteForm(team, viewer)}
${inviteRows(team, view.invites, viewer, view.now)}
<h2>People</h2>`
    : ''
}
<ul class="list members">
${rows}
</ul>
${removed}
</main>`,
    viewer,
  )
}

/// The page for one member: their role, and what an admin can do about it.
export function memberPage(view: TeamView, member: Member): string {
  const { team, viewer } = view
  const you = member.user === view.self
  const admins = view.members.filter(m => m.role === 'admin').length
  const only = member.role === 'admin' && admins <= 1
  const choices = ['admin', 'member', 'billing']
    .filter(role => role !== member.role)
    .map(
      role =>
        `<li><a class="row" href="${text(memberPath(team, member.user))}/role/${role}"><span class="main"><span class="name">Make ${roleName(role)}</span><span class="meta">${ROLE_CAN[role]}</span></span>${CHEVRON}</a></li>`,
    )
    .join('\n')
  const remove = `<li><a class="row" href="${text(memberPath(team, member.user))}/remove"><span class="main"><span class="name danger">Remove from Team</span></span>${CHEVRON}</a></li>`
  return document(
    `${member.email ?? 'Member'} – Far Cooler`,
    `<main class="wrap page">
<a class="back" href="${text(teamPath(team))}">‹ ${text(team.name)}</a>
<h1>${member.email === null ? 'No email on file' : email(member.email)}</h1>
<p class="lede">${roleName(member.role)}${you ? ' · You' : ''}${joinedOn(member.joinedAt) ? ` · ${joinedOn(member.joinedAt)}` : ''}</p>
${banner(view.notice)}
${
  only
    ? `<p class="note">${you ? 'You’re' : 'They’re'} the only admin. Make someone else an admin before changing ${you ? 'your' : 'their'} role.</p>`
    : `<h2>Role</h2>
<ul class="list">
${choices}
</ul>
${you ? '' : `<h2>Access</h2>\n<ul class="list">\n${remove}\n</ul>`}`
}
</main>`,
    viewer,
  )
}

/// "Make them an admin?", the step before a role changes.
export function roleConfirmPage(view: TeamView, member: Member, role: string): string {
  const { team, viewer } = view
  const who = member.email ?? 'This person'
  const demoting = member.role === 'admin'
  const you = member.user === view.self
  const lede = you
    ? `You’ll become ${article(role)} of ${team.name}. ${demoting ? 'You’ll no longer be able to manage people, roles or invite links.' : ''}`.trim()
    : role === 'admin'
      ? `${who} will become an admin of ${team.name}. They’ll be able to manage people, roles and invite links.`
      : `${who} will become ${article(role)} of ${team.name}. ${demoting ? 'They’ll no longer be able to manage people, roles or invite links. ' : ''}${role === 'billing' ? 'They’ll see the team, but not who’s in it.' : 'They’ll see the team and who’s in it.'}`
  return message(
    `Make ${roleName(role)}?`,
    lede,
    `${post(`${memberPath(team, member.user)}/role`, viewer, `<input type="hidden" name="role" value="${text(role)}"><button class="button" type="submit">Make ${roleName(role)}</button>`)}<a class="cancel" href="${text(memberPath(team, member.user))}">Cancel</a>`,
    undefined,
    viewer,
  )
}

/// "Remove them?", with what removing does not do. Their keys stay on runners
/// until a runner admin revokes them (R-64): the page says so and names the
/// runners where they hold a role.
export function removeConfirmPage(view: TeamView, member: Member, runners: Runner[]): string {
  const { team, viewer } = view
  const who = member.email ?? 'This person'
  const held = runners
    .map(runner => ({ runner, role: runner.access.find(a => a.user === member.user)?.role }))
    .filter((r): r is { runner: Runner; role: string } => r.role !== undefined)
  const list = held.length
    ? `<p class="fine">They hold a role on:</p>
<ul class="held">
${held.map(h => `<li><span class="name">${text(h.runner.name)}</span> <span class="meta">${text(runnerRole(h.role))}</span></li>`).join('\n')}
</ul>`
    : '<p class="fine">They don’t hold a role on any of this team’s runners.</p>'
  return document(
    'Remove from Team? – Far Cooler',
    `<main class="wrap message">
<h1>Remove from Team?</h1>
<p class="lede">${text(who)} will leave ${text(team.name)}, and an invite link can’t bring them back.</p>
<p class="fine">Their keys stay on runners until a runner admin revokes them. Removing them here doesn’t reach into those runners.</p>
${list}
<div class="actions">${post(`${memberPath(team, member.user)}/remove`, viewer, '<button class="button danger" type="submit">Remove from Team</button>')}<a class="cancel" href="${text(memberPath(team, member.user))}">Cancel</a></div>
</main>`,
    viewer,
  )
}

const RUNNER_ROLES: Record<string, string> = { owner: 'Owner', admin: 'Admin', read: 'Read', control: 'Control', shell: 'Shell' }
const runnerRole = (role: string) => (Object.hasOwn(RUNNER_ROLES, role) ? RUNNER_ROLES[role] : 'Access')

/// The one time an invite link is shown.
export function inviteCreatedPage(invite: NewInvite, link: string, viewer: Viewer): string {
  const team: TeamInfo = { id: invite.team, name: invite.name, personal: false }
  return document(
    'Invite Link – Far Cooler',
    `<main class="wrap page">
<a class="back" href="${text(teamPath(team))}">‹ ${text(team.name)}</a>
<h1>Invite Link</h1>
<p class="lede">Anyone with this link can join ${text(team.name)} as ${article(invite.role)}. Copy it now: it won’t be shown again.</p>
<p class="link" id="link">${text(link)}</p>
<p class="note">It works until ${text(on(invite.expiresAt))}, for ${invite.uses} ${invite.uses === 1 ? 'person' : 'people'}${invite.domain ? `, and only for @${text(invite.domain)} email addresses` : ''}. Revoke it from the team page at any time.</p>
<div class="actions left"><a class="button" href="${text(teamPath(team))}">Done</a></div>
</main>`,
    viewer,
  )
}

/// Where the Invite Link page lands once the link has been shown, or when
/// there never was one to show.
export function inviteShownPage(team: string, viewer: Viewer): string {
  return message(
    'That link was already shown',
    'An invite link is shown once, when it’s made. Make a new one if you need it, and revoke any you no longer want from the team page.',
    `<a class="button" href="${text(`/app/teams/${encodeURIComponent(team)}`)}">Back to Team</a>`,
    undefined,
    viewer,
  )
}

/// "Readmit them?", the step before access comes back.
export function readmitConfirmPage(view: TeamView, who: Removed): string {
  const { team, viewer } = view
  return message(
    'Readmit?',
    `${who.email ?? 'This person'} will be a member of ${team.name} again, and will see the team and who’s in it.`,
    `${post(`${memberPath(team, who.user)}/readmit`, viewer, '<button class="button" type="submit">Readmit</button>')}<a class="cancel" href="${text(teamPath(team))}">Cancel</a>`,
    undefined,
    viewer,
  )
}

export function refusedPage(viewer?: Viewer): string {
  return message(
    'Couldn’t verify this request',
    'It may have come from another site, or the page was open too long. Go back, reload it, and try again.',
    '<a class="button" href="/app">My Teams</a>',
    undefined,
    viewer,
  )
}

export function notAdminPage(viewer: Viewer): string {
  return message('Only admins can do that', 'Ask a team admin to make this change.', '<a class="button" href="/app">My Teams</a>', undefined, viewer)
}
