#!/usr/bin/env python3
"""ov-1160: maintained nextest selection for per-push macOS Rust.

--check runs on Linux too, before a missing Mac test can land. A new source
scope mentioning target_os=macos (including cfg!, cfg_attr, all/any and file
level cfg) must be selected in its entirety. No Rust parser guessing where
an attribute ends: select the whole top-level module, or whole package for
crate roots. Explicit #[path] test files belong to the module which includes them.
The JSON also records native coverage which has no explicit cfg in our code
(e.g. notify's FSEvents backend). Linux and nightly Mac run everything.
"""
import argparse
import copy
import json
from pathlib import Path
import re
import sys
import tempfile
try:
    import tomllib
except ImportError:
    sys.exit('macos-rust-tests.py requires Python 3.11 or later (stdlib tomllib)')

ROOT = Path(__file__).resolve().parent.parent
SELECTION = ROOT / 'scripts/macos-rust-tests.json'
MAC = re.compile(r'(?:target_os\s*=\s*"macos"|target_vendor\s*=\s*"apple")')


def scope(path, aliases=None, seen=(), roots=()):
    if path in roots:
        return 'packages', None
    aliases = aliases or {}
    if path in seen:
        return 'packages', None
    if path in aliases:
        if aliases[path] is None:
            return 'packages', None
        owner, name = aliases[path]
        if owner in roots or owner.as_posix() in ('src/lib.rs', 'src/main.rs'):
            return 'modules', name
        return scope(owner, aliases, (*seen, path), roots)
    parts = path.parts
    if parts[0] == 'tests':
        # Shared integration helpers can be included by many binaries.
        return ('binaries', path.stem) if len(parts) == 2 else ('packages', None)
    if parts[0] == 'src' and parts[1] not in ('lib.rs', 'main.rs', 'bin'):
        return 'modules', parts[1].removesuffix('.rs')
    return 'packages', None


def covered(selection, package, kind, name):
    return package in selection['packages'] or name in selection.get(kind, {}).get(package, {})


def crate_manifests(root):
    """Read workspace members (including globs) and crates outside that list."""
    manifests = set((root / 'crates').rglob('Cargo.toml'))
    workspace = root / 'Cargo.toml'
    config = tomllib.loads(workspace.read_text()).get('workspace', {}) if workspace.exists() else {}
    if workspace.exists():
        manifests.add(workspace)
    for member in config.get('members', []):
        manifests.update(root.glob(member + '/Cargo.toml'))
    crates = {}
    pending = sorted(manifest.resolve() for manifest in manifests)
    seen = set()
    while pending:
        manifest = pending.pop()
        if manifest in seen:
            continue
        seen.add(manifest)
        data = tomllib.loads(manifest.read_text())
        if 'package' in data:
            crates[manifest] = data
            # Path dependencies within the workspace can be implicit members,
            # without appearing in members or living below crates/.
            pending.extend(dependency for dependency, _ in local_dependencies(
                manifest, data, config.get('dependencies', {}), root)
                           if dependency.is_relative_to(root) and dependency.exists())
    return crates, config.get('dependencies', {})


def local_dependencies(manifest, data, workspace_deps, root):
    for target, table in [(None, data), *data.get('target', {}).items()]:
        mac = bool(target and (MAC.search(target) or '-apple-darwin' in target))
        for kind in ('dependencies', 'dev-dependencies', 'build-dependencies'):
            for name, spec in table.get(kind, {}).items():
                if not isinstance(spec, dict):
                    continue
                base = manifest.parent
                if spec.get('workspace'):
                    spec = workspace_deps.get(name, {})
                    base = root
                if isinstance(spec, dict) and 'path' in spec:
                    yield (base / spec['path'] / 'Cargo.toml').resolve(), mac


