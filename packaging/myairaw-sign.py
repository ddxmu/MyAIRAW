#!/usr/bin/env python3
"""Sign exact checksums.txt bytes with the local, never-published Ed25519 key."""
import argparse
import hashlib
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('key', type=Path)
parser.add_argument('directory', type=Path)
parser.add_argument('assets', nargs='+')
args = parser.parse_args()
manifest = args.directory / 'checksums.txt'
manifest.write_text(''.join(f"{hashlib.sha256((args.directory / name).read_bytes()).hexdigest()}  {name}\n" for name in args.assets))
subprocess.run(['/opt/homebrew/opt/openssl@3/bin/openssl', 'pkeyutl', '-sign', '-rawin',
                '-inkey', str(args.key), '-in', str(manifest), '-out', str(manifest) + '.sig'], check=True)
