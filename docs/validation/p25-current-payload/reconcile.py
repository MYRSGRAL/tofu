"""Read-only сверка конкретного retained package; не лицензионный PASS."""
from __future__ import annotations

import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile

BASE = '3247e6c32a5ed83ec5d812dfc1f70695d3f2b3b4'
SOURCE = '14229843f9a11bf98ec4f5992df00f3247102ea2'
PACKAGE = Path('/home/myrs/projects/yonik/local/release-controller/linux-controller-rehearsal-14229843-msrv85-mtree4/package.tar')
PREFIX = 'usr/share/licenses/yonik/'


def git_bytes(ref: str, path: str) -> bytes:
    return subprocess.check_output(['git', 'show', f'{ref}:{path}'])


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def main() -> None:
    with PACKAGE.open('rb') as stream:
        package_hash = hashlib.file_digest(stream, 'sha256').hexdigest()
    assert package_hash == '26a013747e4c1dbb95e7290cd37b981f5dc7ef891a8865897f09fc45bacd0237'
    with tarfile.open(PACKAGE) as archive:
        members = archive.getmembers()
        assert len({m.name for m in members}) == len(members)
        assert all(m.isfile() for m in members)
        def read(name: str) -> bytes:
            stream = archive.extractfile(name)
            assert stream is not None
            return stream.read()
        entries = [{'path': m.name, 'size': m.size, 'sha256': digest(read(m.name))} for m in members]
        index = json.loads(read(PREFIX + 'index.json'))
        refs = {h for entry in index for h in entry['hashes']}
        assert all(digest(read(PREFIX + h)) == h for h in refs)
        comparisons = []
        for shipped, source in [('inventory.json', 'linux-inventory.json'), ('go-sources.json', 'composition/go-sources.json'), ('SOURCE-OBLIGATIONS.md', 'SOURCE-OBLIGATIONS.md'), ('SCOPE.md', 'composition/P25-SCOPE.md')]:
            path = 'packaging/licenses/' + source
            comparisons.append({'path': PREFIX + shipped, 'source_path': path, 'matches_source142': read(PREFIX + shipped) == git_bytes(SOURCE, path), 'matches_accepted3247': read(PREFIX + shipped) == git_bytes(BASE, path)})
        qt = json.loads(git_bytes(BASE, 'packaging/licenses/composition/qt-sources.json'))
        qt_refs = {r['sha256'] for p in qt for r in p['texts']}
        qt_missing = sorted(qt_refs - refs)
        go = json.loads(read(PREFIX + 'go-sources.json'))
        offers = []
        for record in go['records']:
            name = 'usr/share/yonik/sources/go/' + record['zip_sha256'] + '.zip'
            offers.append({'module': record['module'], 'version': record['version'], 'path': name, 'present': name in archive.getnames(), 'hash_verified': digest(read(name)) == record['zip_sha256'] if name in archive.getnames() else None})
        # Исторический inventory может быть шире фактического module closure.
        source_name = 'usr/share/yonik/sources/release-source.tar'
        with tarfile.open(fileobj=io.BytesIO(read(source_name))) as source_tar:
            instructions = [m.name for m in source_tar.getmembers() if m.name.endswith(('packaging/arch/README.md', 'docs/user/linux-ru.md', 'docs/user/linux-en.md'))]
        primary = []
        for folder, records, field in [('texts', json.loads(read(PREFIX + 'inventory.json'))['packages'], None), ('composition/texts', go['records'], 'texts')]:
            for record in records:
                for ref in (record[field] if field else record['license_files'] + record['notice_files']):
                    h = ref['sha256']
                    assert digest(read(PREFIX + h)) == h
                    assert digest(git_bytes(SOURCE, 'packaging/licenses/' + folder + '/' + h)) == h
                    primary.append(h)
        core_licenses = []
        for core, filename in [('xray', 'xray-26.10.31.LICENSE'), ('sing-box', 'sing-box-c005f353.LICENSE')]:
            with tarfile.open(fileobj=io.BytesIO(read('usr/share/yonik/sources/' + core + '/patched.tar'))) as sources:
                stream = sources.extractfile('src/LICENSE')
                assert stream is not None
                h = digest(stream.read())
                assert h == digest(git_bytes(SOURCE, 'packaging/licenses/upstream/' + filename))
                core_licenses.append({'core': core, 'member': 'src/LICENSE', 'sha256': h})
        result = {'core_primary_licenses_in_patched_archives': core_licenses, 'primary_ref_checks': len(primary), 'base': BASE, 'source': SOURCE, 'evidence_ref': '8844d221d6b55aa0aade08f214dec3d42269e7d1', 'package': str(PACKAGE), 'package_sha256': package_hash, 'entries': entries, 'notices_entries': len(index), 'verified_notice_hashes': len(refs), 'inventory_comparisons': comparisons, 'go_inventory_source_zip_presence': offers, 'qt_primary_text_hashes': sorted(qt_refs), 'qt_primary_hashes_outside_notices_index': qt_missing, 'instructions_in_release_source': instructions, 'final_linux_acceptance': False}
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