def mac_dependency_crates(crates, workspace_deps, root):
    """Find local crates whose incoming dependency paths all require a Mac.

    Cargo's workspace dependency declarations are templates, not incoming
    edges. Resolve them at each consumer, retaining that consumer's target.
    Include optional, dev and build edges so features cannot hide a consumer.
    Treat any Apple/Mac target mention as native conservatively (including
    mixed predicates); an extra whole-package run is safer than an omission.
    """
    incoming = {manifest: [] for manifest in crates}
    edges = []
    for manifest, data in crates.items():
        for dependency, mac in local_dependencies(manifest, data, workspace_deps, root):
            if dependency in crates:
                incoming[dependency].append((manifest, mac))
                edges.append((manifest, dependency, mac))
    # Start at Mac-target edges, then follow transitive paths. Prune anything
    # with a shared consumer until stable. This also handles dependency cycles
    # without accidentally classifying an unrelated, unreferenced crate.
    native = {dependency for _, dependency, mac in edges if mac}
    while True:
        expanded = native | {dependency for parent, dependency, _ in edges if parent in native}
        if expanded == native:
            break
        native = expanded
    while True:
        shared = {dependency for dependency in native
                  if any(not mac and parent not in native for parent, mac in incoming[dependency])}
        if not shared:
            break
        native -= shared
    return native


def check(root, selection):
    root = root.resolve()
    errors = []
    # Scan every crate, not only today's workspace list: a new Mac-only crate
    # or a crate-level cfg must demand a package selection too.
    manifests, workspace_deps = crate_manifests(root)
    native = mac_dependency_crates(manifests, workspace_deps, root)
    for manifest, data in manifests.items():
        body = manifest.read_text()
        package = data['package']['name']
        if manifest in native and package not in selection['packages']:
            errors.append(f'{manifest.relative_to(root)}: Mac-only dependency crate requires whole package {package}')
        crate = manifest.parent
        # Custom Cargo target roots have root-level test names too; a file
        # named src/darwin.rs is not necessarily a `darwin::` module.
        roots = {Path('src/lib.rs'), Path('src/main.rs')}
        roots.update(Path(p) for p in re.findall(r'\bpath\s*=\s*"([^"]+\.rs)"', body))
        sources = {p.relative_to(crate): p.read_text() for p in sorted(crate.rglob('*.rs'))
                   if 'target' not in p.relative_to(crate).parts}
        aliases = {}
        for path, content in sources.items():
            for relative, name in re.findall(r'#\[path\s*=\s*"([^"]+)"\]\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;', content):
                target = (crate / path.parent / relative).resolve()
                try:
                    target = target.relative_to(crate.resolve())
                except ValueError:
                    if target.exists() and MAC.search(target.read_text()) and package not in selection['packages']:
                        errors.append(f'{target}: shared Mac-specific source requires whole package {package}')
                    continue
                owner = (path, name)
                aliases[target] = owner if target not in aliases or aliases[target] == owner else None
        for source, content in sources.items():
            if MAC.search(content):
                kind, name = scope(source, aliases, roots=roots)
                if not covered(selection, package, kind, name):
                    errors.append(f'{(crate / source).relative_to(root)}: select {package} {kind} {name or "(whole crate)"}')
        # A shared caller's Mac dependency table needs native coverage, but
        # does not make the caller Mac-only. Complete selection for the actual
        # Mac-only dependency is enforced independently above.
        if MAC.search(body) and not (package in selection['packages'] or any(
                selection.get(kind, {}).get(package) for kind in ('modules', 'binaries'))):
            errors.append(f'{manifest.relative_to(root)}: Mac-specific configuration has no native selection')
    return errors


def expression(selection):
    terms = [f'package(={p})' for p in sorted(selection['packages'])]
    for package, modules in sorted(selection['modules'].items()):
        for module in sorted(modules):
            terms.append(f'(package(={package}) and test(/^{module}::/))')
    for package, binaries in sorted(selection['binaries'].items()):
        for binary in sorted(binaries):
            terms.append(f'(package(={package}) and binary(={binary}))')
    return ' or '.join(terms)



def workflow_errors(ci, nightly):
    errors = []
    linux = ci.split('  rust-tests:', 1)[-1].split('\n  rust-macos-tests:', 1)[0]
    if ('cargo nextest run --profile ci --no-fail-fast --archive-file' not in linux
            or ' -E ' in linux or '--partition hash:${{ matrix.shard }}/4' not in linux
            or 'shard: [1, 2, 3, 4]' not in linux or 'scripts/rust-shards.py --archive' not in linux):
        errors.append('Linux must retain the full workspace suite with checked partition coverage')
    mac = ci.split('      - name: macOS native Rust tests', 1)[-1].split('      - uses:', 1)[0]
    if "if: runner.os == 'macOS'" not in mac or 'cargo nextest run --profile ci --workspace --no-run' not in mac or 'macos-rust-tests.py' not in mac:
        errors.append('Mac must build the full workspace and run the selection')
    night = nightly.split('  rust-macos:', 1)[-1].split('\n  #', 1)[0]
    if 'runs-on: macos-latest' not in night or 'cargo nextest run --profile ci --workspace --no-fail-fast' not in night or ' -E ' in night:
        errors.append('Nightly must retain the full Mac workspace suite')
    return errors


