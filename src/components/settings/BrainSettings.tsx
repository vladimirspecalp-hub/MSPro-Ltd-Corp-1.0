// Шаг 10 — настройки двухконтурного мозга.
// Поля: claude_cli_path / model + qwen_endpoint / model + auto-fallback toggle.
// Status badges дублирует CeoChat-вариант для удобства настройки из вкладки.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import BrainStatusBadges from "../chat/BrainStatusBadges";
import { useToast } from "../common/Toast";

interface AppSettings {
  claude_cli_path: string;
  claude_cli_model: string;
  qwen_endpoint: string;
  qwen_model: string;
  auto_fallback_qwen: boolean;
  brain_mode?: string;
}

export default function BrainSettings() {
  const [s, setS] = useState<AppSettings | null>(null);
  const [editing, setEditing] = useState<Partial<AppSettings>>({});
  const [saving, setSaving] = useState(false);
  const { toast } = useToast();

  useEffect(() => {
    (async () => {
      try {
        const fetched = await invoke<AppSettings>("get_settings");
        setS(fetched);
      } catch (e) {
        toast({ kind: "error", text: `Не удалось загрузить настройки: ${String(e)}` });
      }
    })();
  }, [toast]);

  if (!s) {
    return <p style={{ color: "#999", fontSize: 13 }}>Загружаю настройки…</p>;
  }

  function pending<K extends keyof AppSettings>(key: K, fallback: AppSettings[K]): AppSettings[K] {
    return (editing[key] ?? s?.[key] ?? fallback) as AppSettings[K];
  }

  async function saveField(field: "claude_cli_path" | "claude_cli_model" | "qwen_endpoint" | "qwen_model") {
    const value = (editing[field] ?? "") as string;
    if (!value.trim()) {
      toast({ kind: "error", text: `Поле ${field} не может быть пустым` });
      return;
    }
    setSaving(true);
    try {
      await invoke("set_brain_string_field", { field, value: value.trim() });
      setS((prev) => (prev ? { ...prev, [field]: value.trim() } : prev));
      setEditing((prev) => {
        const { [field]: _omit, ...rest } = prev;
        return rest;
      });
      toast({ kind: "success", text: "Сохранено" });
    } catch (e) {
      toast({ kind: "error", text: `Не удалось сохранить: ${String(e)}` });
    } finally {
      setSaving(false);
    }
  }

  async function toggleFallback(next: boolean) {
    setSaving(true);
    try {
      await invoke("set_auto_fallback_qwen", { enabled: next });
      setS((prev) => (prev ? { ...prev, auto_fallback_qwen: next } : prev));
      toast({ kind: "success", text: next ? "Auto-fallback включён" : "Auto-fallback выключен" });
    } catch (e) {
      toast({ kind: "error", text: `Не удалось сохранить: ${String(e)}` });
    } finally {
      setSaving(false);
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
      <h2 style={{ margin: "0 0 4px", fontSize: 18 }}>🧠 Двухконтурный Мозг</h2>
      <p style={{ margin: "0 0 16px", color: "#666", fontSize: 13 }}>
        Основной контур — Claude 4.7 Opus через локально установленный CLI
        (<code>claude</code> от Anthropic). Резервный — Qwen 3 локально (Ollama / LM Studio).
        При недоступности Claude автоматически переходим на Qwen.
      </p>

      <div style={{ marginBottom: 16 }}>
        <BrainStatusBadges />
      </div>

      <h3 style={{ margin: "16px 0 8px", fontSize: 14 }}>⭐ Основной — Claude CLI</h3>

      <Field
        label="Путь к Claude CLI"
        value={pending("claude_cli_path", "claude")}
        onChange={(v) => setEditing((p) => ({ ...p, claude_cli_path: v }))}
        onSave={() => saveField("claude_cli_path")}
        hint="«claude» если в PATH, или абсолютный путь до .exe, или «wsl claude» если в WSL"
        saving={saving}
        edited={editing.claude_cli_path !== undefined && editing.claude_cli_path !== s.claude_cli_path}
      />

      <Field
        label="Модель Claude"
        value={pending("claude_cli_model", "claude-opus-4-7")}
        onChange={(v) => setEditing((p) => ({ ...p, claude_cli_model: v }))}
        onSave={() => saveField("claude_cli_model")}
        hint="claude-opus-4-7 (рекомендуется) / claude-sonnet-4-7 (быстрее) / claude-haiku-4-5 (дёшево)"
        saving={saving}
        edited={editing.claude_cli_model !== undefined && editing.claude_cli_model !== s.claude_cli_model}
      />

      <h3 style={{ margin: "20px 0 8px", fontSize: 14 }}>🐉 Резервный — Qwen 3 local</h3>

      <Field
        label="Endpoint Qwen"
        value={pending("qwen_endpoint", "http://localhost:11434/v1")}
        onChange={(v) => setEditing((p) => ({ ...p, qwen_endpoint: v }))}
        onSave={() => saveField("qwen_endpoint")}
        hint="Ollama: http://localhost:11434/v1 · LM Studio: http://localhost:1234/v1"
        saving={saving}
        edited={editing.qwen_endpoint !== undefined && editing.qwen_endpoint !== s.qwen_endpoint}
      />

      <Field
        label="Модель Qwen"
        value={pending("qwen_model", "qwen3:32b")}
        onChange={(v) => setEditing((p) => ({ ...p, qwen_model: v }))}
        onSave={() => saveField("qwen_model")}
        hint="qwen3:32b (рекомендуется при 32+ GB RAM) / qwen3:14b / qwen3:7b"
        saving={saving}
        edited={editing.qwen_model !== undefined && editing.qwen_model !== s.qwen_model}
      />

      <h3 style={{ margin: "20px 0 8px", fontSize: 14 }}>🔁 Auto-fallback</h3>
      <label style={{ display: "flex", alignItems: "center", gap: 10, cursor: "pointer" }}>
        <input
          type="checkbox"
          checked={s.auto_fallback_qwen}
          onChange={(e) => toggleFallback(e.target.checked)}
          disabled={saving}
        />
        <span style={{ fontSize: 13 }}>
          При недоступности Claude автоматически переключаться на Qwen 3
          (в чате появится жёлтая плашка)
        </span>
      </label>
    </section>
  );
}

function Field({
  label,
  value,
  onChange,
  onSave,
  hint,
  saving,
  edited,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  onSave: () => void;
  hint: string;
  saving: boolean;
  edited: boolean;
}) {
  return (
    <div style={{ marginBottom: 12 }}>
      <label style={{ display: "block", fontSize: 12, fontWeight: 600, marginBottom: 4, color: "#333" }}>
        {label}
      </label>
      <div style={{ display: "flex", gap: 6 }}>
        <input
          type="text"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          style={{
            flex: 1,
            padding: "8px 10px",
            border: edited ? "1px solid #1565c0" : "1px solid #ccc",
            borderRadius: 4,
            fontSize: 13,
            fontFamily: "ui-monospace, monospace",
            background: "#fff",
            boxSizing: "border-box",
          }}
          disabled={saving}
        />
        {edited && (
          <button
            type="button"
            onClick={onSave}
            disabled={saving}
            style={{
              padding: "6px 14px",
              background: "#1a1a1a",
              color: "#fff",
              border: "none",
              borderRadius: 4,
              cursor: "pointer",
              fontSize: 12,
              fontWeight: 600,
              boxShadow: "none",
            }}
          >
            Сохранить
          </button>
        )}
      </div>
      <div style={{ fontSize: 11, color: "#888", marginTop: 4 }}>{hint}</div>
    </div>
  );
}
