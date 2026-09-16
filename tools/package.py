#!/usr/bin/env python3
"""Package a clean committed repository, including .git, without build caches.

Private input images and rendered observations are opt-in. Original archives,
compiler distributions, target/, out/, sockets and symlinks are never packaged.
"""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import stat
import subprocess
import zipfile
from _support import ROOT, digest

STAMP = (2026, 9, 15, 0, 0, 0)

def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--private', action='store_true', help='include supplied images and derived captures; DO NOT publish')
    a = p.parse_args()
    status = subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT, text=True)
    if status:
        raise RuntimeError('commit or remove working-tree changes before packaging:\n' + status)
    head = subprocess.check_output(['git','rev-parse','HEAD'], cwd=ROOT, text=True).strip()
    paths = subprocess.check_output(['git','ls-files','-z'], cwd=ROOT).decode().split('\0')
    files = {ROOT / name for name in paths if name}
    files.update(p for p in (ROOT/'.git').rglob('*') if p.is_file())
    if a.private:
        for name in ('pokewalker.bin', 'eeprom.bin'):
            path = ROOT/'local-inputs'/name
            if not path.is_file():
                raise ValueError(f'missing private image: {path}')
            files.add(path)
        files.update(p for p in (ROOT/'private-observations').rglob('*') if p.is_file())
    manifest = {'schema':1, 'git_commit':head, 'private':a.private,
                'warning':'Private images and derived artwork/audio are not licensed under MIT.' if a.private else None,
                'files':{}}
    ordered = sorted(files, key=lambda p:p.relative_to(ROOT).as_posix())
    a.out = a.out.resolve()
    a.out.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(a.out,'x',compression=zipfile.ZIP_DEFLATED,compresslevel=9) as archive:
        for path in ordered:
            if path.is_symlink() or not stat.S_ISREG(path.stat().st_mode):
                raise ValueError(f'not a regular nonsymlink file: {path}')
            name = path.relative_to(ROOT).as_posix()
            if name.startswith(('target/', 'out/')) or name.endswith('.lock') and name.startswith('.git/'):
                raise ValueError(f'forbidden generated path: {name}')
            data = path.read_bytes()
            manifest['files'][name] = {'bytes':len(data), 'sha256':hashlib.sha256(data).hexdigest()}
            info = zipfile.ZipInfo('hachistep-starter/'+name, STAMP)
            info.create_system = 3
            info.external_attr = (path.stat().st_mode & 0xffff) << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(info,data)
        info = zipfile.ZipInfo('hachistep-starter/DELIVERY-MANIFEST.json', STAMP)
        info.compress_type = zipfile.ZIP_DEFLATED
        archive.writestr(info,json.dumps(manifest,indent=2)+'\n')
    with zipfile.ZipFile(a.out) as archive:
        failure=archive.testzip()
        if failure:
            raise RuntimeError(f'ZIP CRC failure: {failure}')
    print(json.dumps({'archive':str(a.out),'bytes':a.out.stat().st_size,'sha256':digest(a.out),'commit':head,'files':len(ordered)},indent=2))
if __name__=='__main__':
    try:
        main()
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as e:
        raise SystemExit(str(e))
