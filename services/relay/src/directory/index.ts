/// The team directory's routes (ov-487, phase 1 of ov-480).
///
/// Users, teams, memberships, runners and who may reach them, devices with
/// their public keys and certificates, invites, and the audit log. Every route
/// is a POST with a WorkOS session, like the rest of the relay.
///
/// The directory never holds a private key and never grants access to a
/// runner. It lists, records, and takes access away; adding a key to a runner
/// needs a runner admin's signed grant (phase 4).

import { json, type Env } from '../index'
import { certifyDevice, deviceChallenge, registerDeviceKeys, revokeDeviceKeys } from './devices'
import { confirmHeldDevice } from './hold'
import { acceptInvite, createInvite, listInvites, previewInvite, revokeInvite } from './invites'
import { runnerKeys } from './inventory'
import { claimRunner, listRunners, registerRunner, removeRunner, removeRunnerRole } from './runners'
import { createTeam, listAudit, listMembers, me, readmit, removeMember, setRole } from './teams'

export const DIRECTORY_PREFIX = '/v1/directory/'

const ROUTES: Record<string, (request: Request, env: Env) => Promise<Response>> = {
  me,
  'teams/create': createTeam,
  'teams/members': listMembers,
  'teams/members/role': setRole,
  'teams/members/remove': removeMember,
  'teams/members/readmit': readmit,
  'teams/audit': listAudit,
  'teams/devices/confirm': confirmHeldDevice,
  'teams/runners': listRunners,
  'teams/invites/create': createInvite,
  'teams/invites/list': listInvites,
  'teams/invites/revoke': revokeInvite,
  'invites/accept': acceptInvite,
  'invites/preview': previewInvite,
  'runners/register': registerRunner,
  'runners/claim': claimRunner,
  'runners/remove': removeRunner,
  'runners/keys': runnerKeys,
  'runners/roles/remove': removeRunnerRole,
  'devices/challenge': deviceChallenge,
  'devices/register': registerDeviceKeys,
  'devices/certify': certifyDevice,
  'devices/revoke': revokeDeviceKeys,
}

export async function directory(request: Request, env: Env, path: string): Promise<Response> {
  const route = Object.hasOwn(ROUTES, path.slice(DIRECTORY_PREFIX.length)) ? ROUTES[path.slice(DIRECTORY_PREFIX.length)] : undefined
  if (!route) return json({ error: 'not found' }, 404)
  return await route(request, env)
}
