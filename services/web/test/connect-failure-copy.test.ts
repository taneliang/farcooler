// Every word a failed connection can arrive as has its own sentence
// (src/client/copy.ts, ov-852): the general one is for a word nobody has met.
// The words are crates/web's WebError::word and the tunnel's dial words
// (crates/tailcat/go/main_js.go dialWord).

import { describe, expect, it } from 'vitest'

import { connectFailure } from '../src/client/copy'

const GENERAL = connectFailure('a word nobody has met', 'Cosmo')

const WEB_ERROR_WORDS = [
  'host_key_changed', 'host_key_unknown', 'key_rejected', 'handshake_failed', 'bad_key', 'passkey_declined',
  'exec_failed', 'protocol', 'runner_too_old', 'refused', 'closed', 'no_answer',
]
const DIAL_WORDS = ['derp', 'no_answer', 'io']

describe('connectFailure', () => {
  it('gives every real reason a sentence of its own, not the general one', () => {
    for (const word of [...WEB_ERROR_WORDS, ...DIAL_WORDS]) {
      const said = connectFailure(word, 'Cosmo')
      expect(said, word).not.toBe(GENERAL)
    }
  })

  it('never says the same thing for two different reasons', () => {
    const words = [...new Set([...WEB_ERROR_WORDS, ...DIAL_WORDS])]
    expect(new Set(words.map(w => connectFailure(w, 'Cosmo'))).size).toBe(words.length)
  })

  it('keeps the general sentence for a word it does not know', () => {
    expect(GENERAL).toBe('The connection to Cosmo didn’t work. Try again.')
  })

  it.each([
    ['io', 'tunnel'],
    ['handshake_failed', 'signing in'],
    ['exec_failed', 'channel'],
    ['protocol', 'couldn’t read'],
    ['refused', 'refused'],
    ['closed', 'closed'],
  ])('%s says what happened, and to whom', (word, phrase) => {
    expect(connectFailure(word, 'Cosmo')).toContain(phrase)
    expect(connectFailure(word, 'Cosmo')).toContain('Cosmo')
  })
})
