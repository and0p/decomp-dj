#!/usr/bin/env python3
"""End-to-end check of `dj mcp` on a copy of examples/thwaite.

Needs the example hydrated with the current core (`dj -C examples/thwaite clean`) and
dj built (cargo build --release -p dj-cli). Usage: scripts/mcp_smoke.py
"""
import json, os, shutil, subprocess, sys, tempfile, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DJ = os.path.join(ROOT, "player", "target", "release", "dj")


class Client:
    def __init__(self, project, *extra):
        self.p = subprocess.Popen([DJ, "-C", project, "mcp", *extra], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.n = 0
        r = self.req("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "smoke", "version": "1"}})
        assert r["result"]["serverInfo"]["name"] == "decomp-dj", r

    def req(self, method, params=None):
        self.n += 1
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.n, "method": method, "params": params or {}}) + "\n")
        self.p.stdin.flush()
        return json.loads(self.p.stdout.readline())

    def call(self, name, ok=True, **args):
        r = self.req("tools/call", {"name": name, "arguments": args})["result"]
        text = r["content"][0]["text"]
        if ok and r.get("isError"):
            raise AssertionError(f"{name}: {text}")
        if not ok:
            assert r.get("isError"), f"{name} should have failed"
            return text
        kinds = [c["type"] for c in r["content"]]
        try:
            return json.loads(text), kinds
        except json.JSONDecodeError:
            return text, kinds

    def close(self):
        self.p.stdin.close()
        self.p.wait(timeout=10)


def main():
    tmp = tempfile.mkdtemp(prefix="dj-mcp-")
    proj = os.path.join(tmp, "thwaite")
    shutil.copytree(os.path.join(ROOT, "examples", "thwaite"), proj)
    try:
        c = Client(proj, "--allow", "save")
        tools = {t["name"] for t in c.req("tools/list")["result"]["tools"]}
        assert {"xref", "trace", "open_session", "fuzz", "save_draft"} <= tools, tools

        x, _ = c.call("xref", addr="practiceDoClick", limit=1)
        assert x["hits"]["items"], x
        seg = x["hits"]["items"][0]["segment"]
        t, _ = c.call("trace", addr="practiceDoClick", before=2, after=4)
        assert any(l.startswith(">>> $98A0") for l in t["trace"]["window"]), t
        k, _ = c.call("calls", addr="practiceDoClick", limit=1)
        assert k["segments"][0]["calls"][0]["returned"], k
        c.call("gaps", sort="nearness", limit=3)
        f, _ = c.call("frontier", limit=3)
        assert f["items"], f

        s, _ = c.call("open_session", **{"from": "adde56a6"})
        sid = s["session"]
        s, _ = c.call("step", session=sid, frames=20, press=["DOWN"])
        assert s["frame"] == 20 and s["pending"] == ["P1 JOYPAD DOWN 0"], s
        c.call("hold", session=sid, buttons=["A"])
        s, _ = c.call("step", session=sid, until="ran(practiceDoClick)", max_frames=100)
        assert s["condition_met"], s
        s, _ = c.call("step", session=sid, until="RAM:0x00 == 0x99", max_frames=30)
        assert s["condition_met"] is False and s["frames_run"] == 30, s
        _, kinds = c.call("screenshot", session=sid)
        assert kinds == ["text", "image"], kinds
        c.call("rewind", session=sid, frame=10)
        s, _ = c.call("play_text", session=sid, text="0 P1 JOYPAD START 1\n4 P1 JOYPAD START 0", frames=40)
        assert s["frames"] == 50, s
        b, _ = c.call("branch", session=sid, title="kept")
        c.call("close_session", session=sid)

        d, _ = c.call("draft_from_text", parent=seg[:8], title="smoke", text="0 P1 JOYPAD A 1\n4 P1 JOYPAD A 0", frames=30)
        c.call("append", draft=d["draft"], text="0 P1 JOYPAD B 1\n3 P1 JOYPAD B 0", frames=30)
        c.call("truncate", draft=d["draft"], frame=45)
        assert "past the end" in c.call("append", ok=False, draft=d["draft"], text="40 P1 JOYPAD B 1", frames=10)
        sv, _ = c.call("save_draft", draft=d["draft"])
        assert os.path.exists(sv["path"]), sv
        c.call("discard_draft", draft=b["draft"])

        j, _ = c.call("seek_goal", **{"from": "adde56a6", "condition": "ran(practiceDoClick+0x2b)", "max_frames": 60000, "seconds": 60, "seed": 3})
        for _ in range(60):
            st, _ = c.call("job_status", job=j["job"])
            if st["status"] != "running":
                break
            time.sleep(1)
        assert st["status"] == "goal reached", st
        c.close()

        out = subprocess.run([DJ, "-C", proj, "check", sv["segment"][:8]], capture_output=True, text=True)
        assert out.returncode == 0, out.stdout + out.stderr
        print("ok")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
