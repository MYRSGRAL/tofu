import json, subprocess, hashlib
from pathlib import Path
out=Path(__file__).parent
repo=Path('/home/myrs/.t3/worktrees/yonik/t3-aef0f1a8-4b0e7a90d95fae43')
records={}
for engine,version in [('sing-box','v1.14.4'),('xray','v26.10.31')]:
    binary=out/engine/engine
    metadata=json.loads((repo/'adapters'/engine/version/'native-broker-codec.json').read_text())
    record={'native_protocol':metadata['native_protocol'],'runtime_hooks_connected':metadata['runtime_hooks_connected'],'adapter_features':metadata['adapter_features'],'frozen_spec_commit':metadata['frozen_spec_commit'],'production_issuer_acceptance':False,'linux_package_admission_seal':False,'installed_on_host':False}
    if binary.exists():
        record['binary_sha256']=hashlib.sha256(binary.read_bytes()).hexdigest()
        record['elf_header']=subprocess.check_output(['readelf','-h',str(binary)],text=True)
        record['file_capabilities']=subprocess.check_output(['getcap',str(binary)],text=True)
        record['go_build_info']=subprocess.check_output(['go','version','-m',str(binary)],text=True)
        (out/engine/'elf-header.log').write_text(record['elf_header'])
        (out/engine/'go-build-info.log').write_text(record['go_build_info'])
    records[engine]=record
(out/'capability-abi-manifest.json').write_text(json.dumps(records,indent=2)+'\n')
