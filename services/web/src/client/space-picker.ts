/// The workspace picker in the sidebar (ov-871): a native select, so it works
/// with a keyboard, a screen reader and a phone's wheel. Each option names its
/// workspace and how many items wait in it. Hidden when the runner has fewer
/// than two workspaces, so a runner without any looks as it always did.

import { optionLabel, type Space } from './scope'

/// Draw `spaces` into `select` with `chosen` selected, or leave it be when
/// it would look the same: a poll that found nothing new must not close the
/// list a person has open. Returns what was drawn, to hand back as `last`.
export function drawPicker(select: HTMLSelectElement, spaces: Space[], chosen: string | null, waiting: Map<string, number>, last = ''): string {
  const signature = JSON.stringify([spaces.map(s => [s.id, s.name]), chosen, [...waiting]])
  if (signature === last) return last
  select.hidden = spaces.length < 2
  select.replaceChildren(
    ...spaces.map(space => {
      const option = document.createElement('option')
      option.value = space.id
      option.textContent = optionLabel(space, waiting.get(space.id) ?? 0)
      return option
    }),
  )
  select.value = chosen ?? ''
  return signature
}

/// What the browser remembers of the pick, per runner. Only a convenience: a
/// browser that won't keep it opens the main workspace.
const KEY = (runner: string) => `fc-space:${runner}`

export function savedSpace(runner: string): string | null {
  try {
    return localStorage.getItem(KEY(runner))
  } catch {
    return null
  }
}

export function saveSpace(runner: string, space: string): void {
  try {
    localStorage.setItem(KEY(runner), space)
  } catch {}
}
