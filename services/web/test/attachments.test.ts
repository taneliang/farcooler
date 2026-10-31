// Images beside the conversation's box (src/client/attachments.ts, ov-871):
// which ones may wait to be sent, and the apps' sentences when one may not.

import { describe, expect, it } from 'vitest'

import { admit, IMAGE_WORDS, type Limits, MOST_IMAGES, parseLimits, type Picked, readImages } from '../src/client/attachments'

const image = (id: number, bytes: number, mime = 'image/png'): Picked => ({ id, name: `${id}.png`, mime, data: new Uint8Array(bytes) })
const limits: Limits = { images: true, each: 100, total: 150 }

describe('what may be attached', () => {
  it('keeps images in the order they came', () => {
    const { next, said } = admit([image(1, 10)], [image(2, 10), image(3, 10)], limits, 'studio')
    expect(next.map(i => i.id)).toEqual([1, 2, 3])
    expect(said).toBe('')
  })

  it('leaves out one past the runner’s limit for an image, and says so', () => {
    const { next, said } = admit([], [image(1, 101), image(2, 5)], limits, 'studio')
    expect(next.map(i => i.id)).toEqual([2])
    expect(said).toBe('That image is too large to send. Use a smaller one.')
  })

  it('leaves out one that would take the images past the limit together', () => {
    const { next, said } = admit([image(1, 90)], [image(2, 90)], limits, 'studio')
    expect(next.map(i => i.id)).toEqual([1])
    expect(said).toBe('These images are too large to send together. Send fewer or smaller ones.')
  })

  it('takes at most ten', () => {
    const held = Array.from({ length: MOST_IMAGES }, (_, i) => image(i, 1))
    const { next, said } = admit(held, [image(99, 1)], limits, 'studio')
    expect(next).toHaveLength(MOST_IMAGES)
    expect(said).toBe('A message takes at most 10 images.')
  })

  it('takes only images a browser can show and an agent can read', () => {
    const { next, said } = admit([], [image(1, 5, 'application/pdf'), image(2, 5, 'image/webp')], limits, 'studio')
    expect(next.map(i => i.id)).toEqual([2])
    expect(said).toBe(IMAGE_WORDS.notAnImage)
  })

  it('takes none from a runner too old for images, and names it', () => {
    const { next, said } = admit([], [image(1, 5)], { ...limits, images: false }, 'build-box')
    expect(next).toEqual([])
    expect(said).toBe('build-box needs a newer Far Cooler to take images from here. Update it, then try again.')
  })

  it('goes by the apps’ sizes when the runner’s are not known yet', () => {
    expect(admit([], [image(1, 5_000_000)], null, 'studio').next).toHaveLength(1)
  })
})

describe('the runner’s limits', () => {
  it('reads what the wasm module says, and nothing from what is not it', () => {
    expect(parseLimits('{"images":true,"each":16777216,"total":52428800}')).toEqual({ images: true, each: 16777216, total: 52428800 })
    expect(parseLimits('{"images":"yes"}')).toBeNull()
    expect(parseLimits('nonsense')).toBeNull()
  })
})

describe('reading what was dropped or chosen', () => {
  it('reads the images, in order, and counts the files that are not images', async () => {
    let id = 0
    const files = [new File([new Uint8Array([1, 2])], 'a.png', { type: 'image/png' }), new File(['hello'], 'notes.txt', { type: 'text/plain' }), new File([new Uint8Array([3])], '', { type: 'image/jpeg' })]
    const { picked, others, unreadable } = await readImages(files, () => ++id)
    expect(picked.map(p => [p.id, p.name, p.mime, [...p.data]])).toEqual([
      [1, 'a.png', 'image/png', [1, 2]],
      [2, 'Image', 'image/jpeg', [3]],
    ])
    expect(others).toBe(1)
    expect(unreadable).toBe(0)
  })
})
