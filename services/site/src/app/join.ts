/// The pages of joining a team from an invite link (ov-524).

import { document, message, text, type Viewer } from './pages'
import { article } from './manage'

const SIGN_IN = '<a class="button" href="/app/sign-in">Sign In</a>'

/// The sign-in page for someone who opened a join link signed out.
export function joinSignInPage(): string {
  return message(
    'Join a team on Far Cooler',
    'Sign in to join the team you were invited to. You’ll see its name and confirm before anything happens.',
    SIGN_IN,
    'Use the same account as the Far Cooler app on your Mac or iPhone, or make one.',
  )
}

/// "Join Acme?", the confirmation. The code isn't on the page: the button
/// posts, and the Worker reads the code from the HttpOnly sign-in cookie.
export function joinConfirmPage(team: string, role: string, viewer: Viewer): string {
  return document(
    'Join a Team – Far Cooler',
    `<main class="wrap message">
<h1>Join ${text(team)}?</h1>
<p class="lede">You’ve been invited to join ${text(team)} as ${article(role)}.</p>
<div class="actions"><form method="post" action="/app/join"><input type="hidden" name="csrf" value="${text(viewer.csrf)}"><button class="button" type="submit">Join</button></form><a class="cancel" href="/app">Not Now</a></div>
</main>`,
    viewer,
  )
}

export function joinAlreadyPage(team: { id: string; name: string }, viewer: Viewer): string {
  return message(
    `You’re already in ${team.name}`,
    'There’s nothing more to do. This link didn’t use up one of its places.',
    `<a class="button" href="/app/teams/${encodeURIComponent(team.id)}">Open Team</a>`,
    undefined,
    viewer,
  )
}

const FAILURES: Record<string, [title: string, sentence: string]> = {
  used: ['This invite is used up', 'Everyone it was meant for has already joined. Ask a team admin for a new link.'],
  expired: ['This invite has expired', 'The link only worked for a while, and that time has passed. Ask a team admin for a new one.'],
  revoked: ['This invite was revoked', 'A team admin turned this link off. Ask them for a new one.'],
  domain: [
    'This invite is for another email',
    'It only works for people with a certain email domain, and your account’s verified email isn’t one of them. Sign in with the account it was meant for, or ask a team admin.',
  ],
  removed: ['You were removed from this team', 'A team admin removed you, so a link can’t bring you back. Ask an admin to readmit you.'],
  unknown: ['This invite isn’t valid', 'Check that the whole link was copied, or ask a team admin for a new one.'],
}

/// Why a link doesn't work, in its own plain sentence. An unrecognized reason
/// reads as the generic one.
export function joinFailurePage(reason: string, viewer?: Viewer): string {
  const [title, sentence] = Object.hasOwn(FAILURES, reason) ? FAILURES[reason] : FAILURES.unknown
  return message(title, sentence, '<a class="button" href="/app">My Teams</a>', undefined, viewer)
}
