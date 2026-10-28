/// Every sentence the page shows for a failure, by the stable word the wasm
/// modules give (crates/web's WebError::word, the ceremony's
/// CeremonyError::code, the tunnel's dial word). No Rust or Go string reaches
/// the screen: an unknown word gets the general sentence.

/// Why connecting to `runner` failed.
export function connectFailure(word: string, runner: string): string {
  switch (word) {
    case 'host_key_changed':
      return `${runner} isn’t the runner your phone vouched for: its host key has changed, so Far Cooler won’t connect. If you reinstalled it, add this browser again.`
    case 'host_key_unknown':
      return `Your phone didn’t send a host key for ${runner}, so Far Cooler won’t connect to it. Add this browser again.`
    case 'key_rejected':
      return `${runner} doesn’t recognize this browser’s key. Add this browser again from your phone.`
    case 'no_answer':
      return `${runner} didn’t answer. Check that it’s on and online, then try again.`
    case 'derp':
      return "Far Cooler can’t reach its rendezvous service. Check your connection, then try again."
    case 'runner_too_old':
      return `${runner} needs a newer Far Cooler to show terminals here. Update it, then try again.`
    case 'still_attaching':
      return `${runner} hasn’t opened this terminal yet, so that much typing couldn’t be held for it. Wait for the terminal, then type again.`
    case 'bad_key':
      return "This browser’s passkey couldn’t sign in. Reload the page and try again."
    case 'passkey_declined':
      return `Far Cooler didn’t get your passkey, so it didn’t connect to ${runner}. Select Connect, then touch your passkey when your browser asks.`
    case 'io':
      return `Far Cooler couldn’t open a path through the tunnel to ${runner}. Check that Far Cooler is running on ${runner}, then try again. If it keeps happening, add this browser again.`
    case 'handshake_failed':
      return `${runner} dropped the connection while Far Cooler was signing in. Try again.`
    case 'exec_failed':
      return `${runner} couldn’t open a channel for this terminal. Wait a moment, then try again.`
    case 'protocol':
      return `${runner} sent an answer this browser couldn’t read. Update Far Cooler on ${runner}, then reload this page.`
    case 'refused':
      return `${runner} refused the request. Update Far Cooler on ${runner}, then try again.`
    case 'closed':
      return `The connection to ${runner} closed. Select Connect to open it again.`
    default:
      return `The connection to ${runner} didn’t work. Try again.`
  }
}

/// Why a scanned reply was refused.
export function replyRefusal(word: string): string {
  switch (word) {
    case 'version':
      return 'That code is from a newer Far Cooler. Reload this page, then scan it again.'
    case 'channel':
      return 'That code is from a different edition of Far Cooler. Use the Far Cooler Canary app on your phone.'
    case 'wrong_ceremony':
    case 'wrong_target':
      return 'That reply answers a different code. Scan the reply your phone shows for the code on this page.'
    case 'wrong_account':
      return 'That reply belongs to a different account. Sign in to Far Cooler on your phone with the account this browser uses.'
    case 'stale':
      return 'That reply came after this browser’s code expired. Here’s a new code: scan it with your phone, then scan its reply within two minutes.'
    case 'already_taken':
      return 'This browser already took a reply for this code. Select Start Over to add it again.'
    case 'no_tunnel':
      return "Your phone granted a runner this browser can’t reach. Select Start Over and try again."
    default:
      return "That isn’t a Far Cooler reply. Scan the code your phone shows after you confirm."
  }
}

