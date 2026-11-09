/// Images in the conversation's box (ov-871): what may be attached, and the
/// words when something may not. The composer sends them as `terminal.compose`
/// does on the Mac and the phones, so the limits are the runner's own: the
/// daemon takes at most ten images, and the client core refuses one past
/// what the runner's `compose` accepts (`compose_limits`, crates/web). The
/// runner's refusals are crates/client's (`conversation_words`), so every
/// device says the same thing.
///
/// Nothing here touches the page or wasm, so a test runs it with images made
/// of a few bytes.

import { refused } from './chat-copy'

/// The most images in one message (the daemon's `MOST_IMAGES`).
export const MOST_IMAGES = 10

/// What a browser can show as a thumbnail and the agents read.
export const IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/gif', 'image/webp']

/// What the runner takes from the composer (`WebRunner.composeLimits`).
export interface Limits {
  images: boolean
  /// The most bytes in one image.
  each: number
  /// The most bytes in all of a message's images together.
  total: number
}

/// An image waiting to be sent.
export interface Picked {
  id: number
  name: string
  mime: string
  data: Uint8Array
}

export function parseLimits(json: string): Limits | null {
  try {
    const raw = JSON.parse(json)
    if (typeof raw?.images === 'boolean' && Number.isFinite(raw.each) && Number.isFinite(raw.total)) return { images: raw.images, each: raw.each, total: raw.total }
  } catch {}
  return null
}

export const IMAGE_WORDS = {
  // The runner's own refusals, in the words the apps share (`conversation_words`).
  get imageTooLarge() { return refused('image_too_large') },
  get imagesTooLarge() { return refused('images_too_large') },
  get tooMany() { return refused('images') },
  notAnImage: 'Only PNG, JPEG, GIF and WebP images can be attached.',
  get unreadable() { return refused('image') },
  tooOld: (runner: string) => `${runner} needs a newer Far Cooler to take images from here. Update it, then try again.`,
}

/// Add `incoming` to `held`, in order, keeping each that fits. `said` is the
/// first reason one didn't, or empty.
export function admit(held: Picked[], incoming: Picked[], limits: Limits | null, runner: string): { next: Picked[]; said: string } {
  if (limits && !limits.images) return { next: held, said: IMAGE_WORDS.tooOld(runner) }
  const next = [...held]
  let said = ''
  for (const image of incoming) {
    const total = next.reduce((sum, i) => sum + i.data.length, 0) + image.data.length
    if (!IMAGE_TYPES.includes(image.mime)) said ||= IMAGE_WORDS.notAnImage
    else if (limits && image.data.length > limits.each) said ||= IMAGE_WORDS.imageTooLarge
    else if (next.length >= MOST_IMAGES) said ||= IMAGE_WORDS.tooMany
    else if (limits && total > limits.total) said ||= IMAGE_WORDS.imagesTooLarge
    else next.push(image)
  }
  return { next, said }
}

/// The images among `files`, read: a file the browser can't read is left out
/// and counted in `unreadable`.
export async function readImages(files: File[], nextId: () => number): Promise<{ picked: Picked[]; others: number; unreadable: number }> {
  const picked: Picked[] = []
  let others = 0
  let unreadable = 0
  for (const file of files) {
    if (!file.type.startsWith('image/')) {
      others++
      continue
    }
    try {
      picked.push({ id: nextId(), name: file.name || 'Image', mime: file.type, data: new Uint8Array(await file.arrayBuffer()) })
    } catch {
      unreadable++
    }
  }
  return { picked, others, unreadable }
}

/// The files a drag or a paste carries, if any are files.
export function filesOf(transfer: DataTransfer | null): File[] {
  return transfer ? [...transfer.files] : []
}
