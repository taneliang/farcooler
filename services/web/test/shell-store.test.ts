import { describe, expect, it, vi } from 'vitest'

import { INITIAL, ShellStore } from '../src/client/shell/store'

// The shell's view model (ov-1510): app.ts and work-screen.ts write it, React
// renders it, so a write that changes nothing must render nothing.

describe('the shell store', () => {
  it('tells its listeners about a change, and not about a write that changes nothing', () => {
    const store = new ShellStore()
    const heard = vi.fn()
    store.subscribe(heard)
    store.set({ page: 'plan' })
    expect(store.read().page).toBe('plan')
    expect(heard).toHaveBeenCalledTimes(1)
    store.set({ page: 'plan', spaces: [] })
    store.set({ runner: { ...INITIAL.runner } })
    expect(heard).toHaveBeenCalledTimes(1)
  })

  it('stops telling a listener once it unsubscribes', () => {
    const store = new ShellStore()
    const heard = vi.fn()
    store.subscribe(heard)()
    store.set({ busy: true })
    expect(heard).not.toHaveBeenCalled()
  })

  it('changes one nav item or tool and keeps the rest', () => {
    const store = new ShellStore()
    store.command('nav', 'needs', { label: 'Needs You (3)' })
    store.command('tools', 'commands', { disabled: true })
    expect(store.read().nav.needs.label).toBe('Needs You (3)')
    expect(store.read().nav.board).toEqual(INITIAL.nav.board)
    expect(store.read().tools.commands).toEqual({ ...INITIAL.tools.commands, disabled: true })
    expect(store.read().tools.orchestrator).toEqual(INITIAL.tools.orchestrator)
  })

  it('resets a workspace’s state and handlers, keeping the screen and the runner’s connection', () => {
    const store = new ShellStore()
    const showTab = vi.fn()
    store.handle({ showTab })
    store.set({ screen: 'work', runner: { id: 'studio', label: 'studio', health: 'Connected', runtime: '' }, linkLost: true, page: 'board', startWork: 'Start Work', busy: true })
    store.reset()
    const state = store.read()
    expect(state).toEqual({ ...INITIAL, screen: 'work', runner: { id: 'studio', label: 'studio', health: 'Connected', runtime: '' }, linkLost: true })
    store.handlers.showTab('plan')
    expect(showTab).not.toHaveBeenCalled()
  })
})
