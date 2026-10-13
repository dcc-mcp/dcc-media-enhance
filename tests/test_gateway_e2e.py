"""Real isolated Linux CLI -> gateway REST -> Core skill -> media acceptance.

Requires installed Core/CLI 0.20.44 and FFmpeg. Never installs software, starts a
DCC application, changes global configuration, or mocks the gateway/backend.
"""
import argparse
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request

HOST = r'''
import json, os, pathlib, signal, time
from dcc_mcp_core import McpHttpConfig, create_skill_server
root=pathlib.Path(os.environ['MEDIA_E2E_ROOT'])
c=McpHttpConfig(port=0, server_name='media-enhance-acceptance', enable_cors=False)
c.host='127.0.0.1'; c.dcc_type='python'; c.admin_enabled=False
# Zero disables registration as well as gateway election. Join the owned gateway.
c.gateway_port=int(os.environ['MEDIA_E2E_GATEWAY_PORT']); c.gateway_remote_port=0
c.registry_dir=str(root/'registry')
c.instance_metadata={'dcc_mcp_instance_type':'standalone'}
s=create_skill_server('python',c,extra_paths=[os.environ['DCC_MCP_SKILL_PATHS']],dcc_name='python',accumulated=False)
h=s.start(); (root/'host.json').write_text(json.dumps({'mcp_url':h.mcp_url()}))
running=True
def stop(*_):
    global running
    running=False
signal.signal(signal.SIGTERM,stop); signal.signal(signal.SIGINT,stop)
try:
    while running: time.sleep(.05)
finally: h.shutdown()
'''


def cleanup_owned(children, logs, binary, state, submitted, env, ports):
    """Best-effort independent teardown; any failure makes acceptance fail closed."""
    errors=[]
    native_terminal=None
    try:
        if submitted and not state.exists():
            until=time.monotonic()+5
            while not state.exists() and time.monotonic()<until:time.sleep(.05)
        if state.exists():
            try:
                subprocess.run([str(binary),'cancel',str(state)],env=env,capture_output=True,timeout=5,check=True)
            except (OSError,subprocess.SubprocessError) as error:
                errors.append('native cancel: '+type(error).__name__)
            until=time.monotonic()+5
            while time.monotonic()<until:
                try:native_terminal=json.loads(state.read_text()).get('status')
                except (OSError,ValueError):native_terminal=None
                if native_terminal in {'completed','failed','cancelled'}:break
                time.sleep(.05)
        if submitted and native_terminal not in {'completed','failed','cancelled'}:
            errors.append('native terminal state not established')
    except Exception as error:
        errors.append('native cleanup: '+type(error).__name__)
    finally:
        # A cancellation or one child failure must not skip another owner.
        for child in reversed(children):
            try:
                if child.poll() is None:child.terminate()
            except OSError as error:errors.append('terminate: '+type(error).__name__)
        for child in reversed(children):
            try:child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                try:child.kill();child.wait(timeout=5)
                except (OSError,subprocess.SubprocessError) as error:errors.append('kill/wait: '+type(error).__name__)
            except OSError as error:errors.append('wait: '+type(error).__name__)
        for log in logs:
            try:log.close()
            except OSError as error:errors.append('log close: '+type(error).__name__)
    closed=[]
    for port in ports:
        if port is None:continue
        try:
            with socket.create_connection(('127.0.0.1',port),timeout=.3):closed.append(False)
        except OSError:closed.append(True)
    return {'gateway_host_processes_stopped':all(c.poll() is not None for c in children),
            'exit_codes':[c.returncode for c in children], 'listeners_closed':all(closed),
            'native_cancel_job_terminal_state':native_terminal, 'errors':errors}


