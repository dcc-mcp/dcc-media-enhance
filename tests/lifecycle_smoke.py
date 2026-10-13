"""Native OS filesystem/lifecycle contract outside the test harness."""
import argparse,json,os,subprocess,tempfile,time
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--binary',required=True,type=Path);a=p.parse_args();binary=a.binary.resolve()
def call(*argv,code=0):
    p=subprocess.run([str(binary),*map(str,argv)],capture_output=True,text=True, encoding="utf-8",timeout=60)
    assert p.returncode==code,(p.returncode,p.stdout,p.stderr)
    return json.loads(p.stdout)
with tempfile.TemporaryDirectory(prefix='dcc-lifecycle-') as temp:
    root=Path(temp); source=root/'source.ppm'
    source.write_bytes(b'P6\n1024 1024\n255\n'+bytes([41,73,112])*1024*1024)
    output=root/'result.png'
    job=dict(schema_version=1,operation='denoise',media='image',content='photo',input=source.name,output=output.name,backend='auto',colorspace='srgb',alpha='opaque',limits=dict(threads=1,memory_mb=256,timeout_secs=30,max_pixels=1048576,max_input_mb=8,max_output_mb=8,min_free_disk_mb=0))
    request=root/'job.json';request.write_text(json.dumps(job))
    # Deterministic native lease, independent of media speed or scheduling.
    lock=Path(str(output)+'.lock-owner')
    lock.write_text('{"tool":"dcc-media-enhance","job_id":"lease-fixture"}')
    if os.name == 'nt':
        import ctypes
        from ctypes import wintypes
        api=ctypes.WinDLL('kernel32',use_last_error=True)
        api.CreateFileW.argtypes=[wintypes.LPCWSTR,wintypes.DWORD,wintypes.DWORD,ctypes.c_void_p,wintypes.DWORD,wintypes.DWORD,wintypes.HANDLE]
        api.CreateFileW.restype=wintypes.HANDLE
        api.CloseHandle.argtypes=[wintypes.HANDLE]
        handle=api.CreateFileW(str(lock),0xC0000000,1,None,3,0,None)
        assert handle != ctypes.c_void_p(-1).value
        try: assert call('run',request,code=3)['error']['code']=='OUTPUT_LOCKED'
        finally: api.CloseHandle(handle)
    else:
        import fcntl
        with lock.open('r+b') as lease:
            fcntl.flock(lease,fcntl.LOCK_EX|fcntl.LOCK_NB)
            assert call('run',request,code=3)['error']['code']=='OUTPUT_LOCKED'
    child=subprocess.Popen([str(binary),'run',str(request)],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True, encoding="utf-8")
    try:
        state=root/'result.png.job.json';deadline=time.monotonic()+30
        while not state.exists() and child.poll() is None and time.monotonic()<deadline:time.sleep(.005)
        assert state.exists(),'No owned state'
        assert call('status',state)['status']=='running'
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
