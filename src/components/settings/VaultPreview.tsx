import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/**
 * Память Гендира — preview накопленного опыта (Vault).
 * Показывает ровно тот блок, который попадает в system prompt CEO.
 */
export default function VaultPreview() {
  const [content, setContent] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function load() {
    setLoading(true);
    setError(null);
    try {
      const block = await invoke<string>("get_vault_preview");
      setContent(block);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  async function seedExample() {
    setError(null);
    try {
      await invoke<string>("save_pattern", {
        input: {
          title: "Тест-паттерн Vault",
          content:
            "# Тест-паттерн\n\n" +
            "Этот файл создан кнопкой «Положить пример» в Настройках.\n" +
            "Если Гендир в чате его процитирует — Vault работает корректно.\n",
        },
      });
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <section
      style={{
        marginTop: 32,
        padding: 20,
        background: "#fff",
        borderRadius: 8,
        border: "1px solid #ddd",
      }}
    >
      <h2 style={{ margin: "0 0 4px", fontSize: 18 }}>🧠 Память Гендира (Vault)</h2>
      <p style={{ margin: "0 0 16px", color: "#666", fontSize: 13 }}>
        Файлы из <code>%APPDATA%\ru.msproltd.corp\Vault\02-Patterns</code> и{" "}
        <code>04-Wins</code> подмешиваются в системный промпт CEO. Лимит блока — 16 KB.
        Свежие файлы (по mtime) приоритетнее.
      </p>

      <div style={{ display: "flex", gap: 8, marginBottom: 12 }}>
        <button type="button" onClick={load} style={primaryBtn}>
          {loading ? "Читаю…" : "🧠 Показать память"}
        </button>
        <button type="button" onClick={seedExample} style={secondaryBtn}>
          ➕ Положить пример
        </button>
      </div>

      {error && (
        <div style={errBox}>
          <strong>Ошибка:</strong>
          <pre style={{ margin: "4px 0 0", whiteSpace: "pre-wrap", fontSize: 12 }}>{error}</pre>
        </div>
      )}

      {content !== null && !error && (
        <>
          {content.trim().length === 0 ? (
            <div style={emptyBox}>
              Память пуста. Положи файл в <code>Vault\02-Patterns\</code> или нажми
              «Положить пример».
            </div>
          ) : (
            <pre style={previewBox}>{content}</pre>
          )}
          <div style={{ fontSize: 11, color: "#888", marginTop: 6 }}>
            Размер блока: {content.length} символов / 16 000
          </div>
        </>
      )}
    </section>
  );
}

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
const errBox: React.CSSProperties = {
  padding: 12,
  background: "#fee",
  border: "1px solid #c00",
  borderRadius: 4,
  fontSize: 13,
  marginBottom: 12,
};
const emptyBox: React.CSSProperties = {
  padding: 16,
  background: "#fafafa",
  border: "1px dashed #ccc",
  borderRadius: 4,
  color: "#888",
  fontSize: 13,
  textAlign: "center",
};
const previewBox: React.CSSProperties = {
  background: "#1a1a1a",
  color: "#d4d4d4",
  padding: 14,
  borderRadius: 6,
  fontSize: 12,
  fontFamily: "ui-monospace, monospace",
  whiteSpace: "pre-wrap",
  wordBreak: "break-word",
  maxHeight: 480,
  overflowY: "auto",
  margin: 0,
};
