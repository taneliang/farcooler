# ov-1453 · Pause-snapshot landing test

The shared test helper used a two-second deadline to observe the landing wrapper's first wait message and a three-second deadline to see it exit. A slow wrapper could miss the first deadline while the builder still held its ticket and remained runnable, as in CI run 45263849183, attempt 1, Rust tests (2/4).

The helper now reads the wrapper's flushed stderr lines asynchronously. When the wrapper names the builder as the ticket holder, the helper checks that the builder is runnable and landing is pending, then releases the builder. It awaits the wrapper's exit and checks the landing marker. The helper retains stderr for diagnostics and uses a 30-second timeout as a hang guard.

## Controlled delay

I temporarily added these lines to the test's embedded Python driver after `import heavy_lock as hl`, then ran `yielding::tests::a_legacy_wrapper_launched_after_the_pause_snapshot_does_not_block_landing` with `--exact` before and after the fix.

```python
if 'landing' == __import__('os').environ.get('FARCOOLER_OPERATION_BINDING'):
    __import__('time').sleep(6)
```

| Test version | Verdict | Test time |
| --- | --- | --- |
| Original helper with delay | Failed with `landing must wait on a runnable builder`, builder states `["SN", "SN", "SN", "SN"]`, and empty landing output | 5.13 s |
| Event-driven helper with the same delay | 1 passed, 0 failed | 6.81 s |

The delay was removed from the test source after this comparison.

## Admission latch under load

The first stress series stopped at run 38 when landing reached the hang guard with empty stderr. An instrumented series stopped at run 14 and captured the landing wrapper blocked in `fcntl.flock` on the canonical queue's `.enter` latch.

The late-start fixture made a single pause sweep. Under load, the sweep could see a pending stop and resume the late ticket holder while retaining the latch for the next sweep. The fixture now checks that the forced sweep cleared the late holder's pause before using normal gate admission to finish settling and release the latch.

The final harness passed the exact test in 7.93 seconds with a temporary `python3` shim that slept for six seconds before launching only the landing wrapper.

## Final verification

The exact pause-snapshot test passed **200 consecutive runs in 366.1 seconds** on ARM64 macOS. A foreground driver ran the built harness once per iteration with `--exact`, stopping on any failure. Ten `/usr/bin/yes` workers ran for the entire series with stdout directed to `/dev/null`. Their sampled aggregate CPU use ranged from 75.7% to 500.1%, where 100% is one core. Host one-minute load averages sampled during the series were 134.1 and 158.2.

The parallel `cargo test -p farcooler-daemon --lib yielding::` run on the final build passed all three landing scenarios and 17 of 18 cases overall. The gate-clock test `a_builder_counts_the_time_a_gate_held_it_and_not_its_hand_pause` observed a 589.7 ms hold against its 550 ms upper bound.

Serial nextest passed **18 of 18** yielding tests in 15.0 seconds with zero retries.

```sh
cargo nextest run -p farcooler-daemon --lib -E 'test(/^yielding::tests::/)' --test-threads 1 --retries 0
```
