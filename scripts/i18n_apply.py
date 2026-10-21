"""Apply exact (old, new) text replacements for translating reused components.
Idempotent: a pair whose replacement is already present is skipped."""
import sys, json

def apply(path, pairs, add_import=None):
    s = open(path).read()
    for a, b in pairs:
        if a in s:
            s = s.replace(a, b)
        elif b in s:
            continue
        else:
            raise SystemExit(f"{path}: not found: {a[:80]!r}")
    if add_import and add_import not in s:
        if '<script' in s:
            i = s.index('>', s.index('<script')) + 1
            s = s[:i] + '\n' + add_import + s[i:]
        else:
            s = add_import + '\n' + s
    open(path, 'w').write(s)
