// The size a pane had before the browser fitted it is given back (ov-853).

import { describe, expect, it } from 'vitest'

import { SizeLease } from '../src/client/lease'

describe('SizeLease', () => {
  it('gives back the Mac size once the page has reshaped the pane', () => {
    const lease = new SizeLease()
    lease.begin({ columns: 200, rows: 50 })
    lease.reshape()
    expect(lease.release()).toEqual({ columns: 200, rows: 50 })
    expect(lease.release()).toBeNull()
  })

  it('gives nothing back for a pane it never resized, or one whose size was unknown', () => {
    const lease = new SizeLease()
    lease.begin({ columns: 200, rows: 50 })
    expect(lease.release()).toBeNull()
    lease.begin(null)
    lease.reshape()
    expect(lease.release()).toBeNull()
  })

  it('forgets a previous pane when a new one begins', () => {
    const lease = new SizeLease()
    lease.begin({ columns: 200, rows: 50 })
    lease.reshape()
    lease.begin({ columns: 90, rows: 30 })
    expect(lease.release()).toBeNull()
  })
})
