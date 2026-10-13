"""Validate private skill discovery and the real CLI through the released Core SDK."""
import argparse,hashlib,importlib.metadata,json,os,subprocess,sys,tempfile
from pathlib import Path
from dcc_mcp_core import parse_skill_md, scan_skill_paths
p=argparse.ArgumentParser();p.add_argument('--binary',required=True,type=Path);a=p.parse_args();binary=a.binary.resolve()
skill=Path('skill/media-enhance').resolve()
metadata=parse_skill_md(str(skill));assert metadata and metadata.name=='media-enhance'
assert any(Path(path).resolve()==skill for path in scan_skill_paths(extra_paths=[str(skill.parent)],dcc_name='python'))
env=dict(os.environ,DCC_MEDIA_ENHANCE_BIN=str(binary),DCC_MEDIA_ENHANCE_SHA256=hashlib.sha256(binary.read_bytes()).hexdigest())
def run(script,params,environment=env):
    r=subprocess.run([sys.executable,str(skill/'scripts'/script)],input=json.dumps(params),capture_output=True,text=True,env=environment,timeout=60)
    assert r.returncode==0,(r.returncode,r.stdout,r.stderr)
    return json.loads(r.stdout)
r=run('inspect.py',{'command':'capabilities'})
assert r['success'] and r['context']['media']['name']=='dcc-media-enhance'
assert run('inspect.py',{'command':'run','path':'/tmp/unsafe'})['error']=='INVALID_REQUEST'
assert run('inspect.py',{'command':'capabilities'},dict(env,DCC_MEDIA_ENHANCE_SHA256='0'*64))['error']=='BINARY_IDENTITY'
assert run('inspect.py',{'command':'capabilities'},dict(env,DCC_MEDIA_ENHANCE_BIN='relative.exe'))['error']=='BACKEND_MISSING'
with tempfile.TemporaryDirectory() as d:
    root=Path(d);source=root/'in.ppm';source.write_bytes(b'P6\n2 2\n255\n'+bytes([32,64,96])*4)
    job=dict(schema_version=1,operation='resize',media='image',content='photo',input=source.name,output='out.png',backend='auto',colorspace='srgb',alpha='opaque',scale=2,limits=dict(threads=1,memory_mb=256,timeout_secs=30,max_pixels=1024,max_input_mb=8,max_output_mb=8,min_free_disk_mb=0))
    request=root/'job.json';request.write_text(json.dumps(job))
    result=run('execute.py',{'command':'run','path':str(request)})
    assert result['success'],result
    assert result['context']['media']['status']=='completed'
    assert run('inspect.py',{'command':'status','path':str(root/'out.png.job.json')})['context']['media']['status']=='completed'
print(json.dumps({'success':True,'core_version':importlib.metadata.version('dcc-mcp-core'),'scope':'native skill catalog + SDK envelopes + actual CLI run/status; no deployed gateway or user host modified'}))
