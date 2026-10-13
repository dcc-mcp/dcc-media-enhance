"""Native OS filesystem/lifecycle contract outside the test harness."""
import argparse,json,os,subprocess,tempfile,time
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--binary',required=True,type=Path);a=p.parse_args();binary=a.binary.resolve()
def call(*argv,code=0):
    p=subprocess.run([str(binary),*map(str,argv)],capture_output=True,text=True,timeout=60)
    assert p.returncode==code,(p.returncode,p.stdout,p.stderr)
    return json.loads(p.stdout)
with tempfile.TemporaryDirectory(prefix='dcc-lifecycle-') as temp:
    root=Path(temp); source=root/'source.ppm'
    source.write_bytes(b'P6\n512 512\n255\n'+bytes([41,73,112])*512*512)
    output=root/'result.png'
    job=dict(schema_version=1,operation='denoise',media='image',content='photo',input=source.name,output=output.name,backend='auto',colorspace='srgb',alpha='opaque',limits=dict(threads=1,memory_mb=256,timeout_secs=30,max_pixels=262144,max_input_mb=8,max_output_mb=8,min_free_disk_mb=0))
    request=root/'job.json';request.write_text(json.dumps(job))
    child=subprocess.Popen([str(binary),'run',str(request)],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    try:
        state=root/'result.png.job.json';deadline=time.monotonic()+30
        while not state.exists() and child.poll() is None and time.monotonic()<deadline:time.sleep(.005)
        assert state.exists(),'No owned state'
        assert call('status',state)['status']=='running'
        assert call('run',request,code=3)['error']['code']=='OUTPUT_LOCKED'
        assert call('cancel',state)['cancel_requested']
        out,err=child.communicate(timeout=10);assert child.returncode==5,(out,err)
        assert not output.exists()
        assert call('status',state)['status']=='cancelled'
        assert call('run',request)['status']=='completed'
    finally:
        if child.poll() is None:child.terminate();child.communicate(timeout=10)
    if os.name=='nt':
        for bad in ['NUL.png','result.png:alternate','trailing .png ']:
            request.write_text(json.dumps(dict(job,output=bad)))
            assert call('plan',request,code=2)['error']['code']=='INVALID_PATH'
print(json.dumps({'success':True,'tests':['live_ownership','concurrent_lock','cancel_sentinel','state_replace','retry_after_cancel','platform_paths']}))
