#!/usr/bin/env python3
"""Build-side only: pack full verified components; the customer runs no Python."""
import argparse,hashlib,json,os,stat,tarfile
from pathlib import Path
ROOT=Path(__file__).resolve().parent.parent
p=argparse.ArgumentParser();p.add_argument('target');args=p.parse_args()
release=json.loads((ROOT/'runtime/pi-release.json').read_text())
target=args.target;asset=release['assets'][target];destination=ROOT/'.cache/bundles'/target;destination.mkdir(parents=True,exist_ok=True)
manifest={'schema_version':1,'target':target,'omp_version':release['version'],'components':{}}
def digest(path):
 h=hashlib.sha256()
 with path.open('rb') as f:
  for data in iter(lambda:f.read(1024*1024),b''):h.update(data)
 return h.hexdigest()
def component(name,base,entrypoint):
 base=base.resolve();entries=[]
 for path in sorted(base.rglob('*')):
  info=path.lstat();relative=path.relative_to(base).as_posix()
  if path.is_symlink():
   link=os.readlink(path)
   if Path(link).is_absolute():raise ValueError('Absolute symlink in component')
   path.resolve().relative_to(base)
   entry={'path':relative,'kind':'symlink','target':link,'size':0,'sha256':None,'executable':False}
  elif path.is_dir():entry={'path':relative,'kind':'directory','size':0,'sha256':None,'target':None,'executable':False}
  elif path.is_file():entry={'path':relative,'kind':'file','size':info.st_size,'sha256':digest(path),'target':None,'executable':bool(info.st_mode&0o111)}
  else:raise ValueError('Special file in component')
  entries.append(entry)
 archive=destination/(name+'.tar.gz')
 with tarfile.open(archive,'w:gz',dereference=False) as tar:
  for entry in entries:
   path=base/entry['path'];info=tar.gettarinfo(str(path),arcname=entry['path']);info.uid=info.gid=0;info.uname=info.gname='';info.mtime=0
   if entry['kind']=='file':
    with path.open('rb') as f:tar.addfile(info,f)
   else:tar.addfile(info)
 manifest['components'][name]={'archive':archive.name,'archive_sha256':digest(archive),'entrypoint':entrypoint,'entries':entries}
 print(name,len(entries),archive.stat().st_size,flush=True)
base=ROOT/'.cache/omp'/release['version']/target;binary='omp.exe' if 'windows' in target else 'omp'
assert digest(base/binary)==asset['sha256']
component('omp',base,binary)
if 'apple-darwin' in target:
 arch='arm64' if target.startswith('aarch64') else 'x64';gui=ROOT/'desktop/out'/('AgentCheck-darwin-'+arch)
 component('gui',gui,'AgentCheck.app/Contents/MacOS/AgentCheck')
elif 'windows' in target:
 arch='arm64' if target.startswith('aarch64') else 'x64';gui=ROOT/'desktop/out'/('AgentCheck-win32-'+arch)
 component('gui',gui,'AgentCheck.exe')
(destination/'manifest.json').write_text(json.dumps(manifest,ensure_ascii=True,separators=(',',':'))+'\n')
print(destination,flush=True)
