import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import Database from "@tauri-apps/plugin-sql";
import DepartmentCard, { type Department } from "../home/DepartmentCard";

interface AppInfo {
  name: string;
  version: string;
}

export default function Home() {
  const [pong, setPong] = useState<string>("…");
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [depts, setDepts] = useState<Department[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    (async () => {
      try {
        setPong(await invoke<string>("ping"));
        setInfo(await invoke<AppInfo>("app_info"));
        const db = await Database.load("sqlite:app.db");
        const rows = await db.select<Department[]>(
          "SELECT id, dept_number, name, description FROM departments ORDER BY dept_number ASC"
        );
        setDepts(rows);
      } catch (e) {
        setError(String(e));
      }
    })();
  }, []);

  return (
    <div style={{ padding: "32px 48px", overflowY: "auto", maxWidth: 1400 }}>
      <header style={{ borderBottom: "2px solid #1a1a1a", paddingBottom: 16, marginBottom: 24 }}>
        <h1 style={{ margin: 0, fontSize: 28 }}>MSPro-Ltd Corp 1.0</h1>
        <p style={{ margin: "4px 0 0", color: "#666", fontSize: 14 }}>
          {info ? `${info.name} v${info.version}` : "загружаю…"} · Rust ping:{" "}
          <strong>{pong}</strong>
        </p>
      </header>

      {error && (
        <div
          style={{
            padding: 16,
            background: "#fee",
            border: "1px solid #c00",
            borderRadius: 4,
            marginBottom: 24,
          }}
        >
          <strong>Ошибка инициализации:</strong>
          <pre style={{ margin: "8px 0 0", whiteSpace: "pre-wrap", fontSize: 12 }}>{error}</pre>
        </div>
      )}

      <section>
        <h2 style={{ fontSize: 20, marginBottom: 16 }}>
          Оргструктура — 8 отделений ({depts.length})
        </h2>
        {depts.length === 0 && !error && <p style={{ color: "#999" }}>загружаю из SQLite…</p>}
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "repeat(auto-fill, minmax(380px, 1fr))",
            gap: 16,
          }}
        >
          {depts.map((d) => (
            <DepartmentCard key={d.id} dept={d} />
          ))}
        </div>
      </section>
    </div>
  );
}
