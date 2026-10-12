import { describe, expect, it } from 'vitest'

import html from '../public/index.html?raw'

// The page is a file on disk, not code, so nothing else would notice it drift
// back to a hard-coded build.

describe('public/index.html', () => {
  it('points every Download button at the redirect, never at a build number', () => {
    expect(html.match(/href="\/download\/canary"/g)?.length).toBe(2)
    expect(html).not.toMatch(/Far(%20| )Cooler-\d+\.dmg/)
  })
})
