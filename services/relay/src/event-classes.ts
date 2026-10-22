/// Whether a device keeps a notice class on (ov-94): the owner's task
/// notices and, since ov-633, a member's Needs You push both ask this.

import { DEFAULT_TASK_EVENTS } from './index'

/// Whether a device whose saved classes are `notifyEvents` (null: never
/// saved, the defaults) keeps class `event` on.
export function keepsEvent(notifyEvents: string | null, event: string): boolean {
  const kept = notifyEvents === null ? DEFAULT_TASK_EVENTS : notifyEvents.split(',').filter(e => e !== '')
  return kept.includes(event)
}
