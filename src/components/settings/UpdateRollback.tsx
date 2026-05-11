import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface UpdateInfo {
  version: string;
  current_version: string;
  date: string | null;
  body: string | null;
}

interface BackupEntry {
  filename: string;
  version: string;
  created_at: string;
  size_bytes: number;
  path: string;
}

interface UpdateProgressEvent {
  downloaded: number;
  total: number;
  percent: number;
}

const sectionStyle: React.CSSProperties = {
  border: "1px solid #ddd",
  borderRadius: 8,
  padding: 24,
  marginBottom: 24,
  background: "#fff",
};

const buttonStyle: React.CSSProperties = {
  padding: "10px 20px",
  background: "#1a1a1a",
  color: "#fff",
  border: "none",
  borderRadius: 4,
  cursor: "pointer",
  fontSize: 14,
  fontWeight: 600,
};

const dangerButtonStyle: React.CSSProperties = {
  ...buttonStyle,
  background: "#fff",
  color: "#c00",
  border: "1px solid #c00",
};

export default function UpdateRollback() {
  const [currentVersion, setCurrentVersion] = useState<string>("…");
  const [available, setAvailable] = useState<UpdateInfo | null>(null);
  const [checking, setChecking] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<UpdateProgressEvent | null>(null);
  const [backups, setBackups] = useState<BackupEntry[]>([]);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    (async () => {
      try {
        const info = await invoke<{ name: string; version: string }>("app_info");
        setCurrentVersion(info.version);
        await refreshBackups();
      } catch (e) {
        setError(String(e));
      }
    })();
    const unlistenPromise = listen<UpdateProgressEvent>("update-progress", (event) => {
      setProgress(event.payload);
    });
    return () => {
      unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  async function refreshBackups() {
    try {
      const list = await invoke<BackupEntry[]>("list_backups_cmd");
      setBackups(list);
    } catch (e) {
      console.warn("list_backups_cmd:", e);
    }
  }

  async function checkForUpdate() {
    setChecking(true);
    setError(null);
    setMessage(null);
    try {
      const info = await invoke<UpdateInfo | null>("check_for_update");
      if (info) {
        setAvailable(info);
        setMessage(`Доступна версия ${info.version}`);
      } else {
        setAvailable(null);
        setMessage("У вас последняя версия");
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setChecking(false);
    }
  }

  async function installUpdate() {
    setInstalling(true);
    setError(null);
    try {
      await invoke("install_update_with_backup");
      // App restarts on success — this line typically never runs.
    } catch (e) {
      setError(String(e));
      setInstalling(false);
    }
  }

  async function rollbackTo(version: string) {
    if (!window.confirm(`Откатить на версию ${version}? Приложение перезапустится.`)) {
      return;
    }
    try {
      await invoke("rollback_to", { version });
      // App exits — line typically not reached.
    } catch (e) {
      setError(String(e));
    }
  }

  function formatBytes(b: number): string {
    if (b > 1_000_000) return `${(b / 1_000_000).toFixed(1)} МБ`;
    if (b > 1000) return `${(b / 1000).toFixed(1)} КБ`;
    return `${b} Б`;
  }

  return (
    <section style={sectionStyle}>
      <h2 style={{ marginTop: 0, fontSize: 20 }}>Обновления и откат</h2>
      <div style={{ marginBottom: 16, color: "#666" }}>
        Текущая версия: <strong style={{ color: "#1a1a1a" }}>v{currentVersion}</strong>
      </div>

      <div style={{ display: "flex", gap: 12, marginBottom: 16, flexWrap: "wrap" }}>
        <button
          type="button"
          onClick={checkForUpdate}
          disabled={checking || installing}
          style={buttonStyle}
        >
          {checking ? "Проверяю…" : "🔄 Проверить обновления"}
        </button>
        {available && !installing && (
          <button type="button" onClick={installUpdate} style={{ ...buttonStyle, background: "#4caf50" }}>
            ⬇ Обновить до v{available.version}
          </button>
        )}
        {installing && progress && (
          <div
            style={{
              padding: "10px 16px",
              border: "1px solid #ddd",
              borderRadius: 4,
              fontSize: 13,
              minWidth: 240,
            }}
          >
            Скачиваю… {progress.percent}% ({formatBytes(progress.downloaded)} из{" "}
            {formatBytes(progress.total)})
          </div>
        )}
      </div>

      {message && (
        <div
          style={{
            padding: 12,
            background: "#e8f5e9",
            border: "1px solid #4caf50",
            borderRadius: 4,
            marginBottom: 16,
            fontSize: 14,
          }}
        >
          {message}
        </div>
      )}
      {error && (
        <div
          style={{
            padding: 12,
            background: "#fee",
            border: "1px solid #c00",
            borderRadius: 4,
            marginBottom: 16,
            fontSize: 13,
            whiteSpace: "pre-wrap",
          }}
        >
          {error}
        </div>
      )}
      {available?.body && (
        <details style={{ marginBottom: 16 }}>
          <summary style={{ cursor: "pointer", color: "#666" }}>Что нового</summary>
          <pre
            style={{
              marginTop: 8,
              padding: 12,
              background: "#fafafa",
              borderRadius: 4,
              fontSize: 12,
              whiteSpace: "pre-wrap",
            }}
          >
            {available.body}
          </pre>
        </details>
      )}

      <div style={{ borderTop: "1px solid #eee", paddingTop: 16, marginTop: 16 }}>
        <h3 style={{ fontSize: 16, margin: "0 0 12px" }}>Откат на предыдущую версию</h3>
        {backups.length === 0 ? (
          <p style={{ color: "#999", fontSize: 13 }}>
            Бэкапов пока нет. Они создаются автоматически перед каждым обновлением.
          </p>
        ) : (
          <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
            {backups.map((b) => (
              <li
                key={b.filename}
                style={{
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "space-between",
                  padding: "10px 12px",
                  marginBottom: 6,
                  background: "#fafafa",
                  borderRadius: 4,
                  fontSize: 13,
                }}
              >
                <span>
                  <strong>v{b.version}</strong>{" "}
                  <span style={{ color: "#999" }}>· {b.created_at} · {formatBytes(b.size_bytes)}</span>
                </span>
                <button
                  type="button"
                  onClick={() => rollbackTo(b.version)}
                  style={dangerButtonStyle}
                >
                  Откатить
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}
