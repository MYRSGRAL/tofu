import sys, json, time, traceback, shutil, hashlib, subprocess, fcntl
from pathlib import Path
REPO=Path('/home/myrs/.t3/worktrees/yonik/t3-aef0f1a8-4b0e7a90d95fae43')
OUT=Path('/home/myrs/projects/yonik/local/evidence/full-pinned-158-20261113')
ROOT=Path('/var/tmp/codex-myrs/transient/yonik-full-pinned-158-20261113')
sys.path.insert(0,str(REPO/'tools'))
import build_native_adapter as b
original=b.Runner.run
records=[]
def run(self, argv, cwd):
    if argv[:2]==['go','test']:
        saved=b.COMMAND_TIMEOUT
        b.COMMAND_TIMEOUT=900
        try:
            measured(self,argv+['-run','^$'],cwd,'compile-only')
        finally:
            b.COMMAND_TIMEOUT=saved
    return measured(self,argv,cwd,'package-runtime' if argv[:2]==['go','test'] else 'builder')
def measured(self,argv,cwd,phase):
    start=time.monotonic(); record={'argv':argv,'cwd':str(cwd),'phase':phase}
    try:
        result=original(self,argv,cwd); record['status']='PASS'; return result
    except BaseException as error:
        record['status']='FAIL'; record['error']=str(error); raise
    finally:
        record['elapsed_seconds']=time.monotonic()-start; records.append(record)
        (OUT/'commands.json').write_text(json.dumps(records,indent=2)+'\n')
b.Runner.run=run
results={}
import os
fd=os.open(ROOT,os.O_RDONLY|os.O_DIRECTORY)
fcntl.flock(fd,fcntl.LOCK_SH)
try:
    for engine in ('sing-box','xray'):
        root=ROOT/engine
        root.mkdir()
        try:
            manifest=b.build(root,OUT/engine,b.DEFAULT_PATCH,engine,native_codec=True)
            results[engine]={'status':'PASS','manifest':str(manifest)}
        except BaseException as error:
            results[engine]={'status':'FAIL','error':str(error)}
            (OUT/(engine+'-exception.log')).write_text(traceback.format_exc())
        finally:
            logs=root/'build-native-adapter-logs'
            if logs.exists(): shutil.copytree(logs,OUT/(engine+'-logs'))
            archives=[]
            for file in sorted(root.rglob('*')):
                if file.is_file() and file.suffix in ('.zip','.mod','.info'):
                    archives.append({'path':str(file.relative_to(root)),'sha256':b.sha256(file)})
            (OUT/(engine+'-dependency-archives.json')).write_text(json.dumps(archives,indent=2)+'\n')
            source=root/('build-source-'+engine)
            if source.exists():
                with (OUT/(engine+'-source.diff')).open('w') as stream:
                    subprocess.run(['git','diff','--binary'],cwd=source,stdout=stream,check=True)
            (OUT/'results.json').write_text(json.dumps(results,indent=2)+'\n')
finally:
    os.close(fd)
print(json.dumps(results))
