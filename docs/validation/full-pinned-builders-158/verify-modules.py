import os,fcntl,subprocess,json
from pathlib import Path
root=Path('/var/tmp/codex-myrs/transient/yonik-full-pinned-158-20261113');out=Path(__file__).parent
fd=os.open(root,os.O_RDONLY|os.O_DIRECTORY);fcntl.flock(fd,fcntl.LOCK_SH)
results={}
try:
 for engine in ('sing-box','xray'):
  cache=root/engine/'build-native-adapter-cache'
  env={k:v for k,v in os.environ.items() if not k.startswith('GO')}
  env.update(GOMODCACHE=str(cache/'go-mod'),GOCACHE=str(cache/'go-build'),GOPATH=str(cache/'gopath'),GOTOOLCHAIN='local',GOENV='off',GOWORK='off',GOMAXPROCS='1',CGO_ENABLED='0')
  result=subprocess.run(['go','mod','verify'],cwd=root/engine/('build-source-'+engine),env=env,text=True,capture_output=True,timeout=180)
  (out/(engine+'-go-mod-verify.log')).write_text(result.stdout+result.stderr)
  results[engine]=result.returncode
 (out/'module-verification.json').write_text(json.dumps(results,indent=2)+'\n')
finally: os.close(fd)
assert all(v==0 for v in results.values()),results
