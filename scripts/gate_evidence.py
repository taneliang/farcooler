#!/usr/bin/env python3
"""Prospective v2 lane receipts. See docs/ci/gate-evidence-v2.md before use.

No v1 import, filters, environment-gap promotion, or manual success input.
Use --plan first; --run explicitly executes the canonical selected jobs.
"""
import argparse
import contextlib
import fcntl
import hashlib
import importlib.util
import json
import os
import pathlib
import shutil
import signal
import subprocess
import sys
import tempfile

VERSION = "lane-gate-evidence-v2"
TEMP = "@GATE_RUNNER_TEMP@"
# Public execution knobs only. No ambient tokens, arbitrary env dump or config capture.
PUBLIC_ENV = {
    "HOME", "PATH", "LANG", "LC_ALL", "TERM", "TMPDIR", "JAVA_HOME",
    "ANDROID_HOME", "ANDROID_NDK_HOME", "ANDROID_SDK_ROOT", "DEVELOPER_DIR",
    "SDKROOT", "CARGO_HOME", "RUSTUP_HOME", "GRADLE_USER_HOME", "GOCACHE",
    "GOMODCACHE", "NPM_CONFIG_CACHE", "CARGO_TARGET_DIR",
}
DERIVED_ENV = {
    "CI", "RUNNER_TEMP", "RUNNER_OS", "GITHUB_SHA", "GITHUB_WORKSPACE",
    "FC_LEAK_SHARED", "FC_LEAK_STRICT", "CAPTURE_JDK",
    "CARGO_PROFILE_DEV_DEBUG", "CARGO_PROFILE_TEST_DEBUG",
}
JOB_TOOLS = {
    "wire": ["python3"], "doc-comments": ["python3"],
    "relay": ["node", "npm"], "site": ["node", "npm"],
    "tunnel": ["go", "gofmt"],
    "rust": ["cargo", "rustc", "cargo-nextest", "go", "git"],
    "swift-tests": ["swift", "xcodebuild", "cargo", "rustc", "git"],
    "swift": ["swift", "xcodebuild", "cargo", "rustc", "go", "git"],
    "ios": ["swift", "xcodebuild", "cargo", "rustc", "go", "git"],
    "android": ["cargo", "rustc", "go", "git"],
    "web": ["node", "npm", "cargo", "rustc"],
}


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def file_digest(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def require(ok, message):
    if not ok:
        raise ValueError(message)


def git(repo, *args):
    return subprocess.check_output(["git", "-C", str(repo), *args])


def load_lane():
    path = pathlib.Path(__file__).with_name("lane-gate.py")
    sys.path.insert(0, str(path.parent))
    spec = importlib.util.spec_from_file_location("lane_gate_v2", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def source_snapshot(repo):
    """No source bytes are archived. Missing files, modes and index state matter."""
    # status alone honors hidden-index flags and core.fileMode/fsmonitor caches.
    # Such a "clean" checkout can contain an uncommitted fix to a broken HEAD.
    for option in ("-v", "-f"):
        flags = git(repo, "ls-files", option, "-z").split(b"\0")
        require(all(not entry or entry.startswith(b"H ") for entry in flags),
                "unsupported hidden/sparse/fsmonitor index flags; cannot attest exact SHA")
    head = git(repo, "rev-parse", "HEAD").decode().strip()
    index = {}
    for entry in git(repo, "ls-files", "--stage", "-z").split(b"\0"):
        if entry:
            metadata, name = entry.split(b"\t", 1)
            mode, blob, stage = metadata.split()
            require(stage == b"0" and mode in (b"100644", b"100755"), "unsupported/conflicted index entry")
            index[name] = (mode, blob)
    committed = {}
    for entry in git(repo, "ls-tree", "-r", "-z", head).split(b"\0"):
        if entry:
            metadata, name = entry.split(b"\t", 1)
            mode, kind, blob = metadata.split()
            require(kind == b"blob", "submodules are unsupported v2 inputs")
            committed[name] = (mode, blob)
    require(index == committed, "index differs from committed HEAD")
    # Hash through Git's clean filters (e.g. LFS), independently of index stat
    # caches. C-style quoted stdin paths handle whitespace/newlines/non-ASCII.
    ordered = sorted(index)
    quoted = b"".join(b'"' + b"".join(bytes([c]) if 32 <= c < 127 and c not in (34, 92)
                                    else ("\\%03o" % c).encode() for c in name) + b'"\n' for name in ordered)
    hashes = subprocess.check_output(["git", "-C", str(repo), "hash-object", "--stdin-paths"], input=quoted).splitlines()
    require(len(hashes) == len(ordered) and all(blob == index[name][1] for name, blob in zip(ordered, hashes)),
            "tracked content differs from committed HEAD (even if git status says clean)")
    names = set(git(repo, "ls-files", "-z").split(b"\0"))
    names.update(git(repo, "ls-files", "--others", "--exclude-standard", "-z").split(b"\0"))
    files = {}
    for raw in sorted(names - {b""}):
        name = os.fsdecode(raw)
        path = repo / name
        if path.is_symlink():
            # A source symlink can hide external inputs. Do not silently cover it.
            raise ValueError(f"v2 source symlink is unsupported: {name}")
        if not path.exists():
            files[name] = None
        else:
            require(path.is_file(), f"unsupported source input: {name}")
            if raw in index:
                require(bool(path.stat().st_mode & 0o111) == (index[raw][0] == b"100755"),
                        f"filesystem executable mode differs from HEAD: {name}")
            files[name] = [bool(path.stat().st_mode & 0o111), file_digest(path)]
    require(git(repo, "rev-parse", "HEAD").decode().strip() == head, "HEAD changed during snapshot")
    return {
        "head": head,
        "tree": git(repo, "rev-parse", "HEAD^{tree}").decode().strip(),
        "index": digest(git(repo, "ls-files", "--stage", "-z")),
        "status": digest(git(repo, "-c", "core.fsmonitor=false", "-c", "core.untrackedCache=false",
                             "-c", "core.fileMode=true", "status", "--porcelain=v1", "-z", "--untracked-files=all")),
        "files": files,
    }


def public_file(path):
    """Explicit public prerequisite files only; never walk HOME or cache trees."""
    path = pathlib.Path(path).expanduser().absolute()
    require(path.is_file(), f"missing public prerequisite file: {path}")
    return {"path": str(path), "resolved": str(path.resolve()), "sha256": file_digest(path),
            "executable": bool(path.stat().st_mode & 0o111)}


def android_packages(plans):
    import re
    setup = "\n".join(s.run for s in plans["android"] if s.skip)
    packages = re.findall(r"'((?:ndk|platforms|build-tools);[^']+)'", setup)
    require({p.split(";")[0] for p in packages} == {"ndk", "platforms", "build-tools"},
            "cannot determine Android SDK packages from CI")
    return packages


def android_environment(env, plans):
    sdk = pathlib.Path(env.get("ANDROID_HOME", ""))
    require(sdk.is_absolute(), "Android needs explicit ANDROID_HOME")
    ndk = next(p.split(";", 1)[1] for p in android_packages(plans) if p.startswith("ndk;"))
    pinned = (sdk / "ndk" / ndk).resolve()
    require(not env.get("ANDROID_NDK_HOME") or pathlib.Path(env["ANDROID_NDK_HOME"]).resolve() == pinned,
            "ANDROID_NDK_HOME differs from canonical CI NDK")
    return {**env, "ANDROID_NDK_HOME": str(pinned)}


def prerequisites(lane, repo, chosen, plans, environments, declaration):
    require(set(declaration) == {"schema", "jobs"} and declaration["schema"] == 1,
            "input declaration must have schema 1 and jobs")
    require(set(declaration["jobs"]) == set(chosen), "declare every selected job exactly once")
    result = {}
    for job in chosen:
        entry = declaration["jobs"][job]
        require(set(entry) == {"tools", "sdk", "cache", "native"}, f"incomplete inputs for {job}")
        require(all(isinstance(v, list) and all(isinstance(p, str) and p for p in v)
                    for v in entry.values()), f"invalid input lists for {job}")
        require(bool(entry["tools"]), f"no declared tools for {job}")
        if job in lane.HEAVY:
            require(all(entry[k] for k in ("sdk", "cache", "native")),
                    f"{job} needs explicit SDK, immutable cache and native input files")
        env = environments[job]
        tools = []
        for name in ["bash", *JOB_TOOLS[job], *entry["tools"]]:
            resolved = shutil.which(name, path=env.get("PATH", ""))
            require(resolved, f"missing tool for {job}: {name}")
            tools.append(public_file(resolved))
        result[job] = {"tools": tools, **{k: [public_file(p) for p in entry[k]]
                                        for k in ("sdk", "cache", "native")}}
        if "rustc" in JOB_TOOLS[job]:
            # rustup shims alone do not identify the selected compiler/toolchain.
            for tool in ("rustc", "cargo"):
                output = subprocess.check_output(["rustup", "which", tool], cwd=repo, env=env, text=True).strip()
                result[job]["tools"].append(public_file(output))
        if "xcodebuild" in JOB_TOOLS[job]:
            for tool in ("swift", "clang"):
                output = subprocess.check_output(["xcrun", "--find", tool], cwd=repo, env=env, text=True).strip()
                result[job]["tools"].append(public_file(output))
            result[job]["xcode_version"] = subprocess.check_output(["xcodebuild", "-version"], env=env, text=True)
        if job == "android":
            for key in ("JAVA_HOME", "CAPTURE_JDK"):
                if key in env:
                    for file in ("release", "bin/java", "bin/javac", "lib/modules"):
                        result[job]["tools"].append(public_file(pathlib.Path(env[key]) / file))
        for step in plans[job]:
            if step.skip:
                continue
            reason = lane.blocker(step, repo, {**env, **step.env})
            require(not reason, f"preflight {job}: {reason}")
        if job == "android":
            # Match SDK packages selected by canonical CI, not a separate version pin.
            packages = android_packages(plans)
            sdk = pathlib.Path(env.get("ANDROID_HOME", ""))
            require(sdk.is_absolute(), "Android needs explicit ANDROID_HOME")
            paths = []
            for package in packages:
                kind, version = package.split(";", 1)
                suffixes = {"ndk": ["source.properties"], "platforms": ["android.jar"],
                            "build-tools": ["source.properties", "aapt2", "zipalign", "apksigner"]}[kind]
                paths.extend(public_file(sdk / kind / version / suffix) for suffix in suffixes)
            result[job]["ci_sdk"] = paths
            selected = subprocess.check_output(
                ["bash", "-e", "-c", '. scripts/android-ndk.sh\nandroid_toolchain\n'
                 'ndk_cc aarch64-linux-android\nprintf "%s\\n" "$TOOLCHAIN/bin/clang" "$TOOLCHAIN/bin/llvm-ar"'],
                cwd=repo, env=env, text=True).splitlines()
            require(len(selected) == 3, "cannot resolve actual selected NDK tools")
            result[job]["selected_ndk_tools"] = [public_file(p) for p in selected]
    return result


def scope_for(lane, repo, base, inputs_path, strict_leaks=False):
    repo = pathlib.Path(repo).resolve()
    source = source_snapshot(repo)
    require(source["status"] == digest(b""),
            "v2 exact-SHA evidence requires clean tracked/index/untracked inputs; commit source first")
    inputs_path = pathlib.Path(inputs_path).resolve()
    declaration_bytes = inputs_path.read_bytes()
    declaration = json.loads(declaration_bytes)
    base_sha = git(repo, "rev-parse", "--verify", base + "^{commit}").decode().strip()
    ctx = lane.make_ctx(repo, pathlib.Path(TEMP), base_sha)
    workflows = {"ci": (repo / ".github/workflows/ci.yml").read_text(),
                 "ios-ui-build": (repo / ".github/actions/ios-ui-build/action.yml").read_text()}
    require(all(j in lane.CI_JOB_ALIASES or j in lane.CI_JOBS_NOT_RUN for j in lane.job_ids(workflows["ci"])),
            "CI has jobs the canonical runner does not cover")
    plans = {j: lane.steps_for(j, workflows, ctx) for j in lane.JOBS}
    changes = lane.load_module("v2_changes", repo / "scripts/ci-changes.py")
    ios = lane.load_module("v2_ios", repo / "scripts/ios-ui-plan.py")
    reach = lane.jobs_for(lane.changed_files(repo, base_sha), changes.SITE,
                         lambda f: bool(ios.RELEVANT.match(f)),
                         {j: "\n".join(s.run for s in steps if not s.skip) for j, steps in plans.items()},
                         changes.families)
    chosen = [j for j in lane.JOBS if j in reach]
    require(all(any(not s.skip for s in plans[j]) for j in chosen), "selected job has no executable steps")
    environments = {}
    for job in chosen:
        env = lane.base_env(repo, pathlib.Path(TEMP), ctx, strict_leaks)
        env = {k: v for k, v in env.items() if k in PUBLIC_ENV | DERIVED_ENV}
        env.update(lane.job_env(job))
        if job == "rust":
            env.update(lane.hl.lean_cargo_env(repo))  # no target trimming or mutation
        if job == "android" and ctx["steps.jdk21.outputs.path"]:
            env["CAPTURE_JDK"] = ctx["steps.jdk21.outputs.path"]
        if job == "android":
            # Bind actual discovery to CI even if a newer NDK is also installed.
            env = android_environment(env, plans)
        environments[job] = env
    implementations = {}
    for name in ("gate_evidence.py", "lane-gate.py", "heavy_lock.py"):
        path = pathlib.Path(__file__).with_name(name)
        require(path.read_bytes() == (repo / "scripts" / name).read_bytes(),
                f"runner differs from checkout: {name}")
        implementations[name] = file_digest(path)
    scope = {
        "checkout": str(repo), "source": source, "base_ref": base, "required": chosen,
        "base_sha": base_sha, "merge_base": git(repo, "merge-base", base_sha, "HEAD").decode().strip(),
        "plan": {j: [{"name": s.name, "run": s.run, "cwd": s.cwd, "env": s.env, "skip": s.skip}
                     for s in plans[j]] for j in chosen},
        "environments": environments, "implementations": implementations,
        "inputs_path": str(inputs_path), "inputs_sha256": digest(declaration_bytes),
        "prerequisites": prerequisites(lane, repo, chosen, plans, environments, declaration),
        "strict_leaks": strict_leaks,
        "host": {"platform": sys.platform, "machine": os.uname().machine,
                 "node": os.uname().nodename, "python": public_file(sys.executable)},
    }
    require(source_snapshot(repo) == source, "source changed during preflight")
    return scope, chosen


def summary_text(data):
    rows = [VERSION, f"head: {data['scope']['source']['head']}"]
    for job in data["selected"]:
        for receipt in data["jobs"][job]:
            rows.append(json.dumps({"job": job, **receipt}, sort_keys=True))
    rows.append("error: " + str(data["error"]))
    rows.append("invalidated: " + str(data["invalidated"]))
    return "\n".join(rows) + "\n"


def private_write(path, data):
    with open(path, "xb") as f:
        os.chmod(path, 0o600)
        f.write(data)
        f.flush()
        os.fsync(f.fileno())


def read_bound(root, name, sha):
    require(pathlib.PurePath(name).name == name, "unsafe evidence filename")
    path = root / name
    require(not path.is_symlink() and path.is_file(), f"missing/unsafe evidence: {path}")
    raw = path.read_bytes()
    require(digest(raw) == sha, f"evidence digest mismatch: {path}")
    return raw


def passed(plan, receipts):
    return bool(plan) and len(receipts) == len(plan) and all(
        r == {"index": i, "skip": s["skip"]} if s["skip"] else
        type(r.get("exit")) is int and r["exit"] == 0 and r.get("error") is None
        for i, (s, r) in enumerate(zip(plan, receipts)))


def validate_chain(directory, sha, scope, seen=None, budget=None):
    """Return latest job attempts; validate all old failures, not only reused rows."""
    directory = pathlib.Path(directory)
    seen = set() if seen is None else seen
    budget = [0] if budget is None else budget
    budget[0] += 1
    require(budget[0] <= 256, "oversized evidence graph")
    require(directory.is_absolute() and not directory.is_symlink(), "unsafe evidence directory")
    key = str(directory.resolve())
    require(key not in seen and len(seen) < 64, "cyclic/oversized evidence chain")
    seen.add(key)
    data = json.loads(read_bound(directory, "manifest.json", sha))
    require(set(data) == {"schema", "scope", "parent", "selected", "jobs", "error", "invalidated",
                         "summary_sha256", "start_sha256", "previous_pointer_sha256", "runner_temp"},
            "unrecognized/incomplete evidence manifest")
    require(data["schema"] == VERSION and data["scope"] == scope, "v1/stale/mismatched evidence scope")
    require(isinstance(data["runner_temp"], str) and pathlib.Path(data["runner_temp"]).is_absolute(),
            "missing actual runner temporary location")
    require(type(data["invalidated"]) is bool and (data["error"] is None or isinstance(data["error"], str)),
            "invalid run completion state")
    prior = {}
    if data["parent"] is not None:
        parent = data["parent"]
        require(set(parent) == {"directory", "sha256"}, "invalid evidence parent")
        prior = validate_chain(parent["directory"], parent["sha256"], scope, set(seen), budget)
    needed = [j for j in scope["required"] if not passed(scope["plan"][j], prior.get(j, []))]
    require(data["selected"] == needed and set(data["jobs"]) == set(needed),
            "partial/duplicate/unexpected selected jobs")
    for job in needed:
        receipts = data["jobs"][job]
        require(isinstance(receipts, list) and len(receipts) <= len(scope["plan"][job]), "extra/invalid rows")
        env = dict(scope["environments"][job])
        for i, receipt in enumerate(receipts):
            step = scope["plan"][job][i]
            require(type(receipt.get("index")) is int and receipt["index"] == i, "missing/duplicate/out-of-order step")
            if step["skip"]:
                require(receipt == {"index": i, "skip": step["skip"]}, "skip cannot claim execution")
                continue
            require(set(receipt) == {"index", "command", "cwd", "env", "exit", "log_sha256", "updates", "paths", "error"},
                    "incomplete execution receipt")
            require(receipt["command"] == step["run"] and receipt["cwd"] == step["cwd"] and
                    receipt["env"] == {**env, **step["env"]}, "command/cwd/environment mismatch")
            require(receipt["exit"] is None or type(receipt["exit"]) is int, "exit must be numeric or unknown")
            require(receipt["error"] is None or isinstance(receipt["error"], str), "invalid execution error")
            read_bound(directory, f"{job}-{i}.log", receipt["log_sha256"])
            require(set(receipt["updates"]) <= {"APP_NAME"} and
                    all(isinstance(v, str) for v in receipt["updates"].values()) and
                    isinstance(receipt["paths"], list) and all(isinstance(v, str) for v in receipt["paths"]),
                    "unrecognized environment output")
            env.update(receipt["updates"])
            for path in receipt["paths"]:
                env["PATH"] = path + os.pathsep + env.get("PATH", "")
        prior[job] = [] if data["invalidated"] else receipts
    summary = read_bound(directory, "summary.txt", data["summary_sha256"])
    require(summary == summary_text(data).encode(), "summary does not describe original execution")
    start = {k: v for k, v in data.items() if k not in {"summary_sha256", "start_sha256"}}
    start.update(jobs={j: [] for j in needed}, error=None, invalidated=False)
    require(read_bound(directory, "start.json", data["start_sha256"]) == canonical(start),
            "run-start capture mismatch")
    if data["previous_pointer_sha256"] is not None:
        previous = read_bound(directory, "previous-pointer.txt", data["previous_pointer_sha256"])
        # Fresh full captures may have a different scope. Still retain and audit
        # the old v2 history; it supplies no current coverage unless it is parent.
        if previous.startswith(b"lane-gate-summary v2\n"):
            old = pointer_parent(directory / "previous-pointer.txt")
            if old != data["parent"]:
                old_data = json.loads(read_bound(pathlib.Path(old["directory"]), "manifest.json", old["sha256"]))
                validate_chain(old["directory"], old["sha256"], old_data["scope"], set(seen), budget)
    return prior


def expand_temp(value, tmp):
    return value.replace(TEMP, str(tmp))


def execute_step(step, index, env, repo, tmp, log):
    receipt = {"index": index, "command": step["run"], "cwd": step["cwd"],
               "env": {**env, **step["env"]}, "exit": None, "log_sha256": "",
               "updates": {}, "paths": [], "error": None}
    gh_env, gh_path = tmp / "github_env", tmp / "github_path"
    gh_env.write_text("")
    gh_path.write_text("")
    execution_env = {k: expand_temp(v, tmp) for k, v in receipt["env"].items()}
    execution_env.update(GITHUB_ENV=str(gh_env), GITHUB_PATH=str(gh_path))
    # Numeric subprocess status is the only success producer. Output is never parsed for green.
    with open(log, "xb") as out:
        os.chmod(log, 0o600)
        process = None
        try:
            process = subprocess.Popen(["bash", "-e", "-c", expand_temp(step["run"], tmp)],
                                       cwd=repo / (step["cwd"] or "."), env=execution_env,
                                       stdout=out, stderr=subprocess.STDOUT, start_new_session=True)
            receipt["exit"] = process.wait()
        except BaseException as error:
            receipt["error"] = f"{type(error).__name__}: {error}"
            if process is not None:
                with contextlib.suppress(ProcessLookupError):
                    os.killpg(process.pid, signal.SIGTERM)
                try:
                    receipt["exit"] = process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    with contextlib.suppress(ProcessLookupError):
                        os.killpg(process.pid, signal.SIGKILL)
                    receipt["exit"] = process.wait()
        finally:
            out.flush()
            receipt["log_sha256"] = file_digest(log)
    try:
        lines = gh_env.read_text().splitlines()
        require(all("=" in line for line in lines), "unsupported GITHUB_ENV syntax")
        updates = dict(line.split("=", 1) for line in lines)
        require(set(updates) <= {"APP_NAME"}, "unrecognized GITHUB_ENV output")
        paths = gh_path.read_text().splitlines()
    except Exception as error:
        receipt["error"] = f"environment output protocol refused: {type(error).__name__}"
        return receipt  # retain the actual numeric exit, including zero
    receipt["updates"] = updates
    receipt["paths"] = paths
    env.update(updates)
    for path in receipt["paths"]:
        env["PATH"] = path + os.pathsep + env.get("PATH", "")
    return receipt


@contextlib.contextmanager
def capture_lock(lane, repo):
    path = lane.summary_path(repo).with_name("gate-evidence.lock")
    with open(path, "a+b") as lock:
        os.chmod(path, 0o600)
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield


def capture_active(gitdir):
    path = pathlib.Path(gitdir) / "gate-evidence.lock"
    if not path.exists():
        return False
    try:
        with open(path, "rb") as lock:
            fcntl.flock(lock, fcntl.LOCK_SH | fcntl.LOCK_NB)
        return False
    except BlockingIOError:
        return True


def replace_pointer(dest, text):
    fd, name = tempfile.mkstemp(prefix="gate-v2-pointer-", dir=dest.parent)
    try:
        with os.fdopen(fd, "wb") as out:
            out.write(text.encode())
            out.flush()
            os.fsync(out.fileno())
        os.replace(name, dest)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def run_capture(lane, repo, base, inputs_path, directory, parent=None, strict_leaks=False):
    with capture_lock(lane, repo):
        return _run_capture(lane, repo, base, inputs_path, directory, parent, strict_leaks)


def _run_capture(lane, repo, base, inputs_path, directory, parent, strict_leaks):
    scope, chosen = scope_for(lane, repo, base, inputs_path, strict_leaks)
    prior = validate_chain(parent["directory"], parent["sha256"], scope) if parent else {}
    selected = [j for j in chosen if not passed(scope["plan"][j], prior.get(j, []))]
    require(selected, "all jobs already passed; no retry to execute")
    directory = pathlib.Path(directory).resolve()
    repo = pathlib.Path(repo).resolve()
    require(not directory.is_relative_to(repo), "evidence must be outside the checkout")
    directory.mkdir(mode=0o700)  # exclusive: never overwrite any attempt
    data = {"schema": VERSION, "scope": scope, "parent": parent, "selected": selected,
            "jobs": {j: [] for j in selected}, "error": None, "invalidated": False,
            "previous_pointer_sha256": None}
    dest = lane.summary_path(repo)
    if dest.exists():
        previous = dest.read_bytes()
        private_write(directory / "previous-pointer.txt", previous)
        data["previous_pointer_sha256"] = digest(previous)
    temporary = tempfile.TemporaryDirectory(prefix="gate-v2-")
    data["runner_temp"] = temporary.name
    private_write(directory / "start.json", canonical(data))
    (directory / "start.json").chmod(0o400)
    # A killed/unsealed newer attempt must never leave an older green current.
    replace_pointer(dest, f"lane-gate-summary v2 pending\nevidence: {directory}\n")
    with temporary as temporary_path:
        tmp = pathlib.Path(temporary_path)
        try:
            heavy = any(j in lane.HEAVY for j in selected)
            with lane.machine_lock(repo, "v2 gate: " + ", ".join(selected)) if heavy else contextlib.nullcontext():
                # Revalidate after waiting; a queued snapshot is not permission to run changed inputs.
                require(scope_for(lane, repo, base, inputs_path, strict_leaks)[0] == scope,
                        "inputs changed while queued")
                for job in selected:
                    if job in lane.HEAVY:
                        lane.wait_for_load()
                    env = dict(scope["environments"][job])
                    for i, step in enumerate(scope["plan"][job]):
                        print(f"v2 {job} step {i}: {step['run'].strip()}", flush=True)
                        if step["skip"]:
                            receipt = {"index": i, "skip": step["skip"]}
                        else:
                            receipt = execute_step(step, i, env, repo, tmp, directory / f"{job}-{i}.log")
                        data["jobs"][job].append(receipt)
                        if receipt.get("error"):
                            raise RuntimeError(receipt["error"])
        except BaseException as error:
            data["error"] = f"{type(error).__name__}: {error}"
        finally:
            try:
                data["invalidated"] = scope_for(lane, repo, base, inputs_path, strict_leaks)[0] != scope
            except Exception:
                data["invalidated"] = True
    private_write(directory / "summary.txt", summary_text(data).encode())
    data["summary_sha256"] = file_digest(directory / "summary.txt")
    data["start_sha256"] = file_digest(directory / "start.json")
    private_write(directory / "manifest.json", canonical(data))
    for path in directory.iterdir():
        path.chmod(0o400)
    sha = file_digest(directory / "manifest.json")
    coverage = validate_chain(str(directory), sha, scope)
    complete = not data["error"] and not data["invalidated"] and all(
        passed(scope["plan"][j], coverage.get(j, [])) for j in chosen)
    pointer = f"lane-gate-summary v2\nevidence: {directory}\nsha256: {sha}\n"
    # Preserve any legacy/current pointer before replacing it; never edit older evidence.
    replace_pointer(dest, pointer)
    print(f"v2 evidence: {directory}; {'passed' if complete else 'FAILED'}")
    return 0 if complete else 1


def pointer_parent(path):
    lines = pathlib.Path(path).read_text().splitlines()
    require(len(lines) == 3 and lines[0] == "lane-gate-summary v2" and
            lines[1].startswith("evidence: ") and lines[2].startswith("sha256: "),
            "resume/check requires an exact v2 pointer; v1 cannot be imported")
    return {"directory": lines[1][10:], "sha256": lines[2][8:]}


def check_pointer(repo, pointer):
    parent = pointer_parent(pointer)
    raw = read_bound(pathlib.Path(parent["directory"]), "manifest.json", parent["sha256"])
    data = json.loads(raw)
    require(data.get("schema") == VERSION, "not prospective v2 evidence")
    old = data["scope"]
    current, chosen = scope_for(load_lane(), repo, old["base_ref"], old["inputs_path"], old["strict_leaks"])
    require(current == old, "stale SHA/base/CI/inputs/environment evidence")
    coverage = validate_chain(parent["directory"], parent["sha256"], current)
    require(not data["error"] and not data["invalidated"] and
            all(passed(current["plan"][j], coverage.get(j, [])) for j in chosen),
            "failed/unknown/unexecuted v2 job rows")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--worktree", default=".")
    ap.add_argument("--base", default="origin/main")
    ap.add_argument("--inputs", required=True, help="reviewed public prerequisite JSON file")
    ap.add_argument("--evidence-dir", help="new private directory outside the checkout")
    ap.add_argument("--resume", help="previous v2 summary pointer (never a v1 summary)")
    ap.add_argument("--strict-leaks", action="store_true")
    mode = ap.add_mutually_exclusive_group(required=True)
    mode.add_argument("--plan", action="store_true")
    mode.add_argument("--run", action="store_true")
    args = ap.parse_args(argv)
    try:
        lane = load_lane()
        repo = pathlib.Path(args.worktree).resolve()
        parent = pointer_parent(args.resume) if args.resume else None
        if args.plan:
            scope, chosen = scope_for(lane, repo, args.base, args.inputs, args.strict_leaks)
            prior = validate_chain(parent["directory"], parent["sha256"], scope) if parent else {}
            print(json.dumps({"scope_sha256": digest(canonical(scope)), "required": chosen,
                              "execute": [j for j in chosen if not passed(scope["plan"][j], prior.get(j, []))]}, indent=2))
            return 0
        require(args.evidence_dir, "--run requires a new --evidence-dir")
        return run_capture(lane, repo, args.base, args.inputs, args.evidence_dir, parent, args.strict_leaks)
    except (ValueError, OSError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        print(f"gate-evidence: refused: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
