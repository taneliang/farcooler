// The card sheet's shared phrases (src/client/core-phrases.ts, ov-1041). wasm can't
// load under vitest's worker pool, so a stub answers with the golden
// (test/fixtures/plan-words.json, which the Rust test pins to crates/client's own).

import { describe, expect, it } from 'vitest'

import golden from '../../../test/fixtures/plan-words.json'
import type { PlanPresenter } from '../src/client/plan-present'
import { corePhrases } from '../src/client/core-phrases'

describe('the card sheet’s shared words', () => {
  it('asks the core for the web’s style, once, and says what it answers', () => {
    const seen: any[] = []
    const ask: PlanPresenter = request => {
      seen.push(JSON.parse(request))
      return JSON.stringify(golden.apple)
    }
    expect(corePhrases(ask).waitingOnYou).toBe('Waiting on You')
    expect(corePhrases(ask).waitingOnYou).toBe('Waiting on You')
    expect(seen).toEqual([{ schema: 1, words: { style: 'apple' } }])
  })

  it('leaves the phrase blank when the bridge cannot answer, and asks again', () => {
    let calls = 0
    const ask: PlanPresenter = () => {
      calls += 1
      return calls === 1 ? '{"schema":1,"error":"invalid"}' : JSON.stringify(golden.apple)
    }
    expect(corePhrases(ask).waitingOnYou).toBe('')
    expect(corePhrases(ask).waitingOnYou).toBe('Waiting on You')
  })
})
