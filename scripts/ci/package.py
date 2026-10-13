"""Package native-tested executables, pinned notices, skill and provenance."""
import argparse,hashlib,json,os,platform,shutil,subprocess,tomllib,zipfile
from pathlib import Path
from pe_checks import checked_windows_imports
p=argparse.ArgumentParser();p.add_argument('--binary',required=True,type=Path);p.add_argument('--platform',required=True);p.add_argument('--target',required=True);a=p.parse_args()
binary=a.binary.resolve();v=json.loads(subprocess.check_output([str(binary),'--version'],text=True))
assert v['target']==a.target,(v,a.target)
assert v['source_commit']==os.environ['GITHUB_SHA'],v
version=tomllib.loads(Path('Cargo.toml').read_text())['package']['version'];assert v['version']==version
name=f'dcc-media-enhance-{version}-{a.platform}'
dist=Path('dist');stage=Path('target')/name;stage.mkdir(parents=True,exist_ok=False);dist.mkdir(exist_ok=True)
for path in [binary,Path('LICENSE'),Path('THIRD_PARTY_NOTICES.md'),Path('Cargo.lock'),Path('README.md')]:shutil.copy2(path,stage/path.name)
for folder in ['schemas','examples','skill']:
    shutil.copytree(folder,stage/folder,ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
shutil.copy2('docs/platforms.md',stage/'PLATFORMS.md');shutil.copy2('docs/dcc-mcp-integration.md',stage/'DCC-MCP-INTEGRATION.md')
sysroot=Path(subprocess.check_output(['rustc','--print','sysroot'],text=True).strip());notices=sysroot/'share/doc/rust'
license_dir=stage/'rust-licenses';license_dir.mkdir()
for entry in notices.glob('*'):
    if entry.is_file() and (entry.name.startswith('COPYRIGHT') or entry.name.startswith('LICENSE')):shutil.copy2(entry,license_dir/entry.name)
if (notices/'licenses').is_dir():shutil.copytree(notices/'licenses',license_dir/'licenses')
assert list(license_dir.glob('COPYRIGHT*')),'Missing Rust standard-library copyright notices'
assert list(license_dir.glob('LICENSE*')),'Missing Rust toolchain license texts'
imports=checked_windows_imports(binary.read_bytes()) if a.platform.startswith('windows-') else []
sha=hashlib.sha256(binary.read_bytes()).hexdigest()
metadata=dict(**v,platform=a.platform,binary=binary.name,binary_sha256=sha,
    rustc=subprocess.check_output(['rustc','--version','--verbose'],text=True),runtime=platform.platform(),
    native_tests=['unit_process_tree','real_ffmpeg_smoke','json_contract','lifecycle_lock_cancel','core_skill_bridge'],
    verification_scope='CI native runner only; no end-user hardware or render/OIDN validation on Windows/macOS',
    backends_bundled=False,models_bundled=False,windows_imported_dlls=imports,windows_crt='static' if imports else None)
(stage/'build-manifest.json').write_text(json.dumps(metadata,indent=2)+'\n')
files=sorted(p for p in stage.rglob('*') if p.is_file())
(stage/'SHA256SUMS').write_text(''.join(f'{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.relative_to(stage).as_posix()}\n' for p in files))
archive=dist/(name+'.zip')
with zipfile.ZipFile(archive,'w',zipfile.ZIP_DEFLATED,compresslevel=9) as z:
    for p in sorted(stage.rglob('*')):
        if p.is_file():z.write(p,p.relative_to(stage).as_posix())
(dist/(name+'.sha256')).write_text(f'{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n')
print(json.dumps({'archive':str(archive),'binary_sha256':sha,'source_commit':v['source_commit']}))
