import asyncio
import json
import uuid
from pathlib import Path

import websockets

def load_token() -> str:
    import os

    tok = os.environ.get("MSPRO_TOKEN", "").strip()
    if tok:
        return tok
    data = json.loads(Path(r"c:\CODE\.mcp.json").read_text(encoding="utf-8"))
    return data["mcpServers"]["mspro"]["env"]["MSPRO_TOKEN"]


async def rpc(method: str, params=None):
    url = f"ws://127.0.0.1:8899/?token={load_token()}"
    async with websockets.connect(url, open_timeout=5) as ws:
        req = {"jsonrpc": "2.0", "id": str(uuid.uuid4()), "method": method}
        if params is not None:
            req["params"] = params
        await ws.send(json.dumps(req))
        resp = json.loads(await asyncio.wait_for(ws.recv(), timeout=30))
        if "error" in resp:
            raise RuntimeError(resp["error"])
        return resp.get("result")


async def main():
    ping = await rpc("ping")
    print("ping:", ping)
    state = await rpc("state")
    print("state_keys:", list(state.keys()) if isinstance(state, dict) else type(state))
    if isinstance(state, dict):
        print("app:", state.get("app"))
        print("gateway:", state.get("gateway"))
    n = await rpc("sql/query", {"query": "SELECT COUNT(*) AS n FROM chat_messages"})
    print("chat_count:", n)
    depts = await rpc(
        "sql/query",
        {"query": "SELECT dept_number, name FROM departments ORDER BY dept_number"},
    )
    print("departments:", depts)


if __name__ == "__main__":
    asyncio.run(main())
