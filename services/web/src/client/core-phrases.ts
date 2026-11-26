/// Phrases the card sheet shares with the plan, asked of crates/client's `plan_view`
/// through wasm's `planView` (ov-1041, ruling R-112). The page writes none of them.
/// Title Case, as the board says a state. The asker is injected, as the plan's
/// presenter is, so a test hands in the golden (test/fixtures/plan-words.json).

import type { PlanPresenter } from './plan-present'

export interface CorePhrases {
  /// The label of a card's answer row.
  waitingOnYou: string
}

const cache = new WeakMap<PlanPresenter, CorePhrases>()

/// The phrases, asked once per asker. A bridge that can't answer leaves them blank
/// rather than writing them here.
export function corePhrases(ask: PlanPresenter): CorePhrases {
  const kept = cache.get(ask)
  if (kept) return kept
  let words: CorePhrases = { waitingOnYou: '' }
  try {
    const answer = JSON.parse(ask(JSON.stringify({ schema: 1, words: { style: 'apple' } })))
    if (typeof answer?.words?.waiting_on_you === 'string') {
      words = { waitingOnYou: answer.words.waiting_on_you }
      cache.set(ask, words)
    }
  } catch {
    // Left blank; asked again next time.
  }
  return words
}

export function planRetryLabel(ask: PlanPresenter): string | undefined {
  try {
    const answer = JSON.parse(ask(JSON.stringify({ schema: 1, engine_words: { style: 'apple' } })))
    const label = answer?.words?.tryAgain
    return typeof label === 'string' ? label : undefined
  } catch {
    return undefined
  }
}
