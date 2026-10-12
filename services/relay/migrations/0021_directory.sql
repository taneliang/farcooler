-- The team directory, phase 1 (ov-487; design: ov-480 §2-§3, §7 step 1).
--
-- Users are `accounts`, as before: the WorkOS user id. Everything else here
-- is new: teams, who is in them, the runners each team holds and who may reach
-- each one, the devices each person signs with, invite links, and an
-- append-only log of who changed what.
--
-- Public halves only. A device's keys are stored as `ssh-ed25519 AAAA…` with
-- the comment dropped, and every row that admits one was first proven by a
-- signature from the matching private half. No private key is ever sent here,
-- and no column could hold one.
--
-- The directory records access; it never grants it. A runner's
-- `authorized_keys` stays the only thing sshd obeys, and a key reaches it only
-- through a grant a runner admin's device signed (phase 4). `runner_roles` is
-- what the directory knows and shows, not what a runner enforces.
--
-- Additive only, same as 0002 through 0020: new tables, and every foreign key
-- into a table that existed before this file says ON DELETE CASCADE or SET
-- NULL, so the previous worker's deletes on `accounts` and `daemons` still go
-- through while a deploy rolls out.

-- A team. Every person has exactly one personal team, made the first time they
-- reach the directory: `personal_of` is set on it and on no other. A personal
-- team can't be left, shared, or joined by invite.
CREATE TABLE teams (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  personal_of TEXT UNIQUE REFERENCES accounts(id) ON DELETE CASCADE,
  created_by TEXT REFERENCES accounts(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL
);

-- Who is in a team, and as what. `billing` is stored now so the set needn't
-- change later; it grants nothing yet, not even a look at the runners.
CREATE TABLE memberships (
  team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK (role IN ('admin', 'member', 'billing')),
  joined_at INTEGER NOT NULL,
  PRIMARY KEY (team_id, account_id)
);
CREATE INDEX memberships_by_account ON memberships (account_id);

-- A runner, in exactly one team. `install_key` is `sha256("directory-runner:" +
-- install id)`, global rather than per account (0013's `install_id` is per
-- account), because one runner must not be registered twice, by anyone. The
-- raw id is never stored: it is a UUIDv7 and carries the install time.
--
-- `daemon_id` is the token the runner notifies and beats with. SET NULL, so a
-- revoke through the old `/v1/daemons/revoke` leaves the runner listed with no
-- token rather than failing.
CREATE TABLE runners (
  id TEXT PRIMARY KEY,
  team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  install_key TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,
  daemon_id TEXT REFERENCES daemons(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL
);
CREATE INDEX runners_by_team ON runners (team_id);

-- Who may reach a runner, as the directory knows it. One role per person per
-- runner. `owner` is the admin who added the runner to its team, a label with
-- an admin's powers that other runner admins can't take away; there is at most
-- one. `read`, `control` and `shell` are the fence's scopes.
--
-- `signature` is the runner admin's signed grant. NULL only on the owner row
-- written at registration, which records an admin the runner already has
-- rather than granting anything.
CREATE TABLE runner_roles (
  runner_id TEXT NOT NULL REFERENCES runners(id) ON DELETE CASCADE,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'read', 'control', 'shell')),
  granted_by TEXT,
  signature TEXT,
  granted_at INTEGER NOT NULL,
  PRIMARY KEY (runner_id, account_id)
);
CREATE UNIQUE INDEX runner_roles_one_owner ON runner_roles (runner_id) WHERE role = 'owner';
CREATE INDEX runner_roles_by_account ON runner_roles (account_id);

-- A person's device and its public keys: Key A (every device), Key B (a Mac's
-- shell key) and the tunnel's node key. Separate from `devices`, which holds
-- push tokens and is per app install, so neither table's rules bend for the
-- other.
--
-- A fingerprint belongs to one profile, globally: the inventory (phase 2)
-- matches a runner's keys to people by it, so two claims on one key would make
-- every match a guess. Both Key A and Key B are admitted only with a signature
-- made by that key.
--
-- `state`: the first device a person ever registers is `trusted` on first
-- use; every later one is `pending` until one of their trusted or certified
-- devices signs a certificate for it, which makes it `certified`. `revoked`
-- is kept, never deleted, so a revoked key can't be registered again.
CREATE TABLE device_keys (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  label TEXT NOT NULL,
  key_a TEXT NOT NULL,
  key_a_fingerprint TEXT NOT NULL UNIQUE,
  key_b TEXT,
  key_b_fingerprint TEXT UNIQUE,
  node_key TEXT UNIQUE,
  state TEXT NOT NULL CHECK (state IN ('trusted', 'pending', 'certified', 'revoked')),
  certified_by TEXT REFERENCES device_keys(id) ON DELETE SET NULL,
  certificate TEXT,
  created_at INTEGER NOT NULL,
  certified_at INTEGER,
  revoked_at INTEGER
);
CREATE INDEX device_keys_by_account ON device_keys (account_id);

-- An invite link. The code is shown once and kept only as its hash, like a
-- daemon token. Joins as `member` or `billing`; admin is a role change by an
-- admin, never something a link hands out.
CREATE TABLE invites (
  id TEXT PRIMARY KEY,
  team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  code_hash TEXT NOT NULL UNIQUE,
  role TEXT NOT NULL CHECK (role IN ('member', 'billing')),
  domain TEXT,
  max_uses INTEGER NOT NULL,
  uses INTEGER NOT NULL DEFAULT 0,
  created_by TEXT REFERENCES accounts(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL,
  revoked_at INTEGER
);
CREATE INDEX invites_by_team ON invites (team_id);

-- Who changed what, append-only: nothing updates or deletes a row here. The
-- actor has no foreign key, so a line outlives the account it names. `team_id`
-- is NULL for a person's own devices, which belong to no team.
CREATE TABLE audit (
  id INTEGER PRIMARY KEY,
  team_id TEXT REFERENCES teams(id) ON DELETE CASCADE,
  actor TEXT,
  action TEXT NOT NULL,
  subject TEXT,
  detail TEXT,
  at INTEGER NOT NULL
);
CREATE INDEX audit_by_team ON audit (team_id, id);
CREATE INDEX audit_by_actor ON audit (actor, id);
