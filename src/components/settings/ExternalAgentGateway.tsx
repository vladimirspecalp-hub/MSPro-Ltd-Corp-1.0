import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface GatewayStatus {
  running: boolean;
  port: number | null;
  since: string | null;
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

const tokenBoxStyle: React.CSSProperties = {
  fontFamily: "ui-monospace, Consolas, monospace",
  background: "#f5f5f5",
  padding: "10px 14px",
  borderRadius: 4,
  fontSize: 13,
  letterSpacing: 1,
  wordBreak: "break-all",
  border: "1px solid #ddd",
};

function maskToken(token: string): string {
  if (token.length < 8) return "••••";
  return `${token.slice(0, 4)}${"•".repeat(token.length - 8)}${token.slice(-4)}`;
}

export default function ExternalAgentGateway() {
  const [status, setStatus] = useState<GatewayStatus>({
    running: false,
    port: null,
    since: null,
  });
  const [token, setToken] = useState<string | null>(null);
  const [revealed, setRevealed] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copyHint, setCopyHint] = useState<string | null>(null);

  useEffect(() => {
    refreshStatus();
  }, []);

  async function refreshStatus() {
    try {
      const s = await invoke<GatewayStatus>("external_agent_status");
      setStatus(s);
      if (s.running) {
        const t = await invoke<string>("external_agent_show_token");
        setToken(t);
      }
    } catch (e) {
      setError(String(e));
    }
  }

  async function toggle(next: boolean) {
    setLoading(true);
    setError(null);
    try {
      if (next) {
        const port = await invoke<number>("external_agent_enable");
        setStatus({ running: true, port, since: new Date().toISOString() });
        const t = await invoke<string>("external_agent_show_token");
        setToken(t);
      } else {
        await invoke("external_agent_disable");
        setStatus({ running: false, port: null, since: null });
        setToken(null);
        setRevealed(false);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  async function rotateToken() {
    if (!window.confirm("Сгенерировать новый токен? Старый перестанет работать.")) return;
    try {
      const fresh = await invoke<string>("external_agent_rotate_token");
      setToken(fresh);
      setRevealed(true);
      setCopyHint("Новый токен сгенерирован");
      setTimeout(() => setCopyHint(null), 3000);
    } catch (e) {
      setError(String(e));
    }
  }

  async function copyToken() {
    if (!token) return;
    try {
      await navigator.clipboard.writeText(token);
      setCopyHint("Скопировано в буфер");
      setTimeout(() => setCopyHint(null), 2000);
    } catch (e) {
      setError(`clipboard: ${e}`);
    }
  }

  const wsUrl =
    status.running && status.port && token
      ? `ws://127.0.0.1:${status.port}/?token=${encodeURIComponent(token)}`
      : null;

  return (
    <section style={sectionStyle}>
      <h2 style={{ marginTop: 0, fontSize: 20 }}>External Agent Gateway (Developer Mode)</h2>
      <p style={{ color: "#666", fontSize: 13, margin: "0 0 16px" }}>
        Открывает локальный WebSocket на 127.0.0.1, через который внешний агент (например, Claude)
        может видеть состояние приложения и выполнять действия. Доступ только по токену, только с
        этого ПК.
      </p>

      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 16,
          marginBottom: 16,
          padding: 12,
          background: status.running ? "#e8f5e9" : "#fafafa",
          borderRadius: 4,
        }}
      >
        <div style={{ flex: 1 }}>
          <div style={{ fontWeight: 600 }}>
            {status.running ? "🟢 Включено" : "⚪ Выключено"}
          </div>
          {status.running && status.port && (
            <div style={{ fontSize: 12, color: "#666", marginTop: 2 }}>
              Слушает на 127.0.0.1:{status.port}
              {status.since && ` · с ${new Date(status.since).toLocaleString("ru-RU")}`}
            </div>
          )}
        </div>
        <button
          type="button"
          onClick={() => toggle(!status.running)}
          disabled={loading}
          style={{
            ...buttonStyle,
            background: status.running ? "#c00" : "#4caf50",
          }}
        >
          {loading ? "…" : status.running ? "Выключить" : "Включить"}
        </button>
      </div>

      {status.running && token && (
        <>
          <div style={{ marginBottom: 12 }}>
            <label style={{ fontSize: 13, color: "#666", display: "block", marginBottom: 6 }}>
              Токен доступа
            </label>
            <div style={tokenBoxStyle}>{revealed ? token : maskToken(token)}</div>
            <div style={{ display: "flex", gap: 8, marginTop: 8, flexWrap: "wrap" }}>
              <button
                type="button"
                onClick={() => setRevealed((r) => !r)}
                style={{ ...buttonStyle, padding: "6px 14px", fontSize: 12 }}
              >
                {revealed ? "Скрыть" : "Показать"}
              </button>
              <button
                type="button"
                onClick={copyToken}
                style={{ ...buttonStyle, padding: "6px 14px", fontSize: 12 }}
              >
                Скопировать
              </button>
              <button
                type="button"
                onClick={rotateToken}
                style={{
                  ...buttonStyle,
                  padding: "6px 14px",
                  fontSize: 12,
                  background: "#fff",
                  color: "#1a1a1a",
                  border: "1px solid #1a1a1a",
                }}
              >
                Ротировать
              </button>
              {copyHint && <span style={{ fontSize: 12, color: "#4caf50", alignSelf: "center" }}>{copyHint}</span>}
            </div>
          </div>

          {wsUrl && (
            <details>
              <summary style={{ cursor: "pointer", color: "#666", fontSize: 13 }}>
                Как подключиться
              </summary>
              <div style={{ marginTop: 8, fontSize: 12, color: "#555" }}>
                <p style={{ margin: "0 0 6px" }}>WebSocket URL:</p>
                <div style={{ ...tokenBoxStyle, fontSize: 11 }}>{wsUrl}</div>
                <p style={{ margin: "10px 0 6px" }}>Пример (wscat):</p>
                <div style={{ ...tokenBoxStyle, fontSize: 11 }}>
                  npx wscat -c "{wsUrl}"
                  <br />
                  &gt; {"{"}"jsonrpc":"2.0","id":1,"method":"ping"{"}"}
                </div>
              </div>
            </details>
          )}
        </>
      )}

      {error && (
        <div
          style={{
            marginTop: 12,
            padding: 12,
            background: "#fee",
            border: "1px solid #c00",
            borderRadius: 4,
            fontSize: 13,
            whiteSpace: "pre-wrap",
          }}
        >
          {error}
        </div>
      )}
    </section>
  );
}
