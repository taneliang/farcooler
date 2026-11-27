/// Add Runner (ov-1490): the one command to run on a computer, and the
/// account's runners this browser doesn't open yet.
///
/// The command carries this browser's own offer (crates/ceremony `JoinCode`),
/// so nobody between the page and the terminal can put another key in it. The
/// relay gives only a ticket. The runner's sealed answer is opened like any
/// other reply, and nothing is kept before the person matches its code.

import { say } from './say'

export interface Unenrolled {
  name: string
  lastSeenAt: number | null
}

export interface JoinDeps {
  /// `POST /api/relay/<route>`.
  call: (route: string, body: unknown) => Promise<{ status: number; body: any }>
  /// crates/web `joinCommand`.
  command: (offer: string, ticket: string, team: string) => string
}

/// The command for the offer `offer`, with a ticket the relay made for its
/// ceremony, or null when the relay can't say.
export async function joinCommandFor(deps: JoinDeps, offer: string): Promise<string | null> {
  const ceremony = (JSON.parse(offer) as { ceremony: string }).ceremony
  const answer = await deps.call('join/ticket', { ceremony }).catch(() => null)
  const { ticket, team } = answer?.status === 200 ? (answer.body ?? {}) : ({} as Record<string, unknown>)
  if (typeof ticket !== 'string' || typeof team !== 'string') return null
  return deps.command(offer, ticket, team)
}

export interface ListDeps {
  call: (route: string, body: unknown) => Promise<{ status: number; body: any }>
  /// crates/web `unenrolledRunners`.
  unenrolled: (teams: string, account: string, grants: string) => string
}

/// The account's runners this browser holds no usable grant for, from every
/// team it is in. Null when the directory couldn't be read: the page then says
/// so, rather than showing none.
export async function unenrolledRunners(deps: ListDeps, account: string, grants: unknown): Promise<Unenrolled[] | null> {
  const me = await deps.call('directory/me', {}).catch(() => null)
  if (me?.status !== 200 || !Array.isArray(me.body?.teams)) return null
  const lists: unknown[] = []
  for (const team of me.body.teams as { id: string; role: string }[]) {
    if (team.role === 'billing') continue
    const answer = await deps.call('directory/teams/runners', { team: team.id }).catch(() => null)
    if (answer?.status !== 200) return null
    lists.push(answer.body)
  }
  return JSON.parse(deps.unenrolled(JSON.stringify(lists), account, JSON.stringify(grants ?? []))) as Unenrolled[]
}

/// Draw the not-yet-set-up list, each with its Set Up button.
export function drawUnenrolled(list: HTMLElement, runners: Unenrolled[], setUp: ((name: string) => void) | null, now: number): void {
  list.replaceChildren(
    ...runners.map(runner => {
      const item = document.createElement('li')
      const name = document.createElement('strong')
      name.textContent = runner.name
      const heard = document.createElement('span')
      heard.textContent = say({ say: 'last_heard', at: runner.lastSeenAt, now })
      item.append(name, heard)
      if (setUp) {
        const button = document.createElement('button')
        button.type = 'button'
        button.className = 'quiet'
        button.textContent = say({ say: 'set_up' })
        button.setAttribute('aria-label', say({ say: 'set_up_runner', name: runner.name }))
        button.onclick = () => setUp(runner.name)
        item.append(button)
      }
      return item
    }),
  )
}
