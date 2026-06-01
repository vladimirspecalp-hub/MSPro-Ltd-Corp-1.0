"""Runtime-smoke proof (read-only) — Iteration B Slice 1.

Открывает app.db в READ-ONLY (file:...?mode=ro) и одним консистентным
снапшотом (BEGIN) читает доказательства PAL-прогона:
  - provider_registry: 3 провайдера (R-T-006 self-healing проверка)
  - run_logs: последние строки (provider_id, model, latency, success)
  - dispatcher_logs JOIN run_logs: связь задача↔PAL-прогон
Вывод печатается + дублируется в файл (proof.txt) — пруф из файла, не консоли.

Usage:  python proof_run_logs.py
Прод НЕ трогает (mode=ro). Анти-race: один BEGIN-снапшот (урок Day 4).
"""
import os
import sqlite3
import sys

APPDATA = os.environ.get("APPDATA", "")
DB = os.path.join(APPDATA, "ru.msproltd.corp", "app.db")
OUT = os.path.join(os.path.dirname(__file__), "proof.txt")


def main() -> int:
    if not os.path.exists(DB):
        print(f"DB not found: {DB}")
        return 2
    uri = f"file:{DB}?mode=ro&immutable=0"
    lines = []

    def emit(s: str = "") -> None:
        print(s)
        lines.append(s)

    conn = sqlite3.connect(uri, uri=True, timeout=5)
    try:
        conn.row_factory = sqlite3.Row
        cur = conn.cursor()
        cur.execute("BEGIN")  # один консистентный снапшот

        def table_exists(name: str) -> bool:
            return cur.execute(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?", (name,)
            ).fetchone() is not None

        emit("=== provider_registry (R-T-006: ожидаем 3) ===")
        if not table_exists("provider_registry"):
            emit("  NO TABLE — миграция 08 ещё не применена (значит app < 1.0.34 ИЛИ апгрейд не прошёл)")
        else:
            rows = cur.execute(
                "SELECT id, kind, default_model, status FROM provider_registry ORDER BY id"
            ).fetchall()
            emit(f"COUNT={len(rows)}")
            for r in rows:
                emit(f"  {r['id']:18} kind={r['kind']:16} model={r['default_model']!r:20} status={r['status']}")

        emit("")
        emit("=== run_logs (последние 5) ===")
        if not table_exists("run_logs"):
            emit("  NO TABLE — миграция 08 ещё не применена")
            conn.rollback()
            with open(OUT, "w", encoding="utf-8") as f:
                f.write("\n".join(lines) + "\n")
            emit(f"[written] {OUT}")
            return 0
        try:
            rl = cur.execute(
                "SELECT id, task_id, post_slug, provider_id, model_used, tier, "
                "latency_ms, success, fallback_used, error_kind, created_at "
                "FROM run_logs ORDER BY created_at DESC LIMIT 5"
            ).fetchall()
            emit(f"COUNT_TOTAL={cur.execute('SELECT count(*) c FROM run_logs').fetchone()['c']}")
            for r in rl:
                emit(
                    f"  task={r['task_id']} post={r['post_slug']} "
                    f"provider={r['provider_id']} model={r['model_used']} tier={r['tier']} "
                    f"latency_ms={r['latency_ms']} success={r['success']} "
                    f"fallback={r['fallback_used']} err={r['error_kind']} at={r['created_at']}"
                )
            if not rl:
                emit("  (пусто — задач через PAL ещё не было)")
        except sqlite3.OperationalError as e:
            emit(f"  run_logs read error: {e}")

        emit("")
        emit("=== JOIN dispatcher_logs ↔ run_logs (последняя PAL-задача) ===")
        try:
            j = cur.execute(
                "SELECT d.id task, d.status d_status, d.outbox_path, "
                "rl.provider_id, rl.model_used, rl.latency_ms, rl.success "
                "FROM run_logs rl JOIN dispatcher_logs d ON d.id = rl.task_id "
                "ORDER BY rl.created_at DESC LIMIT 3"
            ).fetchall()
            for r in j:
                emit(
                    f"  task={r['task']} d_status={r['d_status']} "
                    f"outbox={r['outbox_path']} provider={r['provider_id']} "
                    f"model={r['model_used']} latency={r['latency_ms']} success={r['success']}"
                )
            if not j:
                emit("  (нет связанных строк run_logs↔dispatcher_logs)")
        except sqlite3.OperationalError as e:
            emit(f"  join error: {e}")

        conn.rollback()  # read-only — ничего не коммитим
    finally:
        conn.close()

    with open(OUT, "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")
    emit("")
    emit(f"[written] {OUT}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
