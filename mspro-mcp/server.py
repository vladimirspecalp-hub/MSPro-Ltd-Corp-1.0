"""MSPro-Ltd Corp MCP server — нативные tools для Claude.

Sub-D5 (v1.0.27 Phase 11D). Backend сейчас — WebSocket gateway MSPro (порт 8899).
Когда COM сервер будет готов в Sub-D1b — backend переключится на pywin32
`win32com.client.Dispatch("MSProLtdCorp.Application")` без изменения tool API.

Регистрация в `~/.claude/mcp.json` или `.mcp.json`:

    {
      "mcpServers": {
        "mspro": {
          "command": "python",
          "args": ["C:\\\\CODE\\\\MSPro-Ltd Corp 1.0\\\\mspro-mcp\\\\server.py"],
          "env": {"MSPRO_TOKEN": "Imz95evEBTIzjJNE28YJ5JZGlbkJWiEi-p5XF3Exn4o"}
        }
      }
    }

После перезапуска Claude Code появятся tools `mcp__mspro__ping`, `mcp__mspro__list_posts`,
`mcp__mspro__dispatch_task`, `mcp__mspro__query_sql`, `mcp__mspro__get_task_chain`,
`mcp__mspro__get_state`.
"""

from __future__ import annotations

import asyncio
import json
import os
import uuid
from typing import Any

import websockets
from fastmcp import FastMCP

# ---------------------------------------------------------------------------
# Конфиг
# ---------------------------------------------------------------------------

WS_HOST = os.environ.get("MSPRO_WS_HOST", "127.0.0.1")
WS_PORT = int(os.environ.get("MSPRO_WS_PORT", "8899"))


def _token() -> str:
    tok = os.environ.get("MSPRO_TOKEN", "").strip()
    if not tok:
        raise RuntimeError(
            "MSPRO_TOKEN env-var не задан. Скопируй токен из "
            "MSPro → Настройки → External Agent Gateway (Developer Mode)"
        )
    return tok


def _ws_url() -> str:
    return f"ws://{WS_HOST}:{WS_PORT}/?token={_token()}"


# ---------------------------------------------------------------------------
# JSON-RPC client
# ---------------------------------------------------------------------------


async def _call(method: str, params: dict[str, Any] | None = None) -> Any:
    """Один JSON-RPC вызов через WebSocket. Возвращает result или бросает RuntimeError."""
    async with websockets.connect(_ws_url(), open_timeout=5) as ws:
        req = {"jsonrpc": "2.0", "id": str(uuid.uuid4()), "method": method}
        if params is not None:
            req["params"] = params
        await ws.send(json.dumps(req))
        resp_raw = await asyncio.wait_for(ws.recv(), timeout=60)
        resp = json.loads(resp_raw)
        if "error" in resp:
            err = resp["error"]
            raise RuntimeError(f"RPC error {err.get('code')}: {err.get('message')}")
        return resp.get("result")


def _run(coro):
    """Синхронная обёртка для FastMCP tools (FastMCP ожидает sync или async)."""
    return asyncio.run(coro)


# ---------------------------------------------------------------------------
# MCP server
# ---------------------------------------------------------------------------

mcp = FastMCP("mspro")


@mcp.tool()
def ping() -> str:
    """Проверка связи с MSPro-Ltd Corp. Возвращает версию и uptime."""
    return _run(_call("ping"))


@mcp.tool()
def get_state() -> dict:
    """Полное состояние работающего MSPro: app, os, memory, db, uptime_sec, gateway."""
    return _run(_call("state"))


@mcp.tool()
def query_sql(query: str) -> list[dict]:
    """Read-only SELECT/WITH запрос к app.db MSPro. Авто-LIMIT 1000.

    Доступные таблицы:
      * posts (id, slug, title, department_id, status, system_prompt_md, ...)
      * departments (id, dept_number, name)
      * dispatcher_logs (id, from_entity, to_entity, status, hop_kind,
                         parent_task_id, attempts_count, outbox_path, ...)
      * dispatcher_decisions (id, source_task_id, decision_kind, model_used,
                              routing_complexity, elapsed_ms, reasoning)
      * task_artifacts (id, task_id, rel_path, mime_type, size_bytes,
                        created_by, approved_at, rejected_at)
      * chat_messages (id, role, content, model, created_at)
      * audit_logs (Step 5)

    Запрещено: INSERT/UPDATE/DELETE/DROP/ALTER/PRAGMA — gateway вернёт RPC error.
    """
    return _run(_call("sql/query", {"query": query}))


