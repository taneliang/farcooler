/// The React shell for a browser check's harness page (ov-1510): the page's
/// views are parked until the shell mounts them, so every harness built on
/// index.html mounts it first, then sets what the sidebar and header say.

import { mountShell } from '../../src/client/shell/shell'
import { shell, type ShellState } from '../../src/client/shell/store'

export function harnessShell(state: Partial<ShellState> = {}) {
  mountShell(document.getElementById('shell')!)
  shell.set({ screen: 'work', ...state })
  // As app.ts's show() does: the chosen screen, and no other, is showing.
  const screen = shell.read().screen
  for (const id of ['status', 'enroll', 'compare', 'join', 'work']) document.getElementById(id)?.toggleAttribute('hidden', id !== screen)
  return shell
}

export const runnerNamed = (label: string, id = label) => ({ id, label, health: '', runtime: '' })
