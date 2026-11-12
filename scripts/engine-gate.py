#!/usr/bin/env python3
"""Configured Check/Run command for prospective lane gates (ov-1117).

Reuse is disabled unless --enable-reuse is explicitly published after independent
source and concrete prerequisite review. See docs/ci/engine-gate-reuse.md.
"""
import argparse
import json
import os
import pathlib
import subprocess
import sys
import time
import uuid

sys.dont_write_bytecode = True
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import gate_evidence as evidence

# Stable across engine retries, but never across workflows, policies, boards or
# base commits. Per-invocation binding remains in the engine's ProcessFact/log.
CONTEXT = ("FARCOOLER_DEFINITION_PIN", "FARCOOLER_POLICY_PIN", "FARCOOLER_WORKSPACE")


def immutable(path, raw):
    try:
        evidence.private_write(path, raw)
        path.chmod(0o400)
    except FileExistsError:
        evidence.require(not path.is_symlink() and path.read_bytes() == raw,
                         "engine scope input was replaced")


def run(args):
    repo = pathlib.Path.cwd().resolve()
    evidence.require(pathlib.Path(__file__).resolve() == repo / "scripts/engine-gate.py",
                     "adapter must run from candidate checkout")
    lane = evidence.load_lane()
    for key in (*CONTEXT, "FARCOOLER_BASE", "FARCOOLER_CANDIDATE", "FARCOOLER_OPERATION_BINDING"):
        evidence.require(os.environ.get(key), f"missing engine context: {key}")
    source = evidence.source_snapshot(repo)
    evidence.require(source["head"] == os.environ["FARCOOLER_CANDIDATE"], "engine candidate differs from HEAD")
    base = os.environ["FARCOOLER_BASE"]
    evidence.require(evidence.git(repo, "rev-parse", "--verify", base + "^{commit}").decode().strip() == base,
                     "engine base must be a resolved commit")
    root = pathlib.Path(args.evidence_root).resolve()
    evidence.require(not root.is_relative_to(repo), "private evidence root must be outside checkout")
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    evidence.require(root.stat().st_uid == os.getuid() and root.stat().st_mode & 0o077 == 0,
                     "engine evidence root must be private and owned by runner")
    inputs = pathlib.Path(args.inputs).resolve()
    raw = inputs.read_bytes()
    declaration = json.loads(raw)
    # Preserve the reviewed inventory; add public adapter/context inputs to every
    # job's native inventory, so ov-903's unchanged checker binds the engine too.
    evidence.require(set(declaration) == {"schema", "jobs"}, "invalid reviewed input declaration")
    context = {"schema": "engine-gate/v1", "trust_scope": args.trust_scope,
               "workflow": {k: os.environ[k] for k in CONTEXT}, "base": base,
               "inputs": str(inputs), "inputs_sha256": evidence.digest(raw),
               "adapter_sha256": evidence.file_digest(__file__)}
    key = evidence.digest(evidence.canonical(context))
    context_path = root / (key + ".context.json")
    immutable(context_path, evidence.canonical(context))
    for entry in declaration["jobs"].values():
        entry["native"] = [*entry["native"], str(context_path), str(pathlib.Path(__file__).resolve())]
    scoped_inputs = root / (key + ".inputs.json")
    immutable(scoped_inputs, evidence.canonical(declaration))
    started = time.monotonic()
    attempt = root / ("attempt-" + uuid.uuid4().hex)
    # One lock spans lookup, full-hit verification and prospective execution. A
    # first-activation writer cannot race a previously absent summary lookup.
    with evidence.capture_lock(lane, repo):
        scope, chosen = evidence.scope_for(lane, repo, base, scoped_inputs)
        # The unlocked snapshot is only an early refusal. The receipt's actual
        # run-start source must still be the engine's candidate after locking.
        evidence.require(scope["source"]["head"] == os.environ["FARCOOLER_CANDIDATE"],
                         "engine candidate differs from HEAD")
        pointer = lane.summary_path(repo)
        parent, prior = None, {}
        if args.enable_reuse and pointer.is_file():
            try:
                candidate = evidence.pointer_parent(pointer)
                prior = evidence.validate_chain(candidate["directory"], candidate["sha256"], scope)
                parent = candidate
            except (ValueError, OSError, KeyError, TypeError, AttributeError):
                # No legacy/stale/partial assertion can supply coverage. A cold
                # capture below retains the exact pointer without importing it.
                # Malformed JSON receipt shapes can also fail attribute access
                # in ov-903's validator; they are misses, never coverage.
                prior = {}
        reused = [j for j in chosen if evidence.passed(scope["plan"][j], prior.get(j, []))]
        if len(reused) == len(chosen):
            try:
                evidence.check_pointer(repo, pointer)
            except (ValueError, OSError, KeyError, TypeError, AttributeError, subprocess.CalledProcessError):
                # Even complete rows cannot override a failed run completion.
                parent, reused = None, []
        if len(reused) == len(chosen):
            evidence.require(evidence.scope_for(lane, repo, base, scoped_inputs)[0] == scope,
                             "inputs changed during cache lookup")
            code = 0
        else:
            if parent is None and pointer.exists():
                # A miss must execute afresh even when historical evidence is
                # missing/corrupt. Quarantine the exact pointer, never its logs;
                # do not import the broken chain into a new coverage claim.
                previous = pointer.read_bytes()
                previous_sha = evidence.digest(previous)
                archive = root / ("history-" + previous_sha + ".pointer.txt")
                immutable(archive, previous)
                evidence.replace_pointer(pointer, "engine-gate retained-history\n"
                                         f"pointer: {archive}\nsha256: {previous_sha}\n")
            code = evidence._run_capture(lane, repo, base, scoped_inputs, attempt, parent, False,
                                         expected_scope=scope)
        # Metrics are observational, never an alternate success producer. Full
        # hit saved time is not guessed from wall time of partial captures.
        metrics = {"schema": "engine-gate-metrics/v1", "binding": os.environ["FARCOOLER_OPERATION_BINDING"],
                   "scope_sha256": evidence.digest(evidence.canonical(scope)),
                   "required_jobs": len(chosen), "reused_jobs": len(reused),
                   "selected_jobs": len(chosen) - len(reused), "exit": code,
                   "wall_seconds": time.monotonic() - started}
        evidence.private_write(root / ("invocation-" + uuid.uuid4().hex + ".json"), evidence.canonical(metrics))
        print(json.dumps(metrics, sort_keys=True), flush=True)
        return code


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--inputs", required=True)
    ap.add_argument("--evidence-root", required=True)
    ap.add_argument("--trust-scope", required=True, help="reviewed runner/configuration epoch; change on external input drift")
    ap.add_argument("--enable-reuse", action="store_true")
    args = ap.parse_args()
    try:
        evidence.require(bool(args.trust_scope.strip()), "empty trust scope")
        return run(args)
    except (ValueError, OSError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        print(f"engine-gate: refused: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
