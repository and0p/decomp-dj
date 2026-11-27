#!/usr/bin/env python3
"""Builds and checks the cores in connectors/cores.toml: for each, its connector's build.sh,
the example's ROM, and a replay of the example from power-on with every hash compared.

  scripts/cores.py list                 the cores and their examples
  scripts/cores.py run [CORE...]        build what is missing, then check (default: every core
                                        with an example)
  scripts/cores.py build CORE           run the connector's build.sh
  scripts/cores.py rom CORE             build the example's ROM
  scripts/cores.py check CORE           replay the example and check the connector's flags
  scripts/cores.py deps CORE            install the packages the core and its ROM need
                                        (apt on Linux, Homebrew on macOS, pip)
  scripts/cores.py matrix [--event E] [--base SHA]
                                        the job matrix for CI, as `matrix=<json>`

`dj` is $DJ, else target/release/dj. Needs Python 3.11 or later.
"""
import argparse
import json
import os
import platform
import shlex
import subprocess
import sys
import tomllib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MANIFEST = os.path.join(ROOT, "connectors", "cores.toml")
# A system's name in the manifest, the runner CI uses for it, and the name CI shows.
SYSTEMS = {"linux": ("ubuntu-22.04", "linux-x86_64"), "macos": ("macos-15", "macos-arm64")}
# A change to one of these is a change to every core's check.
SHARED = ("connectors/cores.toml", "scripts/cores.py", ".github/workflows/ci.yml", "include/")


def cores():
    with open(MANIFEST, "rb") as f:
        out = tomllib.load(f).get("core", [])
    for c in out:
        for key in ("name", "library"):
            if key not in c:
                sys.exit(f"{MANIFEST}: a [[core]] has no `{key}`")
        if "example" in c and not ("rom" in c and "build_rom" in c):
            sys.exit(f"{MANIFEST}: {c['name']} has an example but no `rom` or `build_rom`")
        for s in c.get("systems", []) + c.get("push", []):
            if s not in SYSTEMS:
                sys.exit(f"{MANIFEST}: {c['name']} names the system `{s}` (known: {', '.join(SYSTEMS)})")
    return out


def core(name):
    for c in cores():
        if c["name"] == name:
            return c
    sys.exit(f"no core `{name}` in {MANIFEST} (scripts/cores.py list)")


def here():
    return {"Linux": "linux", "Darwin": "macos"}.get(platform.system(), "")


def dj():
    exe = os.environ.get("DJ") or os.path.join(ROOT, "target", "release", "dj")
    if not os.path.exists(exe) and not os.path.exists(exe + ".exe"):
        sys.exit(f"{exe} isn't built: cargo build --release -p dj-cli (or set DJ)")
    return exe


def library(c):
    """The core's path without its suffix, which is how dj takes it."""
    return os.path.join(ROOT, "build", "cores", c["library"])


def built(c):
    return any(os.path.exists(library(c) + s) for s in (".so", ".dylib", ".dll"))


def example(c):
    return os.path.join(ROOT, "examples", c["example"])


def rom(c):
    return os.path.join(example(c), c["rom"])


def run(cmd, cwd=ROOT):
    print("+ " + (cmd if isinstance(cmd, str) else shlex.join(cmd)), flush=True)
    code = subprocess.run(cmd, cwd=cwd, shell=isinstance(cmd, str)).returncode
    if code != 0:
        sys.exit(code)


def build(c):
    run(["bash", os.path.join("connectors", c["name"], "build.sh")])
    if not built(c):
        sys.exit(f"connectors/{c['name']}/build.sh didn't make build/cores/{c['library']}")


def build_rom(c):
    run(c["build_rom"], cwd=example(c))
    if not os.path.exists(rom(c)):
        sys.exit(f"`{c['build_rom']}` didn't make {rom(c)}")


def check(c):
    if "example" not in c:
        print(f"{c['name']}: no example to replay; built only")
        return
    exe = dj()
    # Every recorded hash, from power-on: fails if a segment plays differently here.
    run([exe, "-C", example(c), "check", "--from-power-on", "--strict", "--rom", rom(c), "--core", library(c)])
    # The flags the connector set while that played, against the decoder.
    run([exe, "-C", example(c), "connector-check"])
    for cmd in c.get("after", []):
        run(cmd.format(dj=shlex.quote(exe), rom=shlex.quote(rom(c)), core=shlex.quote(library(c))))


def deps(c):
    system = here()
    if system == "linux" and c.get("apt"):
        run(["sudo", "apt-get", "update", "-qq"])
        run(["sudo", "apt-get", "install", "-y", "-qq", *c["apt"]])
    if system == "macos" and c.get("brew"):
        run(["brew", "install", "--quiet", *c["brew"]])
    if c.get("pip"):
        run([sys.executable, "-m", "pip", "install", "--quiet", *c["pip"]])


def changed_files(base):
    """Files changed since `base`. None when that can't be told (so: assume everything)."""
    if not base or set(base) == {"0"}:
        return None
    res = subprocess.run(["git", "diff", "--name-only", base, "HEAD"], cwd=ROOT, capture_output=True, text=True)
    return res.stdout.split("\n") if res.returncode == 0 else None


def matrix(event, base):
    """One job per core and system. Linux (a core's `push` systems) on every push; every
    system of a core when it or its example changed; everything, built anew with no cache, on
    the weekly run and a run started by hand."""
    everything = event in ("schedule", "workflow_dispatch")
    files = None if everything else changed_files(base)
    jobs = []
    for c in cores():
        systems = c.get("systems", list(SYSTEMS))
        mine = [f"connectors/{c['name']}/"] + ([f"examples/{c['example']}/"] if "example" in c else [])
        on_push = [s for s in c.get("push", ["linux"]) if s in systems]
        # (When the change isn't known, a first push or a shallow clone: the push systems.)
        touched = files is not None and any(f.startswith(p) for f in files for p in (*mine, *SHARED))
        wanted = systems if everything or touched else on_push
        for s in wanted:
            runner, shown = SYSTEMS[s]
            jobs.append({
                "core": c["name"],
                "library": c["library"],
                "example": c.get("example", ""),
                "os": runner,
                "system": shown,
                "fresh": everything,
                "experimental": bool(c.get("experimental", False)),
            })
    return {"include": jobs}


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list")
    r = sub.add_parser("run")
    r.add_argument("cores", nargs="*")
    for name in ("build", "rom", "check", "deps"):
        sub.add_parser(name).add_argument("core")
    m = sub.add_parser("matrix")
    m.add_argument("--event", default="push")
    m.add_argument("--base", default="")
    a = p.parse_args()

    if a.cmd == "list":
        for c in cores():
            state = "built" if built(c) else "not built"
            print(f"{c['name']:<18} {c.get('example', '(no example)'):<14} {state}")
    elif a.cmd == "run":
        chosen = [core(n) for n in a.cores] or [c for c in cores() if "example" in c]
        for c in chosen:
            print(f"== {c['name']}", flush=True)
            if not built(c):
                build(c)
            if "example" in c and not os.path.exists(rom(c)):
                build_rom(c)
            check(c)
        print(f"ok: {', '.join(c['name'] for c in chosen)}")
    elif a.cmd == "build":
        build(core(a.core))
    elif a.cmd == "rom":
        build_rom(core(a.core))
    elif a.cmd == "check":
        check(core(a.core))
    elif a.cmd == "deps":
        deps(core(a.core))
    elif a.cmd == "matrix":
        print("matrix=" + json.dumps(matrix(a.event, a.base), separators=(",", ":")))


if __name__ == "__main__":
    main()
