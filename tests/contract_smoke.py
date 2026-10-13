"""Backend-independent JSON contract and fail-closed backend discovery."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
p=argparse.ArgumentParser(); p.add_argument('--binary',required=True,type=Path); args=p.parse_args()
binary=args.binary.resolve()
def run(*argv,code=0,env=None):
    result=subprocess.run([str(binary),*map(str,argv)],capture_output=True,text=True, encoding="utf-8",env=env,timeout=60)
    assert result.returncode==code,(result.returncode,result.stdout,result.stderr)
    assert not result.stderr.strip()
    return json.loads(result.stdout)
v=run('--version'); assert v['source_commit'] and v['target']
c=run('capabilities'); assert c['cancellation']=='owned_state_sentinel'
assert 'batch' in c['commands'] and c['dcc_mcp']['discovery']=='DCC_MCP_SKILL_PATHS'
assert run('unknown',code=2)['error']['code']=='USAGE'
with tempfile.TemporaryDirectory(prefix='dcc path 空格 ') as d:
    source=Path(d)/'source.ppm'; source.write_bytes(b'P6\n1 1\n255\n\x20\x30\x40')
    env=dict(os.environ,PATH='')
    assert run('probe',source,code=3,env=env)['error']['code']=='BACKEND_MISSING'
    doctor=run('doctor',env=env)
    assert not any(b['present'] for b in doctor['backends'])
    assert doctor['capacity_observations']['free_disk_bytes']>0
print(json.dumps({'success':True,'target':v['target'],'tests':['version_metadata','capabilities','typed_usage','missing_backends','unicode_space_paths']}))