/// Why a granted runner can't be opened here.
///
/// `cause` is the phone's reason for a pending runner (crates/web's
/// `enroll::CAUSES`), or empty from a phone that didn't say. Only
/// `no_passkeys`, which the phone reads off the runner's own capabilities,
/// names OpenSSH: an empty cause is not a guess about the runner (ov-828).
export function unreachable(why: string, runner: string, cause = ''): string {
  if (why !== 'pending')
    return `${runner} is only reachable at an address, and a browser can only reach runners through Far Cooler’s tunnel. Turn on the tunnel for ${runner}, then add this browser again.`
  switch (cause) {
    case 'no_passkeys':
      return `${runner}’s OpenSSH is too old for passkeys, so your phone couldn’t add this browser to it. Update OpenSSH on ${runner} to 8.4 or later, then add this browser again.`
    case 'too_old':
      return `Far Cooler on ${runner} is too old to add devices. Update it, then add this browser again.`
    case 'not_allowed':
      return `${runner} didn’t let your phone add devices. Update Far Cooler on ${runner}, or add this browser from your Mac.`
    case 'not_owner':
      return `Only ${runner}’s owner can add devices to it. Ask them to add this browser.`
    case 'read_only':
      return `Your phone can only view ${runner}, so it can’t add devices there. Add this browser from your Mac.`
    case 'not_connected':
      return `Your phone wasn’t connected to ${runner}. Open Far Cooler on your phone and wait for ${runner} to connect, then add this browser again.`
    case 'failed':
      return `${runner} didn’t take this browser’s passkey. Add this browser again from your phone.`
    default:
      return `Your phone couldn’t add this browser to ${runner}. Add this browser again while your phone is connected to ${runner}.`
  }
}

/// The headline for joining a teammate's runners, as the phone words it
/// (CeremonyStore.joinTitle): one runner by name, or several.
export function joinTitle(labels: string[]): string {
  return labels.length === 1 ? `Join Your Teammate’s Runner “${labels[0]}”?` : 'Join Your Teammate’s Runners?'
}

/// What joining means, under it (CeremonyStore.joinDetail).
export function joinDetail(from: string): string {
  return `This code says ${from} added this device to their runners. Work there runs on the Claude account signed in on each runner, so its usage and limits are shared. Join only if you were adding this device with them just now.`
}

/// What the page says about a connection that dropped (ov-772), by the
/// reconnector's view. `title` names the state in a word; `detail` says what
/// to do. `action` is the button, when there is one.
export function linkNotice(
  view: import('./reconnect').LinkView,
  runner: string,
): { title: string; detail: string; action: string | null } | null {
  switch (view.kind) {
    case 'up':
      return null
    case 'reconnecting':
      return {
        title: `Reconnecting to ${runner}…`,
        detail:
          view.tunnel === 'failed'
            ? `Far Cooler couldn’t reach ${runner} yet. Select Reconnect to try again, then touch your passkey.`
            : 'The connection dropped. Select Reconnect, then touch your passkey. The terminal comes back as you left it.',
        action: 'Reconnect',
      }
    case 'connecting':
      return { title: `Reconnecting to ${runner}…`, detail: 'Touch your passkey if your browser asks.', action: null }
    case 'failed':
      return {
        title: `Couldn’t reconnect to ${runner}`,
        detail: connectFailure(view.word, runner),
        action: 'Reconnect',
      }
  }
}

/// Why Needs You can't be shown, by the failure word of the read. A runner too
/// old to send the list is the phones' sentence (AgentKit's olderRunnerNote).
export function needsYouFailure(word: string, runner: string): string {
  if (word === 'runner_too_old') return `Update Far Cooler on ${runner} to see decisions and asks here.`
  if (word === 'closed' || word === 'no_answer') return `${runner} didn’t answer, so this list may be behind. Far Cooler is reconnecting.`
  return `Far Cooler couldn’t read what needs you on ${runner}. It tries again in a few seconds.`
}

/// What an answer says when the runner didn't take it: by the runner's `what`
/// where it named one, else by its code. `agent` is who the answer was for
/// (`claude`). Mirrors AgentKit's PhoneAnswer.refusal, so a browser and a
/// phone say the same thing; the runner's words (`scope-denied`) never reach
/// the screen.
///
/// `not_held` is an ask nothing holds any more: someone got there first.
/// `not_delivered` is one still held that the agent didn't take, so trying
/// again can work. Both are one code on the wire.
export function answerRefusal(failure: { word: string; code?: string; what?: string }, runner: string, agent: string): string {
  switch (failure.what) {
    case 'not_held':
      return 'Someone already answered this.'
    case 'not_delivered':
      return `Couldn’t reach ${agent}. Try again.`
    case 'too_soon':
      return 'An answer was just sent. Check the terminal before answering again.'
    case 'unconfirmed':
      return `Sent, but ${agent} hasn’t moved on yet. Check the terminal before answering again.`
    case 'not_on_yes':
      return 'The prompt’s cursor isn’t on Yes. Answer it in the terminal.'
  }
  if (failure.word !== 'refused') {
    // The connection, not the runner, said no.
    return failure.word === 'closed' || failure.word === 'no_answer'
      ? `${runner} didn’t answer. Your answer may not have arrived. Check the terminal, then try again.`
      : 'Your answer may not have reached the runner. Try again.'
  }
  switch (failure.code) {
    case 'scope-denied':
      return `This browser can only look at ${runner}. Answering needs control, which is granted from a device that already has it.`
    case 'capability-unsupported':
      return `Far Cooler on ${runner} is too old for this. Update it there, then try again.`
    case 'not-found':
      return 'It isn’t on the runner anymore. Something else removed it while this was open.'
    default:
      return `${runner} didn’t take your answer. Try again.`
  }
}

