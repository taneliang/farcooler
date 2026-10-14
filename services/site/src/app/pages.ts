/// The web app's pages, rendered on the Worker as complete HTML.
///
/// No script at all: every page is a plain document, and the content security
/// policy forbids script outright. Every value that came from the directory
/// (team names, emails) goes through `text()`.

import { STYLE } from './style'

export interface Team {
  id: string
  name: string
  personal: boolean
  role: string
}

export interface Member {
  user: string
  email: string | null
  role: string
  joinedAt: number
}

/// Who is signed in, for the bar.
export interface Viewer {
  email: string | null
}

/// Escape for HTML text and double-quoted attributes.
export function text(value: unknown): string {
  return String(value ?? '')
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;')
}

const ROLES: Record<string, string> = { admin: 'Admin', member: 'Member', billing: 'Billing' }
const roleName = (role: string) => (Object.hasOwn(ROLES, role) ? ROLES[role] : 'Member')

/// An email, escaped, that a narrow row breaks after the @ and before each
/// dot rather than mid-word.
function email(value: string): string {
  return text(value).replaceAll('@', '@<wbr>').replaceAll('.', '<wbr>.')
}

const CHEVRON =
  '<svg class="chevron" viewBox="0 0 8 14" aria-hidden="true"><path d="M1 1l6 6-6 6" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>'

const joined = new Intl.DateTimeFormat('en-US', { month: 'short', day: 'numeric', year: 'numeric', timeZone: 'UTC' })

/// "Joined Oct 11, 2026", or nothing for a time that isn't a date: a value
/// past what Date can hold would otherwise throw a RangeError and a 500.
function joinedOn(at: unknown): string {
  if (typeof at !== 'number') return ''
  const date = new Date(at)
  return Number.isNaN(date.getTime()) ? '' : `Joined ${joined.format(date)}`
}

function document(title: string, body: string, viewer?: Viewer): string {
  const account = viewer
    ? `<div class="account">${viewer.email ? `<span class="who">${text(viewer.email)}</span>` : ''}<form method="post" action="/app/sign-out"><button class="plain" type="submit">Sign Out</button></form></div>`
    : ''
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="dark light">
<meta name="robots" content="noindex">
<title>${text(title)}</title>
<style>${STYLE}</style>
</head>
<body>
<header class="bar"><div class="wrap"><a class="mark" href="/app">Far Cooler</a>${account}</div></header>
${body}
</body>
</html>
`
}

/// A page with one message and at most one action, for every state that
/// isn't a list.
function message(title: string, lede: string, action?: string, fine?: string, viewer?: Viewer): string {
  return document(
    `${title} – Far Cooler`,
    `<main class="wrap message">
<h1>${text(title)}</h1>
<p class="lede">${text(lede)}</p>
${action ? `<div class="actions">${action}</div>` : ''}
${fine ? `<p class="fine">${text(fine)}</p>` : ''}
</main>`,
    viewer,
  )
}

const SIGN_IN_BUTTON = '<a class="button" href="/app/sign-in">Sign In</a>'

export function signInPage(): string {
  return message(
    'Your teams, on the web',
    'Sign in to see the teams you’re in and who’s in each one.',
    SIGN_IN_BUTTON,
    'Use the same account as the Far Cooler app on your Mac or iPhone.',
  )
}

export function signInFailedPage(): string {
  return message('Sign-in didn’t finish', 'It may have been canceled, or taken too long. Try signing in again.', SIGN_IN_BUTTON)
}

export function notConfiguredPage(): string {
  return message(
    'Not available yet',
    'Signing in on the web isn’t set up yet. Check back soon.',
  )
}

export function unreachablePage(retry: string, viewer?: Viewer): string {
  return message(
    'Couldn’t load this page',
    'Far Cooler’s server didn’t answer. Try again in a moment.',
    `<a class="button" href="${text(retry)}">Try Again</a>`,
    undefined,
    viewer,
  )
}

export function limitedPage(): string {
  return message('Try again in a minute', 'There have been a lot of sign-in attempts from your network just now.', '<a class="button" href="/app">Try Again</a>')
}

export function notFoundPage(viewer?: Viewer): string {
  return message(
    'Team not found',
    'You may not be in this team anymore, or the link is wrong.',
    '<a class="button" href="/app">My Teams</a>',
    undefined,
    viewer,
  )
}

export function forbiddenPage(viewer?: Viewer): string {
  return message(
    'Members aren’t visible to you',
    'Billing members can see a team, but not who’s in it. Ask a team admin if you need to.',
    '<a class="button" href="/app">My Teams</a>',
    undefined,
    viewer,
  )
}

export function teamsPage(teams: Team[], viewer: Viewer): string {
  const rows = teams
    .map(
      team => `<li><a class="row" href="/app/teams/${encodeURIComponent(team.id)}"><span class="main"><span class="name">${text(team.name)}</span><span class="meta">${team.personal ? 'Personal team · ' : ''}${roleName(team.role)}</span></span>${CHEVRON}</a></li>`,
    )
    .join('\n')
  return document(
    'My Teams – Far Cooler',
    `<main class="wrap page">
<h1>My Teams</h1>
<p class="lede">The teams you’re in, and your role in each.</p>
${teams.length ? `<ul class="list">\n${rows}\n</ul>` : '<p class="note">You’re not in any teams yet.</p>'}
</main>`,
    viewer,
  )
}

export function membersPage(team: { name: string; personal: boolean }, members: Member[], self: string, viewer: Viewer): string {
  const rows = members
    .map(member => {
      const label = member.email === null ? 'No email on file' : email(member.email)
      const initial = (member.email ?? '?').trim().charAt(0) || '?'
      const meta = [
        member.user === self ? 'You' : '',
        joinedOn(member.joinedAt),
      ].filter(Boolean).join(' · ')
      return `<li><div class="row"><span class="avatar" aria-hidden="true">${text(initial)}</span><span class="main"><span class="name">${label}</span>${meta ? `<span class="meta">${meta}</span>` : ''}</span><span class="end">${roleName(member.role)}</span></div></li>`
    })
    .join('\n')
  const count = members.length === 1 ? '1 person' : `${members.length} people`
  return document(
    `${team.name} – Far Cooler`,
    `<main class="wrap page">
<a class="back" href="/app">‹ My Teams</a>
<h1>${text(team.name)}</h1>
<p class="lede">${team.personal ? 'Your personal team. It holds your own runners, and only you are in it.' : count}</p>
<ul class="list members">
${rows}
</ul>
</main>`,
    viewer,
  )
}