def self_test():
    selected = {'packages': {}, 'modules': {}, 'binaries': {}}
    dependency_self_test()
    with tempfile.TemporaryDirectory(prefix='ov1160-') as tmp:
        root = Path(tmp)
        (root / 'Cargo.toml').write_text('[workspace]\nmembers = ["platform/new-mac"]\n')
        crate = root / 'platform/new-mac'
        (crate / 'src').mkdir(parents=True)
        (crate / 'Cargo.toml').write_text('[package]\nname = "new-mac"\n')
        file = crate / 'src/new.rs'
        # Each planted uncovered case must be RED before it is selected.
        for cfg in ('#[cfg(target_os = "macos")]', '#[cfg(all(test, target_os = "macos"))]',
                    '#[cfg_attr(target_os = "macos", ignore)]', 'if cfg!(target_os = "macos") {}', '#[cfg(target_vendor = "apple")]'):
            file.write_text(cfg + '\n#[test] fn added_mac_test() {}\n')
            assert check(root, selected), cfg
            fixed = copy.deepcopy(selected)
            fixed['modules']['new-mac'] = {'new': 'platform module'}
            assert not check(root, fixed), cfg
        file.unlink()
        # A Mac-only crate root cannot be waved through by one module rule.
        (crate / 'src/lib.rs').write_text('#![cfg(target_os = "macos")]\n#[test] fn mac_only() {}')
        assert check(root, fixed)
        fixed['packages']['new-mac'] = 'Mac-only crate'
        assert not check(root, fixed)
        assert 'package(=new-mac)' in expression(fixed)
        (crate / 'src/lib.rs').unlink()
        (crate / 'Cargo.toml').write_text('[package]\nname = "new-mac"\n[lib]\npath = "src/darwin.rs"\n')
        (crate / 'src/darwin.rs').write_text('#![cfg(target_os = "macos")]\n#[test] fn added() {}')
        narrow = copy.deepcopy(selected)
        narrow['modules']['new-mac'] = {'darwin': 'wrong: this is a crate root'}
        assert check(root, narrow)
        assert not check(root, fixed)
    # Mac-only integration tests must be selected by binary, not just a
    # similarly named unit module.
    with tempfile.TemporaryDirectory(prefix='ov1160-integration-') as tmp:
        root = Path(tmp)
        crate = root / 'crates/native'
        (crate / 'tests').mkdir(parents=True)
        (crate / 'Cargo.toml').write_text('[package]\nname = "native"\n')
        (crate / 'tests/mac.rs').write_text('#![cfg(target_os = "macos")]\n#[test] fn added() {}')
        assert check(root, selected)
        fixed = copy.deepcopy(selected)
        fixed['binaries']['native'] = {'mac': 'Mac integration'}
        assert not check(root, fixed)
    assert scope(Path('tests/new_case.rs')) == ('binaries', 'new_case')
    assert scope(Path('src/fs_watch/offload_tests.rs')) == ('modules', 'fs_watch')
    assert scope(Path('src/rpc_agent_answer_tests.rs'), {Path('src/rpc_agent_answer_tests.rs'): (Path('src/rpc_agent_answer.rs'), 'tests')}) == ('modules', 'rpc_agent_answer')
    assert scope(Path('src/extra_tests.rs')) == ('modules', 'extra_tests')
    assert scope(Path('src/shared.rs'), {Path('src/shared.rs'): None}) == ('packages', None)
    assert scope(Path('tests/common/helper.rs')) == ('packages', None)
    assert scope(Path('src/bin/mac.rs')) == ('packages', None)
    ci = (ROOT / '.github/workflows/ci.yml').read_text()
    nightly = (ROOT / '.github/workflows/nightly.yml').read_text()
    assert not workflow_errors(ci, nightly)
    assert workflow_errors(ci.replace('--no-fail-fast --archive-file', '-p farcooler-cli --archive-file'), nightly)
    assert workflow_errors(ci, nightly.replace('--workspace --no-fail-fast', '-p farcooler-cli'))
    assert workflow_errors(ci.replace('--workspace --no-run', '-p farcooler-cli --no-run'), nightly)
    print('macOS Rust selection self-test: planted uncovered cfg tests and Mac-only crate rejected')


