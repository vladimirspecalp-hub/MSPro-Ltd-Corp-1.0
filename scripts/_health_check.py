import asyncio
import json
import os
import uuid
from pathlib import Path

import websockets


def token():
    import os
    t = os.environ.get("MSPRO_TOKEN", "").strip()
    if t:
        return t
    d = json.loads(Path(r"c:\CODE\.mcp.json").read_text(encoding="utf-8"))
    return d["mcpServers"]["mspro"]["env"]["MSPRO_TOKEN"]


async def rpc(method, params=None):
    url = f"ws://127.0.0.1:8899/?token={token()}"
    async with websockets.connect(url, open_timeout=5) as ws:
        req = {"jsonrpc": "2.0", "id": str(uuid.uuid4()), "method": method}
        if params:
            req["params"] = params
        await ws.send(json.dumps(req))
        r = json.loads(await asyncio.wait_for(ws.recv(), timeout=30))
        if "error" in r:
            return None, r["error"]
        return r.get("result"), None


async def main():
    checks = []
    p, e = await rpc("ping")
    checks.append(("Gateway ping", p is not None and "1.0." in str(p), str(p)))

    st, _ = await rpc("state")
    ver = (st or {}).get("app", {}).get("version", "?")
    mem = (st or {}).get("memory", {})
    checks.append(("App version", ver.startswith("1.0.3"), ver))
    checks.append(("Memory config", bool(mem), str(mem)[:120]))

    hmt = Path(os.environ["APPDATA"]) / "ru.msproltd.corp/Vault/01-HMT-Knowledge"
    n = len(list(hmt.glob("*.md"))) if hmt.is_dir() else 0
    checks.append(("HMT Knowledge vault", n >= 8, f"{n} md files"))

    rows, _ = await rpc("sql/query", {"query": "SELECT slug, length(COALESCE(system_prompt_md,'')) sp FROM posts"})
    hco = next((r for r in (rows or []) if r["slug"] == "hco-head"), None)
    ok_hco = hco and hco["sp"] > 100
    checks.append(("hco-head prompt saved", ok_hco, f"sp_len={hco['sp'] if hco else 'missing'}"))

    vault = Path(os.environ["APPDATA"]) / "ru.msproltd.corp/Vault/posts/hco-head"
    vf = list(vault.rglob("*.md")) if vault.is_dir() else []
    checks.append(("hco-head Vault files", len(vf) >= 3, f"{len(vf)} md files"))

    mad = list((Path(os.environ["APPDATA"]) / "ru.msproltd.corp/Vault/02-Patterns").glob("*hco*"))
    checks.append(("MAD pattern", len(mad) >= 1, mad[0].name if mad else "none"))

    print("=== HEALTH CHECK ===")
    passed = 0
    for name, ok, detail in checks:
        status = "PASS" if ok else "FAIL"
        if ok:
            passed += 1
        print(f"[{status}] {name}: {detail}")
    print(f"\n{passed}/{len(checks)} passed")
    return passed == len(checks)


if __name__ == "__main__":
    raise SystemExit(0 if asyncio.run(main()) else 1)
