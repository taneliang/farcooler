// Why a granted runner can't be opened here (src/client/copy.ts, ov-828):
// the phone's reason when it gave one, and OpenSSH named only when the phone
// read it off the runner. The owner's Cosmo, on OpenSSH 10.3, was told its
// OpenSSH might be too old because the reply carried no reason at all.

import { describe, expect, it } from 'vitest'

import { connectFailure, unreachable } from '../src/client/copy'

const CAUSES = ['too_old', 'no_passkeys', 'not_allowed', 'not_owner', 'read_only', 'not_connected', 'failed']

describe('unreachable', () => {
  it('names OpenSSH only when the phone said the runner lacks passkeys', () => {
    expect(unreachable('pending', 'Cosmo', 'no_passkeys')).toContain('OpenSSH')
    for (const cause of ['', 'too_old', 'not_allowed', 'not_owner', 'read_only', 'not_connected', 'failed', 'something new'])
      expect(unreachable('pending', 'Cosmo', cause), cause).not.toContain('OpenSSH')
  })

  it('says something different for each reason, and names the runner', () => {
    const said = CAUSES.map(cause => unreachable('pending', 'Cosmo', cause))
    expect(new Set([...said, unreachable('pending', 'Cosmo')]).size).toBe(CAUSES.length + 1)
    for (const sentence of said) expect(sentence).toContain('Cosmo')
  })

  it('tells a runner the phone couldn’t reach from one that refused', () => {
    expect(unreachable('pending', 'Cosmo', 'not_connected')).toContain('wasn’t connected')
    expect(unreachable('pending', 'Cosmo', 'not_allowed')).toContain('didn’t let the other device add this browser')
  })

  it('sends a member or a view-only phone to who can add the browser, not to an update', () => {
    for (const cause of ['not_owner', 'read_only']) expect(unreachable('pending', 'Cosmo', cause), cause).not.toContain('Update')
    expect(unreachable('pending', 'Cosmo', 'not_owner')).toContain('owner')
    expect(unreachable('pending', 'Cosmo', 'read_only')).toContain('Add Runner')
  })

  // ov-1490: there are no native apps to send anyone to.
  it('sends nobody to a phone app or a Mac app', () => {
    for (const cause of [...CAUSES, ''])
      expect(unreachable('pending', 'Cosmo', cause), cause).not.toMatch(/your phone|your mac/i)
    for (const word of ['host_key_changed', 'host_key_unknown', 'key_rejected'])
      expect(connectFailure(word, 'Cosmo'), word).not.toMatch(/phone|mac app/i)
  })

  it('keeps the address-only sentence for a runner with no tunnel', () => {
    expect(unreachable('address_only', 'Cosmo', 'failed')).toContain('only reachable at an address')
  })
})
