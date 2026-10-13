#!/usr/bin/env python3
"""Real, CPU-only smoke tests. Requires installed ffmpeg/ffprobe, not a GPU.

Creates only original deterministic synthetic fixtures in a NEW work directory.
The binary path is explicit; no algorithm or backend is mocked.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser()
parser.add_argument('--binary', required=True, type=Path)
parser.add_argument('--work-parent', type=Path, default=Path(tempfile.gettempdir()))
args = parser.parse_args()
binary = args.binary.resolve()
work = Path(tempfile.mkdtemp(prefix='dcc-media-smoke-', dir=args.work_parent))
results = []

def check(label, fn):
    started = time.monotonic()
    try:
        fn()
        results.append({'test': label, 'success': True, 'seconds': round(time.monotonic()-started, 3)})
    except Exception as exc:
        results.append({'test': label, 'success': False, 'error': str(exc)})
        raise

def call(*argv, code=0):
    proc = subprocess.run([str(binary), *map(str, argv)], capture_output=True, text=True, timeout=120)
    assert proc.returncode == code, (proc.returncode, proc.stdout, proc.stderr)
    assert not proc.stderr.strip(), proc.stderr
    assert len(proc.stdout.splitlines()) == 1, proc.stdout
    return json.loads(proc.stdout)

def ffmpeg(*argv):
    subprocess.run(['ffmpeg', '-hide_banner', '-loglevel', 'error', '-nostdin', '-n', '-threads', '1', '-filter_threads', '1', *map(str, argv)], check=True, capture_output=True, timeout=30)

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

limits = dict(threads=1, memory_mb=128, timeout_secs=30, max_pixels=262144,
              max_input_mb=8, max_output_mb=8, min_free_disk_mb=700)
def job(**kw):
    data = dict(schema_version=1, operation='denoise', media='image', content='photo',
                input='source.ppm', output='denoised.png', backend='auto',
                colorspace='srgb', alpha='opaque', limits=limits)
    data.update(kw)
    return data

def save(name, data):
    path = work/name
    path.write_text(json.dumps(data))
    return path

# No downloaded images. Small gradient + deterministic sensor-like noise.
rgb = bytes(max(0,min(255,(x*3+y*2+c*19)%220+((x*17+y*31+c*7)%25)-12))
            for y in range(24) for x in range(32) for c in range(3))
source = work/'source.ppm'
source.write_bytes(b'P6\n32 24\n255\n'+rgb)
source_hash = sha(source)
image_job = save('image.json', job())

def basic():
    assert call('--version')['name'] == 'dcc-media-enhance'
    assert call('schema')['properties']['schema_version']['const'] == 1
    d = call('doctor')
    assert d['ffmpeg_filters']['nlmeans'] and d['ffmpeg_filters']['atadenoise']
    assert d['default_device'] == 'cpu'
    assert call('probe', source)['media']['width'] == 32
    p = call('plan', image_job)
    assert p['backend'] == 'ffmpeg-nlmeans' and not (work/'denoised.png').exists()
check('doctor_schema_probe_plan_no_media_output', basic)

def image_success():
    result = call('run', image_job)
    assert result['verification']['pixel_decode'] is True
    assert result['source_unchanged'] is True and sha(source) == source_hash
    assert result['output_media']['width'] == 32
    assert result['status'] == 'completed' and not result['cache_hit']
    assert (work/'denoised.png.manifest.json').exists()
    assert call('status', work/'denoised.png.job.json')['status'] == 'completed'
    assert call('run', image_job)['cache_hit'] is True
check('real_image_denoise_verified_commit_and_cache', image_success)

def reject_changes():
    altered = job(limits=dict(limits, threads=2))
    assert call('run', save('altered.json', altered), code=2)['error']['code'] == 'OUTPUT_EXISTS'
    assert call('plan', save('overwrite.json', job(output='source.ppm')), code=2)['error']['code'] == 'SOURCE_OVERWRITE'
    assert call('plan', save('alpha.json', job(alpha='straight')), code=2)['error']['code'] == 'UNSUPPORTED_ALPHA'
    assert call('plan', save('zero.json', job(limits=dict(limits, threads=0))), code=2)['error']['code'] == 'INVALID_LIMITS'
    assert call('plan', save('extra.json', dict(job(), arbitrary_command='no')), code=2)['error']['code'] == 'INVALID_JSON'
    assert sha(source) == source_hash
check('fail_closed_contract_and_no_clobber', reject_changes)

def paths():
    (work/'link.png').symlink_to(source)
    assert call('plan', save('symlink.json', job(output='link.png')), code=2)['error']['code'] == 'UNSAFE_OUTPUT'
    foreign = work/'foreign.png.job.json'
    foreign.write_text('{"owned":"someone else"}')
    assert call('run', save('foreign.json', job(output='foreign.png')), code=3)['error']['code'] == 'STATE_NOT_OWNED'
    assert foreign.read_text() == '{"owned":"someone else"}'
    assert call('cancel', foreign, code=2)['error']['code'] == 'INVALID_STATE'
check('symlink_and_foreign_sidecar_safety', paths)

def resize():
    r = call('run', save('resize.json', job(operation='resize', output='resized.png', scale=2)))
    assert (r['output_media']['width'], r['output_media']['height']) == (64,48)
    assert r['backend']['id'] == 'ffmpeg-lanczos'  # Explicitly not neural SR.
    e = call('plan', save('upscale.json', job(operation='upscale', output='upscaled.png', scale=2)), code=2)
    assert e['error']['code'] == 'MODEL_REQUIRED'
check('cpu_resize_separate_from_unconfigured_neural_upscale', resize)

def video():
    # Irregular presentation timing, deliberate skipped intervals, lossless audio.
    ffmpeg('-f','lavfi','-i','testsrc2=size=32x24:rate=12:duration=1',
           '-f','lavfi','-i','sine=frequency=440:sample_rate=8000:duration=1',
           '-vf',"select='not(eq(n,2)+eq(n,5))'", '-fps_mode','vfr',
           '-c:v','ffv1','-threads','1','-c:a','pcm_s16le', work/'source.mkv')
    raw_hash = sha(work/'source.mkv')
    p = save('video.json', job(media='video', input='source.mkv', output='denoised.mkv'))
    r = call('run', p)
    v = r['verification']['video']
    assert v['relative_frame_timestamps_equal'] and v['audio_packet_sha256_equal']
    assert r['output_media']['frames'] == 10
    assert sha(work/'source.mkv') == raw_hash
    assert call('run', p)['cache_hit'] is True
check('real_vfr_video_denoise_audio_payload_and_timing', video)

def batch():
    batch_file = save('batch.json', {'schema_version':1, 'jobs':[job(),job(alpha='straight')]})
    r = call('batch', batch_file, code=3)
    assert not r['success'] and r['results'][0]['result']['cache_hit']
    assert r['results'][1]['error']['code'] == 'UNSUPPORTED_ALPHA'
check('serial_batch_resume_and_partial_failure_exit_code', batch)

def crash_status():
    state = save('dead.job.json', dict(tool='dcc-media-enhance', schema_version=1,
                 job_id='fixture-no-process', status='running', supervisor_pid=2147483647,
                 supervisor_identity='0', supervisor_pid_namespace=os.readlink('/proc/self/ns/pid')))
    assert call('status', state)['status'] == 'interrupted'
    assert call('cancel', state)['cancel_requested']
    assert call('cancel', state)['cancel_requested']
check('interrupted_state_and_idempotent_cancel', crash_status)


def live_cancel():
    source=work/'cancel-source.ppm'
    source.write_bytes(b'P6\n512 512\n255\n'+bytes([73,107,149])*(512*512))
    request=save('live-cancel.json', job(input=source.name, output='cancelled.png'))
    proc=subprocess.Popen([str(binary),'run',str(request)],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    state=work/'cancelled.png.job.json'; deadline=time.monotonic()+60
    while not state.exists() and proc.poll() is None and time.monotonic()<deadline:
        time.sleep(0.002)
    assert state.exists(), 'Running state was never created'
    acknowledgement=call('cancel',state)
    stdout,stderr=proc.communicate(timeout=20)
    assert acknowledgement['cancel_requested'], acknowledgement
    assert proc.returncode==5 and json.loads(stdout)['error']['code']=='CANCELLED', (proc.returncode,stdout,stderr)
    assert not (work/'cancelled.png').exists() and not (work/'cancelled.png.manifest.json').exists()
    assert call('status',state)['status']=='cancelled'
    resumed=call('run',request)
    assert resumed['status']=='completed' and not resumed['cache_hit']
check('real_cli_cancel_and_retry_preserved_source',live_cancel)

report = {'success':all(r['success'] for r in results), 'binary_sha256':sha(binary),
          'work_directory':str(work), 'tests':results,
          'quality_claim':'synthetic correctness smoke tests, not a quality benchmark'}
(work/'report.json').write_text(json.dumps(report, indent=2))
print(json.dumps(report, indent=2))
