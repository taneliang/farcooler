#!/usr/bin/env python3
"""ov-1160: maintained nextest selection for per-push macOS Rust.

--check runs on Linux too, before a missing Mac test can land. A new source
scope mentioning target_os=macos (including cfg!, cfg_attr, all/any and file
level cfg) must be selected in its entirety. No Rust parser guessing where
an attribute ends: select the whole top-level module, or whole package for
crate roots. Separate *_tests files belong to the corresponding module.
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

ROOT = Path(__file__).resolve().parent.parent
SELECTION = ROOT / 'scripts/macos-rust-tests.json'
MAC = re.compile(r'(?:target_os\s*=\s*"macos"|target_vendor\s*=\s*"apple")')


def scope(path):
    parts = path.parts
    if parts[0] == 'tests':
        return 'binaries', parts[1].removesuffix('.rs')
    if parts[0] == 'src' and parts[1] not in ('lib.rs', 'main.rs'):
        # All nested files are covered by the top-level module's prefix.
        return 'modules', parts[1].removesuffix('.rs').removesuffix('_tests')
    return 'packages', None


def covered(selection, package, kind, name):
    return package in selection['packages'] or name in selection.get(kind, {}).get(package, {})


def check(root, selection):
    errors = []
    # Scan every crate, not only today's workspace list: a new Mac-only crate
    # or a crate-level cfg must demand a package selection too.
    manifests = set((root / 'crates').rglob('Cargo.toml'))
    workspace = root / 'Cargo.toml'
    if workspace.exists():
        body = workspace.read_text()
        members = re.search(r'(?s)members\s*=\s*\[(.*?)\]', body)
        if members:
            for member in re.findall(r'"([^"]+)"', members[1]):
                manifests.update(root.glob(member + '/Cargo.toml'))
    for manifest in sorted(manifests):
        body = manifest.read_text()
        package = re.search(r'(?m)^name\s*=\s*"([^"]+)"', body).group(1)
        crate = manifest.parent
        for source in sorted(crate.rglob('*.rs')):
            if 'target' in source.relative_to(crate).parts:
                continue
            if MAC.search(source.read_text()):
                kind, name = scope(source.relative_to(crate))
                if not covered(selection, package, kind, name):
                    errors.append(f'{source.relative_to(root)}: select {package} {kind} {name or "(whole crate)"}')
        if MAC.search(body) and not (package in selection['packages'] or any(
                selection.get(kind, {}).get(package) for kind in ('modules', 'binaries'))):
            errors.append(f'{manifest.relative_to(root)}: Mac-specific crate has no selection')
    return errors


def expression(selection):
    terms = [f'package(={p})' for p in sorted(selection['packages'])]
    for package, modules in sorted(selection['modules'].items()):
        for module in sorted(modules):
            # binary kind avoids a coincidentally named integration test.
            terms.append(f'(package(={package}) and kind(lib) and test(/^{module}::/))')
    for package, binaries in sorted(selection['binaries'].items()):
        for binary in sorted(binaries):
            terms.append(f'(package(={package}) and binary(={binary}))')
    return ' or '.join(terms)



def workflow_errors(ci, nightly):
    errors = []
    linux = ci.split('      - name: Full Rust tests and shadow measurement', 1)[-1].split('      - name:', 1)[0]
    if "if: runner.os == 'Linux'" not in linux or 'cargo nextest run --profile ci --workspace --no-fail-fast' not in linux or ' -E ' in linux:
        errors.append('Linux must retain the full workspace suite')
    mac = ci.split('      - name: macOS native Rust tests', 1)[-1].split('      - uses:', 1)[0]
    if "if: runner.os == 'macOS'" not in mac or 'cargo nextest run --profile ci --workspace --no-run' not in mac or 'macos-rust-tests.py' not in mac:
        errors.append('Mac must build the full workspace and run the selection')
    night = nightly.split('  rust-macos:', 1)[-1].split('\n  #', 1)[0]
    if 'runs-on: macos-latest' not in night or 'cargo nextest run --profile ci --workspace --no-fail-fast' not in night or ' -E ' in night:
        errors.append('Nightly must retain the full Mac workspace suite')
    return errors


def self_test():
    selected = {'packages': {}, 'modules': {}, 'binaries': {}}
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
    assert scope(Path('src/rpc_agent_answer_tests.rs')) == ('modules', 'rpc_agent_answer')
    ci = (ROOT / '.github/workflows/ci.yml').read_text()
    nightly = (ROOT / '.github/workflows/nightly.yml').read_text()
    assert not workflow_errors(ci, nightly)
    assert workflow_errors(ci.replace('--workspace --no-fail-fast', '-p farcooler-cli'), nightly)
    assert workflow_errors(ci, nightly.replace('--workspace --no-fail-fast', '-p farcooler-cli'))
    assert workflow_errors(ci.replace('--workspace --no-run', '-p farcooler-cli --no-run'), nightly)
    print('macOS Rust selection self-test: planted uncovered cfg tests and Mac-only crate rejected')


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
