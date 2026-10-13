#!/usr/bin/env python3
"""Bounded regression tests for reviewed cache/path/cancellation boundaries.

Pixels use installed FFmpeg. Only the cancellation test interposes an explicit
sleeping probe fixture, after the real enhancement, to exercise that boundary.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser()
parser.add_argument('--binary', required=True, type=Path)
args = parser.parse_args()
binary = args.binary.resolve()
work = Path(tempfile.mkdtemp(prefix='dcc-media-risk-'))
results = []

def call(*argv, code=0, env=None):
    p = subprocess.run([str(binary), *map(str, argv)], capture_output=True,
                       text=True, timeout=120, env=env)
    assert p.returncode == code, (p.returncode, p.stdout, p.stderr)
    assert not p.stderr.strip(), p.stderr
    return json.loads(p.stdout)

def save(path, value):
    path.write_text(json.dumps(value))
    return path

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

source = work/'source.ppm'
source.write_bytes(b'P6\n8 8\n255\n' + bytes([40, 90, 150])*64)
original_sha = digest(source)
job = dict(schema_version=1, operation='resize', media='image', content='photo',
           input='source.ppm', output='output.png', backend='auto',
           colorspace='srgb', alpha='opaque', scale=2,
           limits=dict(threads=1, memory_mb=128, timeout_secs=30,
                       max_pixels=1024, max_input_mb=8, max_output_mb=8,
                       min_free_disk_mb=700))
request = save(work/'job.json', job)
baseline = call('run', request)
assert baseline['status'] == 'completed'
assert call('run', request)['cache_hit']
manifest = work/'output.png.manifest.json'
valid = manifest.read_bytes()
output_sha = digest(work/'output.png')

# Parseable but semantically incomplete/false manifests must never prove success.
for field in ('verification', 'resource_receipt', 'source_unchanged', 'backend'):
    bad = json.loads(valid)
    del bad[field]
    save(manifest, bad)
    assert call('run', request, code=2)['error']['code'] == 'OUTPUT_EXISTS'
    assert digest(work/'output.png') == output_sha
    assert json.loads(manifest.read_text()) == bad
manifest.write_bytes(valid)
assert call('run', request)['cache_hit']
results.append('incomplete_manifest_rejected_without_output_mutation')

# Identical relative jobs in a different directory have different provenance.
relocated = work/'relocated'
relocated.mkdir()
for name in ('source.ppm', 'output.png', 'output.png.manifest.json', 'job.json'):
    shutil.copyfile(work/name, relocated/name)
assert call('run', relocated/'job.json', code=2)['error']['code'] == 'OUTPUT_EXISTS'
results.append('relocated_manifest_not_reused_with_stale_absolute_paths')

# lexists, not exists: even dangling foreign links are preserved.
for suffix in ('.job.json', '.manifest.json', '.job.cancel'):
    output = 'foreign-' + suffix.replace('.', '-') + '.png'
    sidecar = work/(output + suffix)
    sidecar.symlink_to('missing-foreign-target')
    req = save(work/'foreign.json', dict(job, output=output))
    call('run', req, code=2)
    assert sidecar.is_symlink() and os.readlink(sidecar) == 'missing-foreign-target'
    assert not (work/output).exists()
results.append('dangling_foreign_state_manifest_cancel_links_preserved')

# Extension is deliberately deceptive: the actual demuxer must be allowlisted.
playlist = work/'nested.ppm'
playlist.write_text('ffconcat version 1.0\nfile source.ppm\n')
req = save(work/'nested.json', dict(job, input=playlist.name, output='nested.png'))
assert call('plan', req, code=3)['error']['code'] == 'BACKEND_FAILED'
assert not (work/'nested.png').exists()
results.append('sniffed_playlist_rejected_before_referenced_input_cache')

# A sentinel during output verification must cancel the batch, not become a
# probe failure and continue. The test wrapper is original and local to this run.
real_probe = shutil.which('ffprobe')
assert real_probe
bin_dir = work/'bin'
bin_dir.mkdir()
wrapper = bin_dir/'ffprobe'
wrapper.write_text('#!/usr/bin/env python3\n'
    'import os,sys,time\nfrom pathlib import Path\n'
    'if any("/.dcc-media-" in a and a.endswith("/result.png") for a in sys.argv[1:]):\n'
    f'    Path({str(work/"probe-started")!r}).write_text("ready")\n'
    '    time.sleep(30)\n    sys.exit(7)\n'
    f'os.execv({real_probe!r}, [{real_probe!r}] + sys.argv[1:])\n')
wrapper.chmod(0o700)
env = dict(os.environ, PATH=str(bin_dir)+os.pathsep+os.environ['PATH'])
def cancel_case(fail_state_write):
    output = 'cancelled-state-write.png' if fail_state_write else 'cancelled.png'
    marker = work/'probe-started'
    if marker.exists():
        marker.unlink()
    state = work/(output + '.job.json')
    batch = save(work/'cancel-batch.json', dict(schema_version=1,
        jobs=[dict(job, output=output), dict(job, output='must-not-start.png')]))
    p = subprocess.Popen([str(binary), 'batch', str(batch)], stdout=subprocess.PIPE,
                         stderr=subprocess.PIPE, text=True, env=env)
    try:
        deadline = time.monotonic()+90
        while not marker.exists() and p.poll() is None and time.monotonic()<deadline:
            time.sleep(0.01)
        assert marker.exists(), 'Verification fixture never started'
        if fail_state_write:
            collision = state.with_suffix(f'.{p.pid}.tmp')
            collision.write_text('preserve existing temporary file')
        began = time.monotonic()
        assert call('cancel', state)['cancel_requested']
        stdout, stderr = p.communicate(timeout=5)
        result = json.loads(stdout)
        assert p.returncode == 5 and result['cancelled'] and result['not_started'] == 1, result
        assert result['results'][0]['error']['code'] == 'CANCELLED'
        assert time.monotonic()-began < 5 and not stderr.strip()
        status = call('status', state)['status']
        assert status == ('interrupted' if fail_state_write else 'cancelled'), status
        if fail_state_write:
            assert 'advisory state update failed' in result['results'][0]['error']['message']
            assert collision.read_text() == 'preserve existing temporary file'
        assert not (work/output).exists() and not (work/'must-not-start.png').exists()
        assert not (work/(output + '.manifest.json')).exists()
    finally:
        if p.poll() is None:
            p.terminate()
            p.communicate(timeout=5)

cancel_case(False)
cancel_case(True)
results.append('terminal_state_write_failure_preserves_primary_cancellation')
assert digest(source) == original_sha
results.append('verification_cancel_stops_batch_and_preserves_source')
print(json.dumps(dict(success=True, tests=results, work_directory=str(work)), indent=2))
