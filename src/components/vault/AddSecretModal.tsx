import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { VaultMeta } from "../views/SecurityVault";

interface Props {
  onClose: () => void;
  onCreated: (meta: VaultMeta) => void;
}

const KEY_RE = /^[a-z0-9](?:[a-z0-9_\-]{0,58}[a-z0-9])?$/;

export default function AddSecretModal({ onClose, onCreated }: Props) {
  const [keyName, setKeyName] = useState("");
  const [value, setValue] = useState("");
  const [description, setDescription] = useState("");
  const [accessLevel, setAccessLevel] = useState(0);
  const [showValue, setShowValue] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const keyValid = KEY_RE.test(keyName);
  const valueValid = value.length > 0;
  const canSubmit = keyValid && valueValid && !submitting;

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!canSubmit) return;
    setSubmitting(true);
    setError(null);
    try {
      const meta = await invoke<VaultMeta>("vault_add_secret", {
        input: {
          key_name: keyName.trim(),
          value,
          description: description.trim() || null,
          access_level: accessLevel,
        },
      });
      onCreated(meta);
    } catch (e) {
      setError(String(e));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div style={overlay} onClick={onClose} role="dialog" aria-modal="true">
      <div style={modal} onClick={(e) => e.stopPropagation()}>
        <h2 style={{ marginTop: 0, fontSize: 20 }}>🔐 Новый секрет</h2>

        <form onSubmit={submit}>
          <div style={field}>
            <label style={label} htmlFor="vault-key">Имя ключа</label>
            <input
              id="vault-key"
              style={{
                ...input,
                fontFamily: "ui-monospace, monospace",
                borderColor: keyName && !keyValid ? "#c00" : "#ccc",
              }}
              value={keyName}
              onChange={(e) => setKeyName(e.target.value.toLowerCase())}
              placeholder="n8n_api_key"
              maxLength={60}
              autoFocus
            />
            <div style={help}>
              латиница a-z, цифры, _ и -, 2–60 символов
              {keyName && !keyValid && <span style={{ color: "#c00" }}> · недопустимый формат</span>}
            </div>
          </div>

          <div style={field}>
            <label style={label} htmlFor="vault-value">Значение</label>
            <div style={{ display: "flex", gap: 6 }}>
              <input
                id="vault-value"
                type={showValue ? "text" : "password"}
                style={{ ...input, flex: 1, fontFamily: "ui-monospace, monospace" }}
                value={value}
                onChange={(e) => setValue(e.target.value)}
                placeholder="sk-xxxxxxxxxxxx..."
              />
              <button
                type="button"
                onClick={() => setShowValue((v) => !v)}
                style={{ ...input, width: 44, cursor: "pointer", background: "#f5f5f5" }}
                title={showValue ? "Скрыть" : "Показать"}
              >
                {showValue ? "🙈" : "👁"}
              </button>
            </div>
            <div style={help}>не логируется, попадает только в Windows Credential Manager</div>
          </div>

          <div style={field}>
            <label style={label} htmlFor="vault-desc">Описание (опционально)</label>
            <input
              id="vault-desc"
              style={input}
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="API key for n8n workflows"
              maxLength={200}
            />
          </div>

          <div style={field}>
            <label style={label} htmlFor="vault-level">Уровень доступа</label>
            <select
              id="vault-level"
              style={input}
              value={accessLevel}
              onChange={(e) => setAccessLevel(parseInt(e.target.value, 10))}
            >
              <option value={0}>0 · public — может читать любая роль</option>
              <option value={1}>1 · heads — главы отделов и выше</option>
              <option value={2}>2 · ceo — Гендир и Владелец</option>
              <option value={3}>3 · owner — только Владелец</option>
            </select>
          </div>

          {error && (
            <div style={errBox}>{error}</div>
          )}

          <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 8 }}>
            <button type="button" onClick={onClose} style={cancelBtn}>Отмена</button>
            <button type="submit" disabled={!canSubmit} style={submitBtn(canSubmit)}>
              {submitting ? "Сохраняю…" : "Сохранить"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

const overlay: React.CSSProperties = {
  position: "fixed", inset: 0, background: "rgba(0,0,0,0.5)",
  display: "flex", alignItems: "center", justifyContent: "center", zIndex: 1000,
};
const modal: React.CSSProperties = {
  background: "#fff", borderRadius: 8, padding: 24,
  width: "min(560px, 92vw)", maxHeight: "90vh", overflowY: "auto",
};
const field: React.CSSProperties = { marginBottom: 14 };
const label: React.CSSProperties = { display: "block", fontSize: 13, fontWeight: 600, marginBottom: 6, color: "#333" };
const input: React.CSSProperties = {
  display: "block", width: "100%", padding: "10px 12px", fontSize: 14,
  border: "1px solid #ccc", borderRadius: 4, fontFamily: "inherit", boxSizing: "border-box",
};
const help: React.CSSProperties = { fontSize: 11, color: "#888", marginTop: 4 };
const errBox: React.CSSProperties = {
  padding: 12, background: "#fee", border: "1px solid #c00", borderRadius: 4,
  fontSize: 13, marginBottom: 14, whiteSpace: "pre-wrap",
};
const cancelBtn: React.CSSProperties = {
  padding: "10px 20px", background: "#fff", color: "#333",
  border: "1px solid #ccc", borderRadius: 4, cursor: "pointer", fontSize: 14,
};
const submitBtn = (enabled: boolean): React.CSSProperties => ({
  padding: "10px 20px", background: enabled ? "#4caf50" : "#aaa", color: "#fff",
  border: "none", borderRadius: 4, cursor: enabled ? "pointer" : "not-allowed",
  fontSize: 14, fontWeight: 600,
});
