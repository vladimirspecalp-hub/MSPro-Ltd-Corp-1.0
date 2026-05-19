"""Smoke test для mspro-mcp — без MCP клиента, прямые вызовы tool-функций."""

import os
import sys

sys.path.insert(0, os.path.dirname(__file__))
os.environ.setdefault(
    "MSPRO_TOKEN", "Imz95evEBTIzjJNE28YJ5JZGlbkJWiEi-p5XF3Exn4o"
)

import server  # noqa: E402

print("=" * 70)
print("V1: ping")
print(server.ping.fn() if hasattr(server.ping, "fn") else server.ping())

print()
print("=" * 70)
print("V2: list_posts (first 5)")
fn = server.list_posts.fn if hasattr(server.list_posts, "fn") else server.list_posts
posts = fn()
print(f"  total posts: {len(posts)}")
for p in posts[:5]:
    name = p.get("title", "?")
    slug = p.get("slug", "?")
    dept = p.get("dept_number", "?")
    psz = p.get("prompt_size_bytes") or 0
    print(f"  [{dept}] {slug:18} {name[:30]:30} prompt={psz} bytes")

print()
print("=" * 70)
print("V3: query_sql — last 5 dispatcher_logs")
fn = server.query_sql.fn if hasattr(server.query_sql, "fn") else server.query_sql
rows = fn(
    "SELECT id, from_entity, to_entity, status, hop_kind, attempts_count "
    "FROM dispatcher_logs ORDER BY created_at DESC LIMIT 5"
)
for r in rows:
    print(
        f"  {r['id'][:24]:24} {r['from_entity']:12} -> {r.get('to_entity') or 'NULL':18} "
        f"{r['status']:11} hop={r.get('hop_kind') or 'NULL'} att={r.get('attempts_count', '?')}"
    )

print()
print("=" * 70)
print("V4: get_state (truncated)")
fn = server.get_state.fn if hasattr(server.get_state, "fn") else server.get_state
state = fn()
print(f"  app:    {state.get('app')}")
print(f"  uptime: {state.get('uptime_sec')} sec")
print(f"  db:     {state.get('db', {}).get('path')}")
print(f"        size={state.get('db', {}).get('size_bytes')} bytes")

print()
print("=" * 70)
print("ALL TESTS PASSED")
