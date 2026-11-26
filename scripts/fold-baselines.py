"""Resolve supported baseline conflicts and identical Swift floor bumps.

The daemon runs this bundled source with isolated Python and a private fold
worktree. Output is a JSON object of replacement texts. An empty object
means there are no automatic replacements.
"""
import json
import pathlib
import re
import subprocess
import sys

FLOORS = {"apps/macos/test-floor.txt", "apps/shared/AgentKit/test-floor.txt"}
LAYERS = "scripts/daemon-layers.json"
COUNTS = "scripts/daemon-layers-counts.txt"


class UnsupportedConflict(ValueError):
    """The conflict needs a person to settle it."""


def git(*args):
    result = subprocess.run(["git", "--no-pager", *args], capture_output=True, text=True, timeout=5)
    if result.returncode != 0:
        raise RuntimeError(f"git {args[0]} failed: {result.stderr.strip()}")
    return result.stdout


def stages(path):
    entries = git("ls-files", "--unmerged", "--", path).splitlines()
    modes_and_stages = [(entry.split()[0], entry.split()[2]) for entry in entries]
    if modes_and_stages != [("100644", str(i)) for i in (1, 2, 3)]:
        raise UnsupportedConflict(f"{path} is not a regular content conflict")
    return [git("cat-file", "blob", f":{i}:{path}") for i in (1, 2, 3)]


def merge_value(base, ours, theirs):
    if ours == theirs or theirs == base:
        return ours
    if ours == base:
        return theirs
    raise UnsupportedConflict("both sides changed the same entry")


def merge_entries(base, ours, theirs):
    merged = {}
    for key in dict.fromkeys([*ours, *theirs, *base]):
        value = merge_value(base.get(key), ours.get(key), theirs.get(key))
        if value is not None:
            merged[key] = value
    return merged


def owners(table):
    entries = [*table["modules"].items(), ("composition", table["composition"])]
    result = {}
    for layer, modules in entries:
        if not isinstance(modules, list):
            raise UnsupportedConflict("module lists must be arrays")
        for module in modules:
            if not isinstance(module, str) or module in result:
                raise UnsupportedConflict("module ownership must be unique")
            result[module] = layer
    return result


def merge_layers(versions):
    configs = [json.loads(text) for text in versions]
    if any(set(cfg) != {"about", "allowed", "modules", "composition"} for cfg in configs):
        raise UnsupportedConflict("unknown layer table format")
    if any(not isinstance(cfg["allowed"], dict) or not isinstance(cfg["modules"], dict)
           or not isinstance(cfg["about"], str) for cfg in configs):
        raise UnsupportedConflict("invalid layer table")
    if any(not isinstance(uses, list) or any(not isinstance(use, str) for use in uses)
           for cfg in configs for uses in cfg["allowed"].values()):
        raise UnsupportedConflict("invalid layer allowlist")
    allowed = merge_entries(*(cfg["allowed"] for cfg in configs))
    assignments = merge_entries(*(owners(cfg) for cfg in configs))
    # Preserve each existing list's order. A new module follows the current list.
    layer_names = dict.fromkeys(layer for cfg in [configs[1], configs[2], configs[0]]
                                for layer in cfg["modules"] if layer in allowed)
    modules = {layer: [] for layer in layer_names}
    composition = []
    for name, layer in assignments.items():
        if layer == "composition":
            composition.append(name)
        elif layer in modules:
            modules[layer].append(name)
        else:
            raise UnsupportedConflict(f"{name} has no layer")
    return {"about": merge_value(*(cfg["about"] for cfg in configs)),
            "allowed": allowed, "composition": composition, "modules": modules}


def floor(versions):
    if not all(re.fullmatch("[0-9]+", value.strip()) for value in versions):
        raise UnsupportedConflict("test floor is not a number")
    base, ours, theirs = map(int, versions)
    if min(ours, theirs) < base:
        raise UnsupportedConflict("conflicting test floor decrease")
    # A cherry-pick's base is the picked commit's parent. Add the incoming
    # reviewed increase to the current floor, preserving parameterized cases.
    return f"{ours + (theirs - base)}\n"


def regular_blob(revision, path):
    entry = git("ls-tree", revision, "--", path).partition("\t")[0].split()
    if len(entry) == 3 and entry[:2] == ["100644", "blob"]:
        return entry[2]
    return None


def restore_floor_increases(commit, paths):
    result = {}
    for path in FLOORS.intersection(paths):
        base = regular_blob(f"{commit}^", path)
        ours = regular_blob("HEAD^", path)
        theirs = regular_blob(commit, path)
        if base is None or ours is None or base == ours or ours != theirs:
            continue
        # Git kept a single copy of the identical edit. Verify the picked
        # tree still has that blob before restoring the incoming increase.
        if regular_blob("HEAD", path) != ours:
            continue
        try:
            before, after = (git("cat-file", "blob", blob) for blob in (base, ours))
            replacement = floor([before, after, after])
        except ValueError:
            continue
        if int(replacement) > int(after):
            result[path] = replacement
    return result


def resolve(paths, scanner):
    versions = {path: stages(path) for path in paths}
    result = {}
    config = None
    if LAYERS in versions:
        config = merge_layers(versions[LAYERS])
        if scanner["Layers"](config).problems():
            raise UnsupportedConflict("merged layer table is invalid")
        result[LAYERS] = json.dumps(config, indent=1) + "\n"
    if COUNTS in versions:
        counts = [scanner["read_counts"](text) for text in versions[COUNTS]]
        if any(faults for _, faults in counts):
            raise UnsupportedConflict("invalid daemon layer counts")
        _, (ours, _), (theirs, _) = counts
        if config is None:
            config = json.loads(git("cat-file", "blob", f":{LAYERS}"))
        layers = scanner["Layers"](config)
        root = pathlib.Path.cwd()
        source = root / scanner["SRC"]
        if source.resolve() != source or any(path.is_symlink() for path in source.rglob("*.rs")):
            raise UnsupportedConflict("daemon source contains symbolic links")
        violations, _, refused = scanner["scan"](root, layers)
        actual = scanner["tally"](violations, refused)
        # Both reviewed ceilings still apply. The gate will reject edges above
        # a ceiling, including growth against the pinned main.
        result[COUNTS] = scanner["write_counts"]({
            pair: min(limit, theirs.get(pair, 0), actual.get(pair, 0))
            for pair, limit in ours.items()
        })
    for path in FLOORS.intersection(paths):
        result[path] = floor(versions[path])
    return result


def main(scanner_source, paths):
    if not set(paths) <= FLOORS | {LAYERS, COUNTS}:
        return {}
    scanner = {"__file__": "scripts/daemon-layers.py", "__name__": "daemon_layers"}
    exec(scanner_source, scanner)
    try:
        return resolve(paths, scanner)
    except (ValueError, KeyError, TypeError):
        return {}


if __name__ == "__main__":
    scanner_source, *args = sys.argv[1:]
    if args[:1] == ["--after-pick"]:
        _, commit, *paths = args
        replacements = restore_floor_increases(commit, paths)
    else:
        replacements = main(scanner_source, args)
    print(json.dumps(replacements))