/// Why a sealed reply from the phone was not used, by the ceremony's word.
/// Said above the code while the wait goes on, except for `stale`.
export function sealedRefusal(word: string): string {
  switch (word) {
    case 'unopened':
    case 'malformed':
    case 'wrong_ceremony':
    case 'wrong_account':
    case 'wrong_target':
      return 'A reply arrived that this browser couldn’t open, so it was thrown away. If it wasn’t your phone’s, someone else posted it. Your phone’s reply can still arrive.'
    case 'stale':
      return replyRefusal('stale')
    case 'no_tunnel':
      return replyRefusal('no_tunnel')
    default:
      return 'A reply arrived that Far Cooler couldn’t use, so it was thrown away. Your phone’s reply can still arrive.'
  }
}

/// What the page says when the codes don’t match.
export const MISMATCH =
  'The codes didn’t match, so Far Cooler kept nothing from that reply. If you didn’t just add this browser from your phone, someone else may have posted it. Waiting for another reply…'

/// Why this browser couldn’t be put on the account’s device list.
export function registerFailure(word: string): string {
  switch (word) {
    case 'declined':
      return 'Far Cooler didn’t get your passkey. Select Register This Browser and follow your browser’s prompt.'
    case 'claimed':
      return 'This passkey is already registered to another Far Cooler account. Sign out, then sign in with the account your phone uses.'
    case 'revoked':
      return 'This passkey was removed from your account, so it can’t be added again. Sign out and back in to make a new one.'
    case 'unreachable':
      return 'Far Cooler couldn’t reach its service. Check your connection, then try again.'
    default:
      return 'Far Cooler couldn’t add this browser to your account. Try again.'
  }
}

/// Why the plan can't be shown, by the failure word of the read.
export function planFailure(word: string, runner: string): string {
  if (word === 'runner_too_old') return `Update Far Cooler on ${runner} to see the plan here.`
  if (word === 'closed' || word === 'no_answer') return `${runner} didn’t answer, so the plan may be behind. Far Cooler is reconnecting.`
  return `Far Cooler couldn’t read the plan on ${runner}. It tries again in a few seconds.`
}

/// What a plan change says when the runner didn't take it: by the runner's
/// code, in plain words, never the code itself. `what` is what the change was
/// to ("The Next line", "That ruling"). `not-yours` is the runner's gate
/// saying the call belongs to someone else (the owner's, on a member's
/// device).
export function planRefusal(failure: { word: string; code?: string; what?: string }, runner: string, what: string): string {
  if (failure.word !== 'refused') {
    return failure.word === 'closed' || failure.word === 'no_answer'
      ? `${runner} didn’t answer. ${what} may not have changed. Check the plan, then try again.`
      : `${what} may not have reached the runner. Try again.`
  }
  switch (failure.code) {
    case 'not-yours':
      return `${runner} says that one is the owner’s call. Ask the owner, or ask the orchestrator to change it.`
    case 'scope-denied':
      return `This browser can only look at ${runner}. Changing the plan needs control, which is granted from a device that already has it.`
    case 'capability-unsupported':
      return `Far Cooler on ${runner} is too old for this. Update it there, then try again.`
    case 'not-found':
      return 'It isn’t in the plan anymore. Something else removed it while this was open.'
    case 'resource-conflict':
      return 'Someone changed this first. The plan now shows their change.'
    default:
      return `${runner} didn’t take that change. Try again.`
  }
}
