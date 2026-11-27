/// What Add Runner, approval and Devices say (ov-1490). The words are Rust's
/// (crates/client `enroll_view`, R-112): this names the one a screen needs.

import { enrollWords } from 'fc-web'

type Plain =
  | 'add_runner' | 'getting_command' | 'command_expired' | 'command_unavailable' | 'copy_command' | 'copy_by_hand'
  | 'code_expired' | 'not_in_account' | 'added_detail' | 'set_up' | 'no_runner_to_list' | 'no_camera'
  | 'connect_first' | 'choose_runner' | 'reply_not_sent'

export type Say =
  | { say: Plain }
  | { say: 'own_passkey'; fingerprint: string }
  | { say: 'added_title' | 'adding'; runner: string }
  | { say: 'last_heard'; at: number | null; now: number }
  | { say: 'set_up_runner' | 'asking'; name: string }
  | { say: 'devices_title' | 'devices_detail'; runner: string | null }
  | { say: 'this_browser_row'; label: string }
  | { say: 'remove_button'; confirming: boolean }
  | { say: 'remove_label'; confirming: boolean; device: string }
  | { say: 'removing'; device: string; runner: string }
  | { say: 'removed'; device: string; done: string[]; failed: string[]; offline: string[] }
  | { say: 'code_refused'; word: string }
  | { say: 'not_connected'; runners: string[] }
  | { say: 'check_code'; failed: string[] }

export const say = (what: Say): string => enrollWords(JSON.stringify(what))
