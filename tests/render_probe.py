#!/usr/bin/env python3
"""Opt-in real EXR probe. Uses an existing retained EXR, never renders a scene."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

p=argparse.ArgumentParser()
p.add_argument('--binary',type=Path,required=True)
p.add_argument('--input-exr',type=Path,required=True)
p.add_argument('--layer',default='ViewLayer')
p.add_argument('--memory-mb',type=int,default=384)
p.add_argument('--work-parent',type=Path,default=Path(tempfile.gettempdir()))
a=p.parse_args()
source=a.input_exr.resolve()
# Bound fixture size before hashing; production images belong in an explicit job.
assert source.is_file() and not a.input_exr.is_symlink() and source.stat().st_size<=8*1024*1024
before=hashlib.sha256(source.read_bytes()).hexdigest()
work=Path(tempfile.mkdtemp(prefix='dcc-render-probe-',dir=a.work_parent))
job=dict(schema_version=1,operation='denoise',media='image',content='render',input=str(source),output='denoised.exr',backend='blender-oidn',colorspace='scene-linear',alpha='opaque',passes=dict(layer=a.layer,beauty='Combined',albedo='Denoising Albedo',normal='Denoising Normal'),limits=dict(threads=1,memory_mb=a.memory_mb,timeout_secs=90,max_pixels=16384,max_input_mb=8,max_output_mb=8,min_free_disk_mb=700))
request=work/'job.json';request.write_text(json.dumps(job,indent=2))
r=subprocess.run([str(a.binary.resolve()),'run',str(request)],capture_output=True,text=True,timeout=180)
result=json.loads(r.stdout)
(work/'result.json').write_text(json.dumps(result,indent=2))
assert r.returncode==0,result
assert result['verification']['pixel_decode'] and result['source_unchanged']
assert result['output_media']['color_transfer']=='linear'
assert result['output_media']['pixel_format']=='gbrapf32le'
assert hashlib.sha256(source.read_bytes()).hexdigest()==before
print(json.dumps({'success':True,'work':str(work),'resource_receipt':result['resource_receipt'],'output_sha256':result['output_sha256']}))