@mcp.tool()
def dispatch_task(from_entity: str, to_entity: str, payload: dict | None = None) -> dict:
    """Создать задачу в Диспетчере MSPro (новая запись dispatcher_logs).

    Args:
      from_entity: кто шлёт (e.g. "claude-architect", "ceo", "n8n")
      to_entity: целевой пост (slug, e.g. "office-manager", "engineer")
                 ИЛИ "dispatcher" для send_to_dispatcher паттерна (тогда
                 Диспетчер сам определит конечный пост)
      payload: JSON-объект с raw_prompt / target_hint / expected_artifact / ...

    Returns:
      {"task_id": "task-uuid", "status": "in_progress", "created_at": "..."}
    """
    params = {"from": from_entity, "to": to_entity}
    if payload is not None:
        params["payload"] = payload
    return _run(_call("dispatcher/submit", params))


@mcp.tool()
def list_posts() -> list[dict]:
    """Список всех постов MSPro (id, slug, title, department, status, prompt size).

    Read-only — алиас на query_sql со специальным SELECT.
    """
    rows = _run(
        _call(
            "sql/query",
            {
                "query": (
                    "SELECT p.id, p.slug, p.title, p.department_id, "
                    "       d.name AS department_name, d.dept_number, "
                    "       p.status, LENGTH(p.system_prompt_md) AS prompt_size_bytes, "
                    "       p.vault_subdir, p.preferred_model, p.claude_agent_name "
                    "FROM posts p "
                    "LEFT JOIN departments d ON d.id = p.department_id "
                    "ORDER BY d.dept_number, p.created_at"
                )
            },
        )
    )
    return rows


@mcp.tool()
def get_task(task_id: str) -> dict | None:
    """Подробности одной задачи по task_id."""
    rows = _run(
        _call(
            "sql/query",
            {
                "query": (
                    f"SELECT id, from_entity, to_entity, status, hop_kind, "
                    f"       parent_task_id, attempts_count, outbox_path, "
                    f"       routed_by_model, completed_at, created_at, "
                    f"       task_payload, refined_prompt "
                    f"FROM dispatcher_logs WHERE id = '{task_id.replace(chr(39), '')}'"
                )
            },
        )
    )
    return rows[0] if rows else None


@mcp.tool()
def get_task_chain(task_id: str) -> dict:
    """Полная цепочка hops от raw_request до результата + decisions + artifacts.

    Если task_id это refined/subtask — поднимаемся к корню через parent_task_id.

    Returns dict:
      {"root": {...row корневой raw_request...},
       "hops": [...всё что произошло с этим chain...],
       "decisions": [...решения Диспетчера...],
       "artifacts": [...файлы в Outbox...]}
    """
    safe_id = task_id.replace("'", "")

    # 1. Найти корень chain: подняться по parent_task_id пока NULL
    chain_rows = _run(
        _call(
            "sql/query",
            {
                "query": (
                    f"WITH RECURSIVE up(id, parent) AS ("
                    f"  SELECT id, parent_task_id FROM dispatcher_logs WHERE id = '{safe_id}' "
                    f"  UNION ALL "
                    f"  SELECT dl.id, dl.parent_task_id FROM dispatcher_logs dl "
                    f"  JOIN up ON dl.id = up.parent"
                    f") "
                    f"SELECT id FROM up WHERE parent IS NULL LIMIT 1"
                )
            },
        )
    )
    root_id = chain_rows[0]["id"] if chain_rows else safe_id

    # 2. Все hops chain (root + всё что parent_task_id ссылается на цепь)
    all_hops = _run(
        _call(
            "sql/query",
            {
                "query": (
                    f"WITH RECURSIVE chain(id) AS ("
                    f"  SELECT id FROM dispatcher_logs WHERE id = '{root_id}' "
                    f"  UNION ALL "
                    f"  SELECT dl.id FROM dispatcher_logs dl JOIN chain c ON dl.parent_task_id = c.id"
                    f") "
                    f"SELECT dl.* FROM dispatcher_logs dl JOIN chain c ON dl.id = c.id "
                    f"ORDER BY dl.created_at"
                )
            },
        )
    )

    # 3. Decisions
    chain_ids = [h["id"] for h in all_hops]
    decisions = []
    if chain_ids:
        ids_sql = ",".join(f"'{i}'" for i in chain_ids)
        decisions = _run(
            _call(
                "sql/query",
                {
                    "query": (
                        f"SELECT * FROM dispatcher_decisions "
                        f"WHERE source_task_id IN ({ids_sql}) ORDER BY created_at"
                    )
                },
            )
        )

    # 4. Artifacts
    artifacts = []
    if chain_ids:
        ids_sql = ",".join(f"'{i}'" for i in chain_ids)
        artifacts = _run(
            _call(
                "sql/query",
                {
                    "query": (
                        f"SELECT * FROM task_artifacts WHERE task_id IN ({ids_sql}) "
                        f"ORDER BY created_at"
                    )
                },
            )
        )

    root_row = next((h for h in all_hops if h["id"] == root_id), all_hops[0] if all_hops else None)
    return {
        "root": root_row,
        "hops": all_hops,
        "decisions": decisions,
        "artifacts": artifacts,
    }


if __name__ == "__main__":
    mcp.run()
