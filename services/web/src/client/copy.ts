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
    case 'bad_key':
      return "This browser couldn’t sign with its key. Reload the page and try again."
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
export function unreachable(why: string, runner: string): string {
  return why === 'pending'
    ? `Your phone couldn’t add this browser’s key to ${runner}. Add this browser again while your phone can reach it.`
    : `${runner} is only reachable at an address, and a browser can only reach runners through Far Cooler’s tunnel. Turn on the tunnel for ${runner}, then add this browser again.`
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
