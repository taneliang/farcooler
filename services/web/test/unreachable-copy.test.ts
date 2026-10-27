// Why a granted runner can't be opened here (src/client/copy.ts, ov-828):
// the phone's reason when it gave one, and OpenSSH named only when the phone
// read it off the runner. The owner's Cosmo, on OpenSSH 10.3, was told its
// OpenSSH might be too old because the reply carried no reason at all.

import { describe, expect, it } from 'vitest'

import { unreachable } from '../src/client/copy'

const CAUSES = ['too_old', 'no_passkeys', 'not_allowed', 'not_connected', 'failed']

describe('unreachable', () => {
  it('names OpenSSH only when the phone said the runner lacks passkeys', () => {
    expect(unreachable('pending', 'Cosmo', 'no_passkeys')).toContain('OpenSSH')
    for (const cause of ['', 'too_old', 'not_allowed', 'not_connected', 'failed', 'something new'])
      expect(unreachable('pending', 'Cosmo', cause), cause).not.toContain('OpenSSH')
  })

  it('says something different for each reason, and names the runner', () => {
    const said = CAUSES.map(cause => unreachable('pending', 'Cosmo', cause))
    expect(new Set([...said, unreachable('pending', 'Cosmo')]).size).toBe(CAUSES.length + 1)
    for (const sentence of said) expect(sentence).toContain('Cosmo')
  })

  it('tells a runner the phone couldn’t reach from one that refused', () => {
    expect(unreachable('pending', 'Cosmo', 'not_connected')).toContain('wasn’t connected')
    expect(unreachable('pending', 'Cosmo', 'not_allowed')).toContain('didn’t let your phone add devices')
  })

  it('keeps the address-only sentence for a runner with no tunnel', () => {
    expect(unreachable('address_only', 'Cosmo', 'failed')).toContain('only reachable at an address')
  })
})
