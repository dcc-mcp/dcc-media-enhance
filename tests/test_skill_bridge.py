"""Validate private skill discovery and the real CLI through the released Core SDK."""
import argparse,hashlib,importlib.metadata,json,os,subprocess,sys,tempfile,threading,time
from pathlib import Path
from dcc_mcp_core import CancelToken, HostExecutionBridge, SkillCatalog, ToolRegistry, parse_skill_md, scan_skill_paths
p=argparse.ArgumentParser();p.add_argument('--binary',required=True,type=Path);a=p.parse_args();binary=a.binary.resolve()
skill=Path('skill/media-enhance').resolve()
metadata=parse_skill_md(str(skill));assert metadata and metadata.name=='media-enhance'
# Inspect declarations after the actual Core parser/registration path. Flow YAML
# may silently turn an unquoted comma into an extra property, and unknown flat
# annotation names otherwise silently disappear from the callable tool contract.
registry=ToolRegistry();catalog=SkillCatalog(registry)
assert set(catalog.load_skill_object(metadata))=={'media_enhance__inspect','media_enhance__execute','media_enhance__cancel'}
for name,read_only,idempotent in [('inspect',True,True),('execute',False,False),('cancel',False,True)]:
    action=registry.get_action('media_enhance__'+name)
    assert action['annotations']==dict(read_only_hint=read_only,destructive_hint=False,idempotent_hint=idempotent,open_world_hint=False)
    properties=action['input_schema']['properties']
    assert set(properties)=={'command','path','timeout_secs'}
    assert set(properties['command'])=={'type','enum'}
    assert set(properties['path'])=={'type','description'}
    assert set(properties['timeout_secs'])=={'type','minimum','maximum','default'}
assert registry.get_action('media_enhance__inspect')['input_schema']['properties']['path']['description'].endswith('capabilities, doctor and schema.')

assert any(Path(path).resolve()==skill for path in scan_skill_paths(extra_paths=[str(skill.parent)],dcc_name='python'))
env=dict(os.environ,DCC_MEDIA_ENHANCE_BIN=str(binary),DCC_MEDIA_ENHANCE_SHA256=hashlib.sha256(binary.read_bytes()).hexdigest())
def run(script,params,environment=env):
    r=subprocess.run([sys.executable,str(skill/'scripts'/script)],input=json.dumps(params),capture_output=True,text=True, encoding="utf-8",env=environment,timeout=60)
    assert r.returncode==0,(r.returncode,r.stdout,r.stderr)
    return json.loads(r.stdout)
r=run('inspect_media.py',{'command':'capabilities'})
assert r['success'] and r['context']['media']['name']=='dcc-media-enhance'
assert run('inspect_media.py',{'command':'run','path':'/tmp/unsafe'})['error']=='INVALID_REQUEST'
assert run('inspect_media.py',{'command':'capabilities'},dict(env,DCC_MEDIA_ENHANCE_SHA256='0'*64))['error']=='BINARY_IDENTITY'
assert run('inspect_media.py',{'command':'capabilities'},dict(env,DCC_MEDIA_ENHANCE_BIN='relative.exe'))['error']=='BACKEND_MISSING'
# Exercise the real Core in-process runner as well as fresh subprocess entries.
# Environment is test-process local; no host installation/configuration changes.
os.environ.update({k:v for k,v in env.items() if k.startswith('DCC_MEDIA_ENHANCE_')})
bridge=HostExecutionBridge()
r=bridge.execute_script(str(skill/'scripts/inspect_media.py'),{'command':'capabilities'})
assert r['success'] and r['context']['media']['name']=='dcc-media-enhance',r
for script,command in [('execute.py','run'),('cancel.py','cancel')]:
    assert run(script,{'command':command,'path':'relative'})['error']=='INVALID_PATH'
    assert bridge.execute_script(str(skill/'scripts'/script),{'command':command,'path':'relative'})['error']=='INVALID_PATH'

with tempfile.TemporaryDirectory() as d:
    root=Path(d);source=root/'in.ppm';source.write_bytes(b'P6\n2 2\n255\n'+bytes([32,64,96])*4)
    job=dict(schema_version=1,operation='resize',media='image',content='photo',input=source.name,output='out.png',backend='auto',colorspace='srgb',alpha='opaque',scale=2,limits=dict(threads=1,memory_mb=256,timeout_secs=30,max_pixels=1024,max_input_mb=8,max_output_mb=8,min_free_disk_mb=0))
    request=root/'job.json';request.write_text(json.dumps(job))
    result=run('execute.py',{'command':'run','path':str(request)})
    assert result['success'],result
    assert result['context']['media']['status']=='completed'
    assert run('inspect_media.py',{'command':'status','path':str(root/'out.png.job.json')})['context']['media']['status']=='completed'
    # The actual Core in-process token must stop a real owned native CLI job.
    source.write_bytes(b'P6\n1024 1024\n255\n'+bytes([32,64,96])*1024*1024)
    source_sha=hashlib.sha256(source.read_bytes()).hexdigest()
    cancelled=dict(job,operation='denoise',output='cancelled.png',scale=None,
                   limits=dict(job['limits'],max_pixels=1048576))
    request.write_text(json.dumps(cancelled))
    token=CancelToken();result_box={}
    def work():
        result_box['result']=bridge.execute_script(str(skill/'scripts/execute.py'),
            {'command':'run','path':str(request)},cancel_token=token)
    worker=threading.Thread(target=work);worker.start()
    state=root/'cancelled.png.job.json';deadline=time.monotonic()+30
    while not state.exists() and worker.is_alive() and time.monotonic()<deadline:time.sleep(.01)
    assert state.exists(),'In-process job never created owned state'
    token.cancel();worker.join(timeout=10);assert not worker.is_alive()
    assert result_box['result']['error']=='CANCELLED',result_box
    assert not (root/'cancelled.png').exists()
    assert hashlib.sha256(source.read_bytes()).hexdigest()==source_sha
    terminal=run('inspect_media.py',{'command':'status','path':str(state)})['context']['media']['status']
    # Windows TerminateProcess closes the Job but cannot run Rust state cleanup.
    assert terminal in ({'cancelled','interrupted'} if os.name=='nt' else {'cancelled'}),terminal

print(json.dumps({'success':True,'core_version':importlib.metadata.version('dcc-mcp-core'),'scope':'native skill catalog + actual Core HostExecutionBridge + fresh subprocess entries + CLI run/status; no deployed gateway or user host modified'}))