def test_cleanup_failures():
    """No processes or listeners: inject native-cancel failures into real teardown."""
    import io
    from unittest.mock import patch
    class Child:
        def __init__(self):self.returncode=None;self.terminated=False;self.waited=False
        def poll(self):return self.returncode
        def terminate(self):self.terminated=True
        def wait(self,timeout):self.waited=True;self.returncode=0
    with tempfile.TemporaryDirectory() as temporary:
        state=Path(temporary)/'job.json';state.write_text('{"status":"cancelled"}')
        for failure in (subprocess.TimeoutExpired('cancel',5),OSError('injected')):
            children=[Child(),Child()];log=io.StringIO()
            with patch('subprocess.run',side_effect=failure),patch('socket.create_connection',side_effect=OSError):
                receipt=cleanup_owned(children,[log],Path('/unused'),state,True,{},[1,2])
            assert all(c.terminated and c.waited for c in children) and log.closed
            assert receipt['gateway_host_processes_stopped'] and receipt['listeners_closed']
            assert receipt['native_cancel_job_terminal_state']=='cancelled' and receipt['errors']


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--binary',required=True,type=Path)
    parser.add_argument('--cli',type=Path,default=shutil.which('dcc-mcp-cli'))
    parser.add_argument('--report',type=Path)
    args=parser.parse_args()
    test_cleanup_failures()
    assert sys.platform.startswith('linux'), 'This wire acceptance currently targets Linux only'
    assert args.cli, 'Install the explicitly pinned dcc-mcp-cli before this test'
    assert importlib.metadata.version('dcc-mcp-core')=='0.20.44'
    binary=args.binary.resolve(); cli=Path(args.cli).resolve()
    skill=Path(__file__).resolve().parents[1]/'skill'
    report={'core_version':'0.20.44','native_binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),
            'scope':'Linux isolated loopback gateway; real CLI/SDK/FFmpeg; no deployed user host', 'calls':[], 'success':False}
    with tempfile.TemporaryDirectory(prefix='media-gateway-e2e-') as temporary:
        root=Path(temporary)
        env=dict({k:v for k,v in os.environ.items() if not k.startswith(('DCC_MCP_','DCC_MEDIA_','MEDIA_E2E_'))},PYTHONDONTWRITEBYTECODE='1',DCC_MCP_DISABLE_UPDATE_CHECK='1',
                 DCC_MCP_CLI_NO_AUTO_GATEWAY='true',DCC_MCP_CHECKPOINT_IN_MEMORY='1',RUST_LOG='warn',
                 DCC_MCP_SKILL_PATHS=str(skill),DCC_MCP_REGISTRY_DIR=str(root/'registry'),
                 DCC_MEDIA_ENHANCE_BIN=str(binary),DCC_MEDIA_ENHANCE_SHA256=report['native_binary_sha256'],
                 MEDIA_E2E_ROOT=str(root))
        for key,name in [('HOME','home'),('XDG_DATA_HOME','data'),('XDG_CONFIG_HOME','config'),('XDG_CACHE_HOME','cache')]:
            (root/name).mkdir();env[key]=str(root/name)
        (root/'registry').mkdir()
        report['cli_version']=subprocess.check_output([str(cli),'--version'],env=env,text=True).strip()
        assert report['cli_version']=='dcc-mcp-cli 0.20.44'
        with socket.socket() as reserved:
            reserved.bind(('127.0.0.1',0));port=reserved.getsockname()[1]
        url=f'http://127.0.0.1:{port}';env['MEDIA_E2E_GATEWAY_PORT']=str(port)
        children=[];logs=[];host_port=None;submitted_cancel_job=False;cancel_state=root/'cancelled.png.job.json'

        def invoke(label,*arguments,expected_codes=(0,)):
            result=subprocess.run([str(cli),'--base-url',url,'--transport','rest',*arguments,'--output','json'],
                                  env=env,capture_output=True,text=True,timeout=60)
            assert result.returncode in expected_codes,(label,result.returncode,result.stdout[-2000:],result.stderr[-2000:])
            payload=json.loads(result.stdout)
            report['calls'].append({'label':label,'exit_code':result.returncode,
                'control_route':payload.get('control_route'),'gateway_stats_recorded':payload.get('gateway_stats_recorded')})
            if arguments[0]=='call':
                assert payload.get('control_route')=='gateway',payload
                assert payload.get('gateway_stats_recorded') is True,payload
            return payload

        def start(command,name):
            log=open(root/(name+'.log'),'w');logs.append(log)
            child=subprocess.Popen(command,env=env,stdout=log,stderr=subprocess.STDOUT);children.append(child)
            return child

        def wait_for(condition,message,seconds=20):
            until=time.monotonic()+seconds
            while time.monotonic()<until:
                assert all(p.poll() is None for p in children), (message,[p.returncode for p in children],
                    {p.name:p.read_text()[-2000:] for p in root.glob('*.log')})
                if condition():return
                time.sleep(.05)
            raise AssertionError(message)

        def health():
            try:
                with urllib.request.urlopen(url+'/health',timeout=.3) as response:
                    return json.load(response).get('service')=='dcc-mcp-gateway'
            except OSError:return False

        completed=False
        try:
            start([str(cli),'gateway','--host','127.0.0.1','--port',str(port),'--remote-port','0',
                   '--registry-dir',str(root/'registry'),'--no-admin','true','--gateway-idle-timeout-secs','120'],'gateway')
            wait_for(health,'Gateway listener did not become healthy')
            start([sys.executable,'-c',HOST],'host')
            wait_for(lambda:(root/'host.json').exists(),'Skill host did not start')
            host_url=json.loads((root/'host.json').read_text())['mcp_url']
            host_port=int(host_url.split(':')[2].split('/')[0]);assert host_port!=port
            inventory=invoke('inventory','list');assert inventory['total']==1,inventory
            found=invoke('search','search','media-enhance')
            hits={h['backend_tool']:h for h in found['hits']}
            assert set(hits)=={'media_enhance__inspect','media_enhance__execute','media_enhance__cancel'}
            selected=hits['media_enhance__inspect'];assert selected['next_step']['action']=='load_skill'
            loaded=invoke('load','load-skill','--json',json.dumps(selected['next_step']['arguments']))
            assert loaded['loaded'] and loaded['tool_count']==3
            slugs={name:hit['tool_slug'] for name,hit in hits.items()}
            for name,read_only,idempotent in [('inspect',True,True),('execute',False,False),('cancel',False,True)]:
                description=invoke('describe_'+name,'describe',slugs['media_enhance__'+name])
                tool=description['tool'];annotations=tool['annotations']
                assert annotations['readOnlyHint']==read_only and annotations['destructiveHint'] is False
                assert annotations['idempotentHint']==idempotent and annotations['openWorldHint'] is False
                assert set(tool['inputSchema']['properties']['path'])=={'type','description'}
            report['wire_annotations_verified']=True

            def media(label,tool,parameters,*options):
                return invoke(label,'call',slugs['media_enhance__'+tool],'--json',json.dumps(parameters),*options)
            caps=media('capabilities','inspect',{'command':'capabilities'})
            assert caps['output']['context']['media']['version']=='0.2.0'
            source=root/'source.ppm';source.write_bytes(b'P6\n8 8\n255\n'+bytes([32,64,96])*64)
            source_sha=hashlib.sha256(source.read_bytes()).hexdigest()
            job=dict(schema_version=1,operation='resize',media='image',content='photo',input='source.ppm',output='result.png',
                     backend='auto',colorspace='srgb',alpha='opaque',scale=2,
                     limits=dict(threads=1,memory_mb=256,timeout_secs=30,max_pixels=1024,max_input_mb=8,max_output_mb=8,min_free_disk_mb=0))
            request=root/'resize.json';request.write_text(json.dumps(job))
            planned=media('plan','inspect',{'command':'plan','path':str(request)})
            assert planned['output']['context']['media']['expected_width']==16
            result=media('resize','execute',{'command':'run','path':str(request)},'--wait','--wait-timeout-secs','45')
            assert result['wait']['completed'] and not result['wait']['job_resubmitted']
            native=result['structuredContent']['result']['context']['media'];assert native['status']=='completed'
            status=media('completed_status','inspect',{'command':'status','path':str(root/'result.png.job.json')})
            assert status['output']['context']['media']['status']=='completed'
            assert hashlib.sha256(source.read_bytes()).hexdigest()==source_sha
            assert (root/'result.png.manifest.json').is_file() and (root/'result.png').is_file()
            dimensions=json.loads(subprocess.check_output(['ffprobe','-v','error','-show_entries','stream=width,height',
                '-of','json',str(root/'result.png')],text=True))['streams'][0]
            assert dimensions=={'width':16,'height':16},dimensions
            report['resize']={'dimensions':dimensions,'source_retained':True,'core_wait_completed':True,
                              'output_sha256':hashlib.sha256((root/'result.png').read_bytes()).hexdigest()}

            # Real CPU denoise, long enough to request cancellation through another
            # CLI invocation after observing the native supervisor's owned state.
            width=1536;pixels=width*width*3
            source.write_bytes(f'P6\n{width} {width}\n255\n'.encode()+bytes(range(256))*(pixels//256))
            source_sha=hashlib.sha256(source.read_bytes()).hexdigest()
            job.update(operation='denoise',output='cancelled.png',scale=None)
            job['limits'].update(max_pixels=width*width,memory_mb=512)
            request=root/'cancel.json';request.write_text(json.dumps(job))
            submitted_cancel_job=True
            accepted=media('submit_cancel_job','execute',{'command':'run','path':str(request),'timeout_secs':15})
            core=accepted.get('structuredContent',accepted.get('output',{}))
            assert core.get('job_id'),accepted
            wait_for(cancel_state.exists,'Native cancellable state did not appear')
            running=media('running_status','inspect',{'command':'status','path':str(cancel_state)})
            assert running['output']['context']['media']['status']=='running'
            cancelled=media('cancel','cancel',{'command':'cancel','path':str(cancel_state)})
            assert cancelled['output']['context']['media']['cancel_requested'] is True
            wait_for(lambda:json.loads(cancel_state.read_text()).get('status')=='cancelled','Native job did not cancel')
            terminal=media('cancelled_status','inspect',{'command':'status','path':str(cancel_state)})
            assert terminal['output']['context']['media']['status']=='cancelled'
            poll=result['structuredContent']['core_poll'];poll['arguments']['job_id']=core['job_id'];core_result=None
            for _ in range(100):
                response=invoke('core_job_status','call',poll['tool'],'--dcc-type',selected['dcc_type'],
                    '--instance-id',selected['instance_id'],'--require-gateway','--json',json.dumps(poll['arguments']),expected_codes=(0,1))
                core_result=response.get('structuredContent',response.get('output',{}))
                if core_result.get('status') in {'completed','failed','cancelled'}:break
                time.sleep(.05)
            assert core_result and core_result.get('status') in {'completed','failed','cancelled'},core_result
            assert core_result['result']['success'] is False and core_result['result']['error']=='CANCELLED',core_result
            assert core_result['result']['context']['exit_code']==5
            assert not (root/'cancelled.png').exists() and not (root/'cancelled.png.manifest.json').exists()
            assert hashlib.sha256(source.read_bytes()).hexdigest()==source_sha
            report['cancellation']={'native_status':'cancelled','core_status':core_result['status'],
                                    'no_output':True,'source_retained':True,'explicit_media_cancel':True,
                                    'media_error':'CANCELLED','native_exit_code':5,'core_poll_cli_exit_code':report['calls'][-1]['exit_code']}
            completed=True
        finally:
            report['cleanup']=cleanup_owned(children,logs,binary,cancel_state,submitted_cancel_job,env,[port,host_port])
            clean=report['cleanup']
            report['success']=completed and clean['gateway_host_processes_stopped'] and clean['listeners_closed'] and not clean['errors']
            if args.report:
                args.report.parent.mkdir(parents=True,exist_ok=True)
                args.report.write_text(json.dumps(report,indent=2)+'\n')
            assert clean['gateway_host_processes_stopped'] and clean['listeners_closed'] and not clean['errors'],clean
    print(json.dumps(report))


if __name__=='__main__':
    main()
