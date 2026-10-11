#!/usr/bin/env python3
"""Build a file-level delta; unchanged libraries/fonts are not downloaded again."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import stat
import zipfile


def inventory(bundle):
    files = []
    for root, dirs, names in os.walk(bundle, followlinks=False):
        names += [name for name in dirs if (Path(root) / name).is_symlink()]
        dirs[:] = [name for name in dirs if not (Path(root) / name).is_symlink()]
        for name in sorted(names):
            path = Path(root) / name
            link = os.readlink(path) if path.is_symlink() else None
            data = link.encode() if link is not None else path.read_bytes()
            files.append(dict(path=path.relative_to(bundle).as_posix(),
                              sha256=hashlib.sha256(data).hexdigest(), size=len(data),
                              mode=stat.S_IMODE(path.lstat().st_mode), link=link))
    return sorted(files, key=lambda file: file['path'])


def version(bundle):
    with (bundle / 'Contents/Info.plist').open('rb') as file:
        info = plistlib.load(file)
    assert info['CFBundleIdentifier'] == 'com.ddxmu.myairaw'
    return info['CFBundleShortVersionString']


def build(old, new, output):
    old_files = {file['path']: file for file in inventory(old)}
    files = inventory(new)
    manifest = dict(schema=1, **{'from': version(old), 'to': version(new)}, files=files)
    with zipfile.ZipFile(output, 'x', zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for file in files:
            file['payload'] = file['link'] is None and old_files.get(file['path']) != file
            if file['payload']:
                archive.write(new / file['path'], 'files/' + file['path'])
        archive.writestr('manifest.json', json.dumps(manifest, ensure_ascii=False))
    print(f"{output}: {output.stat().st_size} bytes; {sum(f['payload'] for f in files)}/{len(files)} changed files")


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('old', type=Path)
    parser.add_argument('new', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    build(args.old, args.new, args.output)
