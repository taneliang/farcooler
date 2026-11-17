"""Landing disk refusal and removal of the run's extracted nextest archive (ov-1327)."""
from pathlib import Path
import shutil
import subprocess

GIB = 1024 ** 3
DAEMON_RESERVE = 25 * GIB
# ov-1322's initial sampled peak: 38.335506 GiB (tree + scratch + logs).
# 45 GiB leaves 6.66 GiB margin; the doctest tail was unmeasured.
# Source: 0f39e9e38:reports/ov-1322/report.md; see reports/ov-1327/report.md.
GATE_BUDGET = 45 * GIB
NEEDED = GATE_BUDGET + DAEMON_RESERVE


def admit(repo, say=print):
    """Read capacity once; never delete anything or wait for a build lock."""
    free = shutil.disk_usage(repo).free
    message = (f"free {free / GIB:.2f} GB; needed {NEEDED / GIB:.2f} GB "
               f"({GATE_BUDGET / GIB:.0f} GB full gate + {DAEMON_RESERVE / GIB:.0f} GB daemon reserve)")
    say(f"lane-gate: disk admission: {message}", flush=True)
    if free < NEEDED:
        raise ValueError(f"insufficient disk: {message}; free space manually or retry later")


def extract_archive(repo, command, say=print):
    """Extract once into our target, then use metadata for the run and retries."""
    command = list(command)
    if '--archive-file' not in command:
        return command
    repo = Path(repo).resolve()
    index = command.index('--archive-file')
    archive = Path(command[index + 1])
    archive = archive if archive.is_absolute() else repo / archive
    if archive != repo / 'target/rust-tests.tar.zst' or archive.resolve() != archive:
        raise ValueError(f'disk cleanup refused archive: {archive}')
    destination = repo
    if (repo / 'target/nextest').resolve() != repo / 'target/nextest':
        raise ValueError(f'disk cleanup refused extraction: {repo / "target/nextest"}')
    subprocess.run(['cargo', 'nextest', 'list', '--archive-file', str(archive),
                    '--extract-to', str(destination), '--extract-overwrite',
                    '--workspace-remap', str(repo), '--list-type', 'binaries-only',
                    '--message-format', 'json'], cwd=repo, check=True, stdout=subprocess.DEVNULL)
    metadata = repo / 'target/nextest'
    for name in ('cargo-metadata.json', 'binaries-metadata.json'):
        if not (metadata / name).is_file():
            raise ValueError(f'nextest extraction missing {metadata / name}; keeping archive')
    archive.unlink()
    say(f'lane-gate: removed extracted archive {archive}', flush=True)
    # CI supplies extraction/remapping flags after the archive. They require
    # --archive-file, so replace them too; retain filters, profile and wrappers.
    tail, skip = [], False
    for arg in command[index + 2:]:
        if skip:
            skip = False
        elif arg in ('--extract-to', '--workspace-remap', '--archive-format'):
            skip = True
        elif arg not in ('--extract-overwrite', '--persist-extract-tempdir'):
            tail.append(arg)
    return command[:index] + ['--cargo-metadata', str(metadata / 'cargo-metadata.json'),
                             '--binaries-metadata', str(metadata / 'binaries-metadata.json'),
                             '--workspace-remap', str(repo), '--target-dir-remap', str(repo / 'target')] + tail
