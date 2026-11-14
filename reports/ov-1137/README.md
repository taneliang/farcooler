# ov-1137: Linux watcher test race

The negative lock-probe test previously waited for ten empty drains (500 ms),
which only established that no callback had arrived during that interval. It
did not establish that the inotify queue had delivered the fixture and watch
registration events. A delayed callback could therefore surface during the
three-second lock assertion. The CI failure reported the repository root and
`.git` directory themselves, rather than `index.lock`: this is consistent with
inotify open/close events on watched directories from setup. The portable backend
routes every event's paths, including access events. This is the supported race
explanation; the original CI run did not record event kinds, so it does not
prove which setup operation produced those particular events.

The Linux test barrier now writes sentinels in both watched directories and
waits for their close-write events **after routing**. Those events follow the
setup events on the same inotify instance. The subsequent quiet and drain then
clear setup activity before the unchanged lock assertion; the test still checks
that an index rename surfaces the worktree. This instrumentation is test-only.

Validation on an aarch64 Linux VM with local scratch repositories:

- 200 separate invocations of
  `fs_watch::tests::the_index_lock_a_probe_takes_is_not_the_worktree_moving`:
  200 passed, 0 failed. The tested source differs from the committed source
  only in formatting and comments. The 200 individual logs were rechecked:
  each reports one passed test and zero failures.
- `fs_watch::` suite: 23 passed, 0 failed, 1 ignored (the pre-existing
  real-repository watch-count test).

The later workflow return did not include a review finding: the review passed,
and the subsequent landing wait timed out waiting for a green base. The
watcher change and its assertion therefore remain intact.
