// An agent's pane with the Mac's view switch (src/client/agent-pane.ts,
// ov-871): the conversation holds no channel, the terminal holds one, and the
// set budgets for it as for any terminal. The two views are played by the test.

import { describe, expect, it } from 'vitest'

import { AgentPane } from '../src/client/agent-pane'
import { type Pane, PaneSet } from '../src/client/panes'

/// A view, and the channels the runner has open.
class Channels {
  open = 0
}

class View implements Pane<Channels> {
  active = false
  used = false
  visible = false
  attaches: boolean[] = []
  private runner: Channels | null = null
  constructor(readonly holds: boolean) {}
  show(on: boolean) {
    this.visible = on
  }
  async attach(runner: Channels, _t: string, _e: () => void, resuming: boolean) {
    this.attaches.push(resuming)
    this.used = true
    if (!this.holds) return
    if (!this.active) runner.open++
    this.runner = runner
    this.active = true
  }
  detach() {
    if (this.active) this.runner!.open--
    this.active = false
  }
  dispose() {
    this.detach()
  }
}

function rig() {
  const runner = new Channels()
  const chat = new View(false)
  const terminals: View[] = []
  const agent = new AgentPane<Channels>(chat, () => {
    const t = new View(true)
    terminals.push(t)
    return t
  })
  const shells = new Map<string, View>()
  const set = new PaneSet<Channels>(id => {
    if (id === 'agent') return agent
    const shell = new View(true)
    shells.set(id, shell)
    return shell
  })
  const open = (id: string) => set.open(id, runner, () => {})
  return { runner, chat, terminals, agent, set, open, shells }
}

describe('the view switch', () => {
  it('opens as the conversation, which holds no channel', async () => {
    const { agent, chat, runner, open } = rig()
    await open('agent')
    expect(agent.mode).toBe('conversation')
    expect(chat.visible).toBe(true)
    expect(agent.active).toBe(false)
    expect(runner.open).toBe(0)
  })

  it('shows the terminal on a channel of its own, then the conversation again, giving the channel up', async () => {
    const { agent, chat, terminals, runner, open } = rig()
    await open('agent')
    agent.setMode('terminal')
    await open('agent')
    expect(terminals).toHaveLength(1)
    expect(terminals[0].visible).toBe(true)
    expect(chat.visible).toBe(false)
    expect(agent.active).toBe(true)
    expect(runner.open).toBe(1)

    agent.setMode('conversation')
    await open('agent')
    expect(chat.visible).toBe(true)
    expect(terminals[0].visible).toBe(false)
    expect(agent.active).toBe(false)
    expect(runner.open).toBe(0)
    expect(chat.attaches).toHaveLength(2)
  })

  it('makes the terminal’s screen only when it is first asked for, and keeps it after', async () => {
    const { agent, terminals, open } = rig()
    await open('agent')
    expect(terminals).toHaveLength(0)
    agent.setMode('terminal')
    await open('agent')
    agent.setMode('conversation')
    agent.setMode('terminal')
    await open('agent')
    expect(terminals).toHaveLength(1)
    // The second time the screen is resumed rather than cleared.
    expect(terminals[0].attaches).toEqual([false, true])
  })

  it('takes its channel from the set’s budget: seven terminals open, the least recent lets go for it', async () => {
    const { agent, runner, shells, open, set } = rig()
    await open('agent')
    for (let i = 1; i <= 7; i++) await open(`t${i}`)
    expect(runner.open).toBe(7)
    await open('agent')
    agent.setMode('terminal')
    await open('agent')
    expect(runner.open).toBe(7)
    expect(shells.get('t1')?.active).toBe(false)
    expect(set.live).toContain('agent')
  })

  it('goes back to the view it was in after a reconnect', async () => {
    const { agent, terminals, runner, set } = rig()
    await set.open('agent', runner, () => {})
    agent.setMode('terminal')
    await set.open('agent', runner, () => {})
    runner.open = 0
    terminals[0].active = false
    await set.resume(runner, () => {})
    expect(agent.mode).toBe('terminal')
    expect(runner.open).toBe(1)
    expect(terminals[0].attaches).toEqual([false, true])
  })

  it('is used once either view has been shown, and goes away with both', async () => {
    const { agent, chat, terminals, open } = rig()
    expect(agent.used).toBe(false)
    await open('agent')
    expect(agent.used).toBe(true)
    agent.setMode('terminal')
    await open('agent')
    agent.dispose()
    expect(terminals[0].active).toBe(false)
    expect(chat.active).toBe(false)
  })
})
