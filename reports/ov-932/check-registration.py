#!/usr/bin/python3
"""Exercise the actual validation/guard blocks with a private recording command."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
script = ROOT/'apps/macos/build-app.sh'
source = script.read_text()
validation = source[source.index('REGISTER_APP='):source.index('\nesac',source.index('REGISTER_APP='))+6]
guard = source[source.index('if [ "$REGISTER_APP" = "1" ]'):source.index('\nfi',source.index('if [ "$REGISTER_APP" = "1" ]'))+3]
assert source.index('REGISTER_APP=') < source.index('cd "$(dirname')
records = []
temp = Path(tempfile.mkdtemp(prefix='ov932-registration-',dir='/private/tmp'))
fake = temp/'record-registration'
fake.write_text('#!/bin/sh\nprintf "%s\\n" "$@" >> "$OV932_REGISTRATION_RECORD"\n')
fake.chmod(0o700)
for index, value in enumerate([None,'0','1','','false','no','2','-1']):
    log = temp/f'calls-{index}'
    env = dict(os.environ, LSREG=str(fake), APP=str(temp/'private-build'), OV932_REGISTRATION_RECORD=str(log))
    env.pop('FARCOOLER_REGISTER_APP',None)
    if value is not None: env['FARCOOLER_REGISTER_APP'] = value
    result = subprocess.run(['/bin/bash','-c','set -eu\n'+validation+'\n'+guard],env=env,capture_output=True,text=True)
    calls = log.read_text() if log.exists() else ''
    expected = 0 if value in [None,'0','1'] else 2
    assert result.returncode == expected
    assert calls == ('-f\n'+str(temp/'private-build')+'\n' if value in [None,'1'] else '')
    records.append(dict(test='actual-blocks',value=value,exit=result.returncode,stdout=result.stdout,stderr=result.stderr,recorded_arguments=calls))
    if expected == 2:
        result = subprocess.run(['/bin/bash',str(script)],env=env,capture_output=True,text=True)
        assert result.returncode == 2 and result.stdout == '' and result.stderr == 'FARCOOLER_REGISTER_APP must be 0 or 1\n'
        records.append(dict(test='actual-script-refuses-before-build',value=value,exit=result.returncode,stdout=result.stdout,stderr=result.stderr))
syntax = subprocess.run(['/bin/bash','-n',str(script)],capture_output=True,text=True)
assert syntax.returncode == 0
result = dict(script_sha256=hashlib.sha256(script.read_bytes()).hexdigest(),checks=len(records)+1,syntax_exit=syntax.returncode,records=records,
              scratch=str(temp), boundary='Only extracted actual registration blocks used a private recorder. Actual full script ran for invalid values only; no real registry operation, build or app creation.')
(ROOT/'reports/ov-932/registration-check.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(dict(checks=result['checks'],syntax_exit=syntax.returncode,script_sha256=result['script_sha256'])))
