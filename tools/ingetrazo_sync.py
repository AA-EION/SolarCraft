#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Bring IngeTrazo code in crates/ingetrazo up to an upstream commit.

    python3 tools/ingetrazo_sync.py <ingetrazo checkout> [--commit REV] [--accept core/x.py ...]

* Vendored native kernels (rust/ingetrazo_core/src/*.rs) are copied byte for byte from REV
  and their SHA-256 pins updated. Then run `cargo test -p solarcraft-ingetrazo`.
* Ported Python modules are compared with their pinned upstream hash. Changed ones are listed
  with `git diff <pinned>..REV -- <file>` written to target/ingetrazo-sync/<name>.diff for a
  manual port; after porting, re-run with `--accept core/<name>.py` to move that pin to REV.
* New upstream kernels and Python modules not yet mapped are listed as candidates.
The checkout is only read (git show / git diff); nothing is checked out or modified there.
"""
import argparse, hashlib, os, re, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CRATE = os.path.join(ROOT, "crates", "ingetrazo")
MANIFEST = os.path.join(CRATE, "upstream.toml")

def git(repo, *args, binary=False):
    out = subprocess.check_output(["git", "-C", repo, *args])
    return out if binary else out.decode()

def parse(text):
    head, entries, cur = {}, [], None
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("[["):
            cur = {"kind": line.strip("[]")}; entries.append(cur); continue
        m = re.match(r'(\w+) = "(.*)"', line)
        if m: (cur if cur is not None else head)[m.group(1)] = m.group(2)
    return head, entries

def dump(head, entries):
    out = ["# Upstream pin for code taken from IngeTrazo (GPL-3.0-or-later).", "# Maintained by tools/ingetrazo_sync.py; see PORTING.md.", ""]
    out += [f'{k} = "{v}"' for k, v in head.items()] + [""]
    for e in entries:
        out.append(f"[[{e['kind']}]]")
        out += [f'{k} = "{v}"' for k, v in e.items() if k != "kind"] + [""]
    return "\n".join(out)

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("checkout"); ap.add_argument("--commit", default="HEAD"); ap.add_argument("--accept", nargs="*", default=[])
    a = ap.parse_args()
    rev = git(a.checkout, "rev-parse", a.commit).strip()
    head, entries = parse(open(MANIFEST).read())
    report = os.path.join(ROOT, "target", "ingetrazo-sync"); os.makedirs(report, exist_ok=True)
    print(f"pinned {head['commit'][:12]} → {rev[:12]}")
    for e in entries:
        if e["kind"] == "vendored":
            data = git(a.checkout, "show", f"{rev}:{e['upstream']}", binary=True)
            h = hashlib.sha256(data).hexdigest()
            if h != e["sha256"]:
                open(os.path.join(CRATE, e["local"]), "wb").write(data); e["sha256"] = h
                print(f"  vendored  {e['upstream']}: updated")
            else:
                print(f"  vendored  {e['upstream']}: unchanged")
        else:
            data = git(a.checkout, "show", f"{rev}:{e['upstream']}", binary=True)
            h = hashlib.sha256(data).hexdigest()
            if h == e["upstream_sha256"]:
                print(f"  ported    {e['upstream']}: unchanged"); e["commit"] = rev; continue
            if e["upstream"] in a.accept:
                e["upstream_sha256"], e["commit"] = h, rev; print(f"  ported    {e['upstream']}: accepted at {rev[:12]}"); continue
            diff = git(a.checkout, "diff", f"{e['commit']}..{rev}", "--", e["upstream"])
            name = os.path.join(report, os.path.basename(e["upstream"]) + ".diff"); open(name, "w").write(diff)
            print(f"  ported    {e['upstream']}: CHANGED upstream → review {os.path.relpath(name, ROOT)}, port into {e['local']}, then --accept {e['upstream']}")
    head["commit"] = rev
    mapped = {e["upstream"] for e in entries}
    kernels = [p for p in git(a.checkout, "ls-tree", "--name-only", rev, "rust/ingetrazo_core/src/").split() if p.endswith(".rs") and not p.endswith("/lib.rs")]
    for k in kernels:
        if k not in mapped: print(f"  candidate kernel (not vendored): {k}")
    open(MANIFEST, "w").write(dump(head, entries))
    print("next: cargo test -p solarcraft-ingetrazo -p solarcraft-geometry")

if __name__ == "__main__":
    sys.exit(main())
