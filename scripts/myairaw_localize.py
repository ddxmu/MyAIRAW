#!/usr/bin/env python3
"""Apply reviewed Chinese UI literals without touching Rust identifiers or JSON keys."""
import argparse
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
# Comments and raw strings are consumed intact, so the replacement never edits
# documentation, regular expressions, embedded JSON, shader code or SQL.
TOKENS = re.compile(r'//[^\n]*|/\*.*?\*/|r(?P<hash>\#*)".*?"(?P=hash)|"(?:\\.|[^"\\])*"', re.S)

def files():
    return sorted(path for path in (ROOT / "src/app").rglob("*.rs")
                  if path.name not in {"chinese.rs", "photo_metadata.rs"})

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--apply", action="store_true")
    args = parser.parse_args()
    translations = json.loads((ROOT / "assets/zh-CN.json").read_text()) if args.apply else {}
    found = {}
    total = 0
    for path in files():
        source = path.read_text()
        def replace(match):
            nonlocal total
            token = match.group()
            if not token.startswith('"'):
                return token
            value = re.sub(r'\\\n\s*', '', token[1:-1])
            if value in translations:
                total += 1
                return '"' + translations[value] + '"'
            if re.search(r'[A-Z][a-z]|[A-Za-z] [A-Za-z]', value):
                found.setdefault(value, []).append(str(path.relative_to(ROOT)))
            return token
        updated = TOKENS.sub(replace, source)
        if args.apply and updated != source:
            path.write_text(updated)
    print(f"replaced={total}")
    if args.apply:
        return
    for value, paths in sorted(found.items()):
        print(json.dumps(value, ensure_ascii=False), "\t", paths[0])

if __name__ == "__main__":
    main()
