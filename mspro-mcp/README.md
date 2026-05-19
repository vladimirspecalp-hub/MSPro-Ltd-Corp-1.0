# mspro-mcp — MCP server для MSPro-Ltd Corp

FastMCP server который даёт Claude (и любому MCP-клиенту) native tools для
управления работающим MSPro-Ltd Corp desktop приложением.

## Текущий backend: WebSocket Gateway (порт 8899)

В **v1.0.27 Phase 11D Sub-D5** этот сервер общается с MSPro через встроенный
WebSocket gateway (`external_agent::gateway`). После завершения **Sub-D1b**
backend переключится на нативный COM (`pywin32` → `Dispatch("MSProLtdCorp.Application")`)
без изменения API tools — клиенты не заметят разницы.

## Установка

```powershell
pip install fastmcp websockets
```

## Регистрация в Claude Code

В `~/.claude/mcp.json` (или `.mcp.json` проекта):

```json
{
  "mcpServers": {
    "mspro": {
      "command": "python",
      "args": ["C:\\CODE\\MSPro-Ltd Corp 1.0\\mspro-mcp\\server.py"],
      "env": {
        "MSPRO_TOKEN": "<token from MSPro Settings → External Agent Gateway>"
      }
    }
  }
}
```

После перезапуска Claude Code в списке tools появятся:

| Tool | Что делает |
|------|------------|
| `mcp__mspro__ping` | Smoke: версия + uptime |
| `mcp__mspro__get_state` | Полное состояние app (os, memory, db, uptime, gateway) |
| `mcp__mspro__query_sql` | Read-only SELECT/WITH к app.db (LIMIT 1000) |
| `mcp__mspro__list_posts` | Все посты с department + prompt size |
| `mcp__mspro__dispatch_task` | Создать задачу в Диспетчере (от своего имени) |
| `mcp__mspro__get_task` | Подробности одной задачи по id |
| `mcp__mspro__get_task_chain` | Полная цепочка hops (root → refined → subtasks) + decisions + artifacts |

## Безопасность

- Bearer token хранится в DPAPI (Windows Credential Manager)
- WebSocket gateway loopback-only (`127.0.0.1`)
- SQL — только SELECT/WITH, регексом блок INSERT/UPDATE/DELETE/DROP/ALTER/PRAGMA
- `dispatch_task` пишет `from_entity` как есть → audit trail в `dispatcher_logs`

## Smoke test

```powershell
$env:MSPRO_TOKEN = "..."
python "C:\CODE\MSPro-Ltd Corp 1.0\mspro-mcp\server.py"
# (FastMCP запустит stdio server — Ctrl+C для выхода)
```

Для проверки tools — добавь в MCP клиент и вызови `ping`.
