// Whose phone the comparison code is read off (src/client/copy.ts, ov-778
// R-113). A teammate's browser has no phone of its own that shows the code:
// the teammate who sent the reply does, and the screen names them, so a
// reply from someone else can't borrow "your phone".

import { describe, expect, it } from 'vitest'

import { compareWords } from '../src/client/copy'

describe('compareWords', () => {
  it('asks about your own phone for your own reply', () => {
    const { title, detail } = compareWords(null)
    expect(title).toBe('Does your phone show this code?')
    expect(detail).toContain('Your phone sent its answer')
  })

  it('names the teammate whose phone sent a teammate’s reply, and asks to check it in person', () => {
    const { title, detail } = compareWords('alice@example.com')
    expect(title).toBe('Does alice@example.com’s phone show this code?')
    expect(detail).toContain('alice@example.com')
    expect(detail).toContain('in person')
    expect(`${title} ${detail}`).not.toMatch(/your phone/i)
  })
})
