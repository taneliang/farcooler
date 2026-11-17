import json, os, pathlib, signal, subprocess, sys, time
out = pathlib.Path(sys.argv[1]); out.mkdir(parents=True, exist_ok=True)
workers = []
os.environ['PATH'] = '/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/usr/local/bin:' + os.environ['PATH']
try:
    # Exactly sixteen CPU workers, owned and stopped by their recorded PIDs.
    for _ in range(16):
        workers.append(subprocess.Popen(['/usr/bin/yes'], stdout=subprocess.DEVNULL))
        (out / 'load-pids.json').write_text(json.dumps([p.pid for p in workers]))
    results = []
    for n in range(1, 21):
        started = time.monotonic()
        with (out / f'run-{n:02}.log').open('w') as log:
            ran = subprocess.run(['swift', 'test', '--skip-build', '--jobs', '4', '--filter', 'NativeOfferTests.theRealCLINamesTheRunningAgent'], cwd='apps/macos', stdout=log, stderr=subprocess.STDOUT, restore_signals=False, env={**os.environ, 'TMPDIR': '/tmp'})
        said = (out / f'run-{n:02}.log').read_text()
        verified = '✔ Test \"A claude typed into a shell is offered the view, through the real CLI and daemon\" passed' in said
        cpu = subprocess.check_output(['ps', '-p', ','.join(str(p.pid) for p in workers), '-o', '%cpu='], text=True)
        row = dict(run=n, load_workers=sum(p.poll() is None for p in workers), worker_cpu_percent=sum(float(v) for v in cpu.split()), exit=ran.returncode, verified=verified, seconds=round(time.monotonic()-started, 3), load=os.getloadavg())
        results.append(row)
        (out / 'results.json').write_text(json.dumps(results, indent=2))
        print(row, flush=True)
        if ran.returncode or not verified: break
finally:
    for p in workers:
        if p.poll() is None: os.kill(p.pid, signal.SIGTERM)
    for p in workers: p.wait()
    (out / 'load-stopped.json').write_text(json.dumps([dict(pid=p.pid, exit=p.returncode) for p in workers], indent=2))