def dependency_self_test():
    """Real workspace crates with no Mac cfg in their own source/manifest."""
    with tempfile.TemporaryDirectory(prefix='ov1160-dependencies-') as tmp:
        root = Path(tmp)
        (root / 'Cargo.toml').write_text('''[workspace]
members = ["app", "platform/*"]
[workspace.dependencies]
native-alias = { package = "mac-only", path = "platform/native" }
''')
        for directory, name in (('app', 'app'), ('platform/native', 'mac-only'),
                                ('platform/helper', 'helper')):
            crate = root / directory
            (crate / 'src').mkdir(parents=True)
            (crate / 'Cargo.toml').write_text(f'[package]\nname = "{name}"\n')
            (crate / 'src/lib.rs').write_text('mod one;\n#[test] fn native_test() {}\n')
            (crate / 'src/one.rs').write_text('#[test] fn module_test() {}\n')
            (crate / 'tests').mkdir()
            (crate / 'tests/native.rs').write_text('#[test] fn integration_test() {}\n')
        app = root / 'app/Cargo.toml'
        native = root / 'platform/native/Cargo.toml'
        helper = root / 'platform/helper/Cargo.toml'
        selected = {'packages': {'app': 'Mac caller'}, 'modules': {}, 'binaries': {}}
        # Direct/renamed paths, workspace inheritance, dependency subtables,
        # target triples, all dependency kinds, and nested target predicates.
        for dependency in ('''[target.'cfg(target_os = "macos")'.dependencies]
mac-only = { path = "../platform/native" }
''', '''[target.'cfg(all(unix, target_os = "macos"))'.dev-dependencies]
renamed = { package = "mac-only", path = "../platform/native" }
''', '''[target.aarch64-apple-darwin.build-dependencies.native-alias]
workspace = true
''', '''[target.'cfg(target_vendor = "apple")'.dependencies]
native-alias.workspace = true
native-alias.optional = true
'''):
            app.write_text('[package]\nname = "app"\n' + dependency)
            errors = check(root, selected)
            assert any('mac-only' in error for error in errors), errors
            for kind, name in (('modules', 'one'), ('binaries', 'native')):
                partial = copy.deepcopy(selected)
                partial[kind]['mac-only'] = {name: 'incomplete coverage'}
                errors = check(root, partial)
                assert any('mac-only' in error for error in errors), errors
            fixed = copy.deepcopy(selected)
            fixed['packages']['mac-only'] = 'whole Mac-only crate'
            assert not check(root, fixed)
        workspace = root / 'Cargo.toml'
        workspace.write_text(workspace.read_text().replace('"app", "platform/*"', '"app"'))
        assert any('mac-only' in error for error in check(root, selected))
        assert not check(root, fixed)
        # Propagate platform exclusivity through ordinary transitive edges,
        # even a dependency cycle; a partial helper rule is still insufficient.
        native.write_text(native.read_text() + '[dependencies]\nhelper = { path = "../helper" }\n')
        helper.write_text(helper.read_text() + '[dependencies]\nmac-only = { path = "../native" }\n')
        errors = check(root, fixed)
        assert any('helper' in error for error in errors), errors
        fixed['modules']['helper'] = {'one': 'incomplete'}
        assert check(root, fixed)
        fixed['packages']['helper'] = 'whole transitive Mac-only crate'
        assert not check(root, fixed)
        # An unconditional consumer makes both dependencies shared. The Mac
        # caller still has native coverage, but these crates need no Mac rule.
        app.write_text(app.read_text() + '[dependencies]\nhelper = { path = "../platform/helper" }\n')
        assert not check(root, selected)
    print('Mac-only dependency self-test: unselected and partially selected crates rejected before whole-package selection')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--self-test', action='store_true')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    if args.self_test:
        self_test()
    selection = json.loads(SELECTION.read_text())
    errors = check(ROOT, selection)
    errors += workflow_errors((ROOT / '.github/workflows/ci.yml').read_text(),
                              (ROOT / '.github/workflows/nightly.yml').read_text())
    if errors:
        print('\n'.join(errors), file=sys.stderr)
        return 1
    if not args.check and not args.self_test:
        print(expression(selection))
    return 0


if __name__ == '__main__':
    sys.exit(main())
