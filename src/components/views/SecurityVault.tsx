import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import AddSecretModal from "../vault/AddSecretModal";

export interface VaultMeta {
  id: string;
  key_name: string;
  description: string | null;
  access_level: number;
  credential_target: string;
  created_at: string;
  updated_at: string;
}

const ACCESS_LABELS: Record<number, { label: string; color: string }> = {
  0: { label: "0 · public", color: "#888" },
  1: { label: "1 · heads", color: "#1976d2" },
  2: { label: "2 · ceo", color: "#7b1fa2" },
  3: { label: "3 · owner", color: "#c62828" },
};

export default function SecurityVault() {
  const [secrets, setSecrets] = useState<VaultMeta[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [showAdd, setShowAdd] = useState(false);
  const [revealed, setRevealed] = useState<{ key: string; value: string } | null>(null);

  async function refresh() {
    setLoading(true);
    setError(null);
    try {
      const list = await invoke<VaultMeta[]>("vault_list_secrets");
      setSecrets(list);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    refresh();
  }, []);

  async function onReveal(key: string) {
    if (!window.confirm(`Раскрыть значение «${key}»? Оно появится на экране.`)) return;
    try {
      const value = await invoke<string>("vault_reveal_secret", { keyName: key });
      setRevealed({ key, value });
    } catch (e) {
      setError(String(e));
    }
  }

  async function onDelete(key: string) {
    if (!window.confirm(`Удалить секрет «${key}» безвозвратно?`)) return;
    try {
      await invoke("vault_remove_secret", { keyName: key });
      setSecrets((prev) => prev.filter((s) => s.key_name !== key));
    } catch (e) {
      setError(String(e));
    }
  }

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
    } catch (e) {
      console.warn(e);
    }
  }

  return (
    <div style={{ padding: "32px 48px", overflowY: "auto", maxWidth: 1200 }}>
      <header style={{ borderBottom: "2px solid #1a1a1a", paddingBottom: 16, marginBottom: 24 }}>
        <h1 style={{ margin: 0, fontSize: 28 }}>🔐 Отдел СБ</h1>
        <p style={{ margin: "4px 0 0", color: "#666", fontSize: 14 }}>
          Хранилище API-ключей и секретов. Значения зашифрованы DPAPI и лежат в Windows Credential
          Manager — в этой базе только метаданные.
        </p>
      </header>

      <div style={{ display: "flex", alignItems: "center", gap: 12, marginBottom: 16 }}>
        <button
          type="button"
          onClick={() => setShowAdd(true)}
          style={{
            padding: "10px 18px",
            background: "#1a1a1a",
            color: "#fff",
            border: "none",
            borderRadius: 6,
            cursor: "pointer",
            fontSize: 14,
            fontWeight: 600,
          }}
        >
          + Новый секрет
        </button>
        <button
          type="button"
          onClick={refresh}
          style={{
            padding: "10px 18px",
            background: "#fff",
            color: "#1a1a1a",
            border: "1px solid #ccc",
            borderRadius: 6,
            cursor: "pointer",
            fontSize: 14,
          }}
        >
          ↻ Обновить
        </button>
        <span style={{ color: "#888", fontSize: 13, marginLeft: "auto" }}>
          {secrets.length} {secrets.length === 1 ? "ключ" : "ключей"}
        </span>
      </div>

      {error && (
        <div
          style={{
            padding: 12,
            background: "#fee",
            border: "1px solid #c00",
            borderRadius: 4,
            fontSize: 13,
            marginBottom: 16,
            whiteSpace: "pre-wrap",
          }}
        >
          {error}
        </div>
      )}

      {loading && <p style={{ color: "#999" }}>Загружаю…</p>}

      {!loading && secrets.length === 0 && !error && (
        <div
          style={{
            padding: 40,
            background: "#fafafa",
            border: "1px dashed #ccc",
            borderRadius: 8,
            textAlign: "center",
            color: "#888",
          }}
        >
          <p style={{ fontSize: 16, margin: 0 }}>
            🔓 Хранилище пустое. Создай первый секрет — например <code>n8n_api_key</code> или{" "}
            <code>telegram_bot_token</code>.
          </p>
        </div>
      )}

      {secrets.length > 0 && (
        <table style={{ width: "100%", borderCollapse: "collapse", background: "#fff", borderRadius: 6, overflow: "hidden", boxShadow: "0 1px 3px rgba(0,0,0,0.05)" }}>
          <thead>
            <tr style={{ background: "#f5f5f5", textAlign: "left" }}>
              <th style={th}>Ключ</th>
              <th style={th}>Доступ</th>
              <th style={th}>Описание</th>
              <th style={th}>Обновлён</th>
              <th style={{ ...th, textAlign: "right" }}>Действия</th>
            </tr>
          </thead>
          <tbody>
            {secrets.map((s) => {
              const lvl = ACCESS_LABELS[s.access_level] ?? ACCESS_LABELS[0];
              return (
                <tr key={s.id} style={{ borderTop: "1px solid #eee" }}>
                  <td style={{ ...td, fontFamily: "ui-monospace, monospace", fontSize: 13 }}>
                    <strong>{s.key_name}</strong>
                  </td>
                  <td style={td}>
                    <span
                      style={{
                        padding: "2px 10px",
                        borderRadius: 12,
                        background: lvl.color + "22",
                        color: lvl.color,
                        fontSize: 11,
                        fontWeight: 600,
                      }}
                    >
                      {lvl.label}
                    </span>
                  </td>
                  <td style={{ ...td, color: "#666", fontSize: 13 }}>{s.description || "—"}</td>
                  <td style={{ ...td, color: "#999", fontSize: 12 }}>{s.updated_at}</td>
                  <td style={{ ...td, textAlign: "right" }}>
                    <button
                      type="button"
                      onClick={() => onReveal(s.key_name)}
                      style={actionBtn}
                      title="Показать значение"
                    >
                      👁
                    </button>
                    <button
                      type="button"
                      onClick={() => onDelete(s.key_name)}
                      style={{ ...actionBtn, marginLeft: 6, color: "#c00" }}
                      title="Удалить ключ"
                    >
                      🗑
                    </button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}

      {showAdd && (
        <AddSecretModal
          onClose={() => setShowAdd(false)}
          onCreated={(meta) => {
            setSecrets((prev) => {
              const without = prev.filter((p) => p.key_name !== meta.key_name);
              return [meta, ...without];
            });
            setShowAdd(false);
          }}
        />
      )}

      {revealed && (
        <div
          style={overlayStyle}
          onClick={() => setRevealed(null)}
        >
          <div
            style={revealModalStyle}
            onClick={(e) => e.stopPropagation()}
          >
            <h3 style={{ margin: "0 0 12px", fontSize: 16 }}>
              👁 {revealed.key}
            </h3>
            <textarea
              readOnly
              value={revealed.value}
              style={{
                width: "100%",
                minHeight: 80,
                fontFamily: "ui-monospace, monospace",
                fontSize: 13,
                padding: 10,
                border: "1px solid #ccc",
                borderRadius: 4,
                boxSizing: "border-box",
              }}
              onClick={(e) => (e.target as HTMLTextAreaElement).select()}
            />
            <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 12 }}>
              <button type="button" onClick={() => copy(revealed.value)} style={primaryBtn}>
                Скопировать
              </button>
              <button type="button" onClick={() => setRevealed(null)} style={secondaryBtn}>
                Скрыть
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

const th: React.CSSProperties = { padding: "10px 14px", fontWeight: 600, fontSize: 13, color: "#555" };
const td: React.CSSProperties = { padding: "10px 14px", fontSize: 13 };
const actionBtn: React.CSSProperties = {
  background: "transparent",
  border: "1px solid #ddd",
  borderRadius: 4,
  cursor: "pointer",
  padding: "4px 8px",
  fontSize: 14,
};
const overlayStyle: React.CSSProperties = {
  position: "fixed",
  inset: 0,
  background: "rgba(0,0,0,0.5)",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  zIndex: 1000,
};
const revealModalStyle: React.CSSProperties = {
  background: "#fff",
  borderRadius: 8,
  padding: 24,
  width: "min(560px, 92vw)",
};
const primaryBtn: React.CSSProperties = {
  padding: "8px 16px",
  background: "#1a1a1a",
  color: "#fff",
  border: "none",
  borderRadius: 4,
  cursor: "pointer",
  fontSize: 13,
  fontWeight: 600,
};
const secondaryBtn: React.CSSProperties = {
  padding: "8px 16px",
  background: "#fff",
  color: "#1a1a1a",
  border: "1px solid #ccc",
  borderRadius: 4,
  cursor: "pointer",
  fontSize: 13,
};
