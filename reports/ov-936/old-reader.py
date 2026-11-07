#!/usr/bin/python3
"""Exercise the two actual immutable old Store::open implementations, never owner DBs."""
import hashlib, io, json, os, subprocess, tarfile, tempfile
from pathlib import Path
root=Path(__file__).resolve().parents[2]
assert os.environ.get('HEAVY_LOCK_HELD')=='1'
fixture=Path(os.environ['OV936_OLD_READER_FIXTURE'])
assert fixture.is_relative_to(Path('/private/tmp')) and fixture.name=='db.sqlite'
assert fixture.parent.name.startswith('ov936-old-reader-')
assert fixture.is_file() and not fixture.is_symlink()
before=hashlib.sha256(fixture.read_bytes()).hexdigest()
receipts=json.loads((root/'reports/ov-936/old-reader-receipts.json').read_text()) if (root/'reports/ov-936/old-reader-receipts.json').exists() else []
helper=r'''
#[test]
fn actual_old_store_refuses_migrated_private_fixture() {
    let path=std::env::var("OV936_OLD_READER_FIXTURE").unwrap();
    assert!(matches!(farcooler_store::Store::open(&path),Err(farcooler_core::DomainError::NewerData)),"actual pinned old Store::open must refuse");
    println!("actual Store::open refused NewerData; schema={}",farcooler_store::DatabaseSchema::here());
}
'''
for pin in ['fae3f30d0c3d1c475f9dc4463243f293fd88b007','77250986dcb4b1dbb6b63e06d0b11dc1b92d88a3']:
    directory=Path(tempfile.mkdtemp(prefix='ov936-reader-source-',dir='/private/tmp'))
    archive=subprocess.check_output(['git','archive',pin,'Cargo.toml','Cargo.lock','crates','.cargo','proto','scripts/version.sh'],cwd=root)
    # Archive contains only trusted immutable repository files, no synthetic DB pointers.
    with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
        for member in tar.getmembers():
            assert not member.name.startswith('/') and '..' not in Path(member.name).parts
            assert not member.issym() and not member.islnk()
        tar.extractall(directory)
    source=directory/'crates/store/src/store.rs'
    expected=subprocess.check_output(['git','show',f'{pin}:crates/store/src/store.rs'],cwd=root)
    assert source.read_bytes()==expected
    tests=directory/'crates/store/tests'
    tests.mkdir(exist_ok=True)
    (tests/'engine_cutover_old_reader.rs').write_text(helper)
    # Baseline root mtimes are private archive mtimes; bind all dependency roots.
    inputs={}
    for path in sorted(directory.rglob('*')):
        if path.is_file():inputs[str(path.relative_to(directory))]=hashlib.sha256(path.read_bytes()).hexdigest()
    for path in directory.glob('crates/*/src/lib.rs'):os.utime(path,None)
    command=['/Users/e-liang/.cargo/bin/cargo','test','--offline','--manifest-path',str(directory/'Cargo.toml'),'-p','farcooler-store','--test','engine_cutover_old_reader','--','--nocapture']
    result=subprocess.run(command,cwd=directory)
    after=hashlib.sha256(fixture.read_bytes()).hexdigest()
    record=dict(pin=pin,source=str(directory),inputs=inputs,store_source_sha256=hashlib.sha256(expected).hexdigest(),command=command,exit=result.returncode,before=before,after=after,unchanged=before==after)
    receipts.append(record)
    (root/'reports/ov-936/old-reader-receipts.json').write_text(json.dumps(receipts,indent=2)+'\n')
    assert result.returncode==0 and before==after
