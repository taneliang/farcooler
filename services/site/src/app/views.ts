/// The read-only pages that sit beside the team page (ov-562): a team's
/// runners, one runner, the audit log, and the account's own devices.
///
/// Each loader reads the directory with the visitor's session and returns what
/// `index.ts` finishes into a response. Nothing here posts, and none of these
/// pages has a form.

import { notFoundPage, type Viewer } from './pages'
import {
  aboutRunner,
  auditPage,
  devicesPage,
  noAccessPage,
  runnerNotFoundPage,
  runnerPage,
  runnersPage,
  type AuditRow,
  type DeviceRow,
  type KeyReport,
  type Names,
  type RunnerRow,
  type TeamRef,
} from './runners'
import type { Answer, Caller } from './relay'

export type Page = { status: number; html: string } | Exclude<Answer, { kind: 'ok' }>

const list = <T>(value: unknown): T[] => (Array.isArray(value) ? (value as T[]) : [])

/// Read a team's runners, or the page to show instead. A refusal of 404 or 400
/// is "no such team"; a 403 is a billing member.
async function runnersOf(caller: Caller, viewer: Viewer, me: any, team: string): Promise<{ team: TeamRef; runners: RunnerRow[] } | Page> {
  const mine = list<TeamRef>(me?.teams).find(t => t.id === team)
  if (!mine) return { status: 404, html: notFoundPage(viewer) }
  const found = await caller.directory('teams/runners', { team })
  if (found.kind === 'refused' && (found.status === 404 || found.status === 400)) return { status: 404, html: notFoundPage(viewer) }
  if (found.kind === 'refused' && found.status === 403) return { status: 403, html: noAccessPage('runners', viewer) }
  if (found.kind !== 'ok') return found
  const runners = list<RunnerRow>(found.body?.runners).map(r => ({ ...r, access: list<RunnerRow['access'][number]>(r.access) }))
  return { team: { id: mine.id, name: String(mine.name ?? 'Team'), personal: mine.personal === true, role: String(mine.role) }, runners }
}

const isLoaded = (value: { team: TeamRef; runners: RunnerRow[] } | Page): value is { team: TeamRef; runners: RunnerRow[] } => 'runners' in value

export async function runnersFor(caller: Caller, viewer: Viewer, me: any, team: string): Promise<Page> {
  const loaded = await runnersOf(caller, viewer, me, team)
  return isLoaded(loaded) ? { status: 200, html: runnersPage(loaded.team, loaded.runners, viewer) } : loaded
}

/// Who each id in the log is: the people in the team (an admin may list them)
/// and the runners.
function namesOf(runners: RunnerRow[], members: { user: string; email: string | null }[]): Names {
  const users = new Map<string, string>()
  for (const r of runners) for (const a of r.access) if (a.email) users.set(a.user, a.email)
  for (const m of members) if (typeof m.user === 'string' && m.email) users.set(m.user, m.email)
  return { users, runners: new Map(runners.map(r => [r.id, r.name])) }
}

/// An admin's names: the team's people, so an audit line names who did it.
async function adminNames(caller: Caller, team: string, runners: RunnerRow[]): Promise<Names | Exclude<Answer, { kind: 'ok' }>> {
  const members = await caller.directory('teams/members', { team })
  if (members.kind !== 'ok') return members
  return namesOf(runners, [...list<any>(members.body?.members), ...list<any>(members.body?.removed)])
}

export async function runnerFor(caller: Caller, viewer: Viewer, me: any, team: string, runner: string): Promise<Page> {
  const loaded = await runnersOf(caller, viewer, me, team)
  if (!isLoaded(loaded)) return loaded
  const row = loaded.runners.find(r => r.id === runner)
  if (!row) return { status: 404, html: runnerNotFoundPage(loaded.team, viewer) }

  const asked = await caller.directory('runners/keys', { runner })
  if (asked.kind === 'signed-out' || asked.kind === 'limited') return asked
  // A report that didn't load is not an empty one: the page says so.
  const num = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) ? Math.max(0, Math.trunc(v)) : 0)
  const keys: KeyReport | null =
    asked.kind === 'ok'
      ? { ...asked.body, keys: list(asked.body?.keys), total: num(asked.body?.total), unparsedSinceBaseline: num(asked.body?.unparsedSinceBaseline) }
      : null

  let names = namesOf(loaded.runners, [])
  let activity: AuditRow[] | 'failed' | null = null
  if (loaded.team.role === 'admin') {
    const [audit, people] = await Promise.all([caller.directory('teams/audit', { team }), adminNames(caller, team, loaded.runners)])
    for (const read of [audit, people]) if ('kind' in read && (read.kind === 'signed-out' || read.kind === 'limited')) return read
    // Access and Keys still show when only the log fails to load.
    if (audit.kind !== 'ok' || 'kind' in people) activity = 'failed'
    else {
      names = people
      activity = aboutRunner(list<AuditRow>(audit.body?.entries), runner)
    }
  }
  return { status: 200, html: runnerPage({ team: loaded.team, runner: row, keys, activity, names, viewer }) }
}

/// The audit log, a hundred lines at a time (`before` is the id to start under).
export async function auditFor(caller: Caller, viewer: Viewer, me: any, team: string, before: string | null): Promise<Page> {
  const mine = list<TeamRef>(me?.teams).find(t => t.id === team)
  if (!mine) return { status: 404, html: notFoundPage(viewer) }
  if (mine.role !== 'admin') return { status: 403, html: noAccessPage('audit', viewer) }
  const start = before !== null && /^\d{1,15}$/.test(before) ? Number(before) : undefined
  const found = await caller.directory('teams/audit', { team, ...(start === undefined ? {} : { before: start }) })
  if (found.kind === 'refused' && found.status === 403) return { status: 403, html: noAccessPage('audit', viewer) }
  if (found.kind === 'refused' && (found.status === 404 || found.status === 400)) return { status: 404, html: notFoundPage(viewer) }
  if (found.kind !== 'ok') return found
  const runners = await caller.directory('teams/runners', { team })
  if (runners.kind !== 'ok') return runners
  const people = await adminNames(caller, team, list(runners.body?.runners))
  if ('kind' in people) return people
  const entries = list<AuditRow>(found.body?.entries)
  const older = entries.length >= 100 ? entries[entries.length - 1].id : null
  const ref: TeamRef = { id: mine.id, name: String(mine.name ?? 'Team'), personal: mine.personal === true, role: mine.role }
  return { status: 200, html: auditPage(ref, entries, people, typeof older === 'number' ? older : null, start !== undefined, viewer) }
}

export function devicesFor(viewer: Viewer, me: any): Page {
  return { status: 200, html: devicesPage(list<DeviceRow>(me?.devices), viewer) }
}
