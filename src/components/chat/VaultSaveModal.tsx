// Модальное окно сохранения сообщения CEO в файловую память (Vault).
// Переиспользуется для двух категорий — pattern (02-Patterns) и win (04-Wins).

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Brain, Trophy, Save, X, Loader2 } from "lucide-react";
import { useToast } from "../common/Toast";

export type VaultKind = "pattern" | "win";

interface Props {
  initialKind: VaultKind;
  initialContent: string;
  onClose: () => void;
  onSaved?: (kind: VaultKind, title: string, path: string) => void;
}

const KIND_META: Record<
  VaultKind,
  { label: string; sub: string; icon: typeof Brain; accent: string }
> = {
  pattern: {
    label: "Паттерн",
    sub: "02-Patterns — проверенный алгоритм",
    icon: Brain,
    accent: "#1565c0",
  },
  win: {
    label: "Победа",
    sub: "04-Wins — успешный ход, повторить",
    icon: Trophy,
    accent: "#2e7d32",
  },
};

/**
 * Авто-предложение названия из первой непустой строки контента.
 * Чистит markdown заголовки/жирность, обрезает по слову до 60 символов.
 */
function suggestTitle(content: string): string {
  const lines = content.trim().split("\n");
  const firstNonEmpty = lines.find((l) => l.trim().length > 0) ?? "";
  const cleaned = firstNonEmpty
    .replace(/^#+\s*/, "")
    .replace(/^\*+\s*/, "")
    .replace(/\*\*/g, "")
    .replace(/`/g, "")
    .trim();
  if (!cleaned) return "";
  if (cleaned.length <= 60) return cleaned;
  const cut = cleaned.slice(0, 60);
  const lastSpace = cut.lastIndexOf(" ");
  const base = lastSpace > 30 ? cut.slice(0, lastSpace) : cut;
  return base + "…";
}

export default function VaultSaveModal({
  initialKind,
  initialContent,
  onClose,
  onSaved,
}: Props) {
  const [kind, setKind] = useState<VaultKind>(initialKind);
  const [title, setTitle] = useState<string>(() => suggestTitle(initialContent));
  const [content, setContent] = useState<string>(initialContent);
  const [submitting, setSubmitting] = useState(false);
  const { toast } = useToast();

  // ESC закрывает модалку
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !submitting) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, submitting]);

  const canSubmit = title.trim().length > 0 && content.trim().length > 0 && !submitting;

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!canSubmit) return;
    setSubmitting(true);
    try {
      const cmd = kind === "pattern" ? "save_pattern" : "save_win";
      const path = await invoke<string>(cmd, {
        input: { title: title.trim(), content },
      });
      toast({
        kind: "success",
        text:
          (kind === "pattern" ? "🧠 Паттерн" : "🏆 Победа") +
          " сохранён:\n" +
          shortenPath(path),
      });
      onSaved?.(kind, title.trim(), path);
      onClose();
    } catch (err) {
      toast({ kind: "error", text: `Не удалось сохранить:\n${String(err)}` });
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div
      style={overlay}
      onClick={() => !submitting && onClose()}
      role="dialog"
      aria-modal="true"
      aria-labelledby="vault-save-title"
    >
      <div style={modal} onClick={(e) => e.stopPropagation()}>
        <header style={modalHeader}>
          <h2 id="vault-save-title" style={{ margin: 0, fontSize: 18 }}>
            💾 Сохранить в Память
          </h2>
          <button
            type="button"
            onClick={onClose}
            style={closeBtn}
            aria-label="Закрыть"
            disabled={submitting}
          >
            <X size={18} />
          </button>
        </header>

        <form onSubmit={submit} style={{ padding: "16px 24px 20px" }}>
          {/* Kind picker */}
          <div style={field}>
            <div style={label}>Куда сохранить</div>
            <div style={{ display: "flex", gap: 10 }}>
              {(["pattern", "win"] as const).map((k) => {
                const meta = KIND_META[k];
                const Icon = meta.icon;
                const selected = kind === k;
                return (
                  <button
                    type="button"
                    key={k}
                    onClick={() => setKind(k)}
                    style={{
                      flex: 1,
                      display: "flex",
                      alignItems: "center",
                      gap: 10,
                      padding: "10px 12px",
                      borderRadius: 6,
                      border: `2px solid ${selected ? meta.accent : "#ddd"}`,
                      background: selected ? meta.accent + "10" : "#fff",
                      color: "#1a1a1a",
                      cursor: submitting ? "not-allowed" : "pointer",
                      textAlign: "left",
                      boxShadow: "none",
                    }}
                    disabled={submitting}
                    aria-pressed={selected}
                  >
                    <Icon size={20} color={meta.accent} style={{ flexShrink: 0 }} />
                    <div>
                      <div style={{ fontWeight: 600, fontSize: 13, lineHeight: 1.2 }}>
                        {meta.label}
                      </div>
                      <div style={{ fontSize: 11, color: "#666", marginTop: 2 }}>
                        {meta.sub}
                      </div>
                    </div>
                  </button>
                );
              })}
            </div>
          </div>

          {/* Title */}
          <div style={field}>
            <label style={label} htmlFor="vault-title">
              Название
            </label>
            <input
              id="vault-title"
              type="text"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="Например: Формула Опасности — frontend пост"
              maxLength={120}
              required
              autoFocus
              disabled={submitting}
              style={input}
            />
            <div style={help}>
              Из названия формируется имя файла. Кириллица допустима.
            </div>
          </div>

          {/* Content */}
          <div style={field}>
            <label style={label} htmlFor="vault-content">
              Содержимое
            </label>
            <textarea
              id="vault-content"
              value={content}
              onChange={(e) => setContent(e.target.value)}
              rows={12}
              required
              disabled={submitting}
              style={{
                ...input,
                fontFamily: "ui-monospace, monospace",
                fontSize: 13,
                resize: "vertical",
                minHeight: 180,
              }}
            />
            <div style={help}>Можно отредактировать перед сохранением.</div>
          </div>

          <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 8 }}>
            <button
              type="button"
              onClick={onClose}
              style={cancelBtn}
              disabled={submitting}
            >
              Отмена
            </button>
            <button
              type="submit"
              disabled={!canSubmit}
              style={submitBtn(canSubmit, KIND_META[kind].accent)}
            >
              {submitting ? (
                <>
                  <Loader2 size={16} className="spin" /> Сохраняю…
                </>
              ) : (
                <>
                  <Save size={16} /> Сохранить в Память
                </>
              )}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

function shortenPath(p: string): string {
  const idx = p.toLowerCase().lastIndexOf("vault\\");
  return idx >= 0 ? p.slice(idx) : p;
}

const overlay: React.CSSProperties = {
  position: "fixed",
  inset: 0,
  background: "rgba(0,0,0,0.5)",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  zIndex: 1000,
};
const modal: React.CSSProperties = {
  background: "#fff",
  borderRadius: 8,
  width: "min(620px, 92vw)",
  maxHeight: "92vh",
  overflowY: "auto",
  boxShadow: "0 8px 32px rgba(0,0,0,0.25)",
};
const modalHeader: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  padding: "14px 24px",
  borderBottom: "1px solid #eee",
};
const closeBtn: React.CSSProperties = {
  background: "transparent",
  border: "none",
  cursor: "pointer",
  padding: 4,
  color: "#666",
  display: "flex",
  boxShadow: "none",
};
const field: React.CSSProperties = { marginBottom: 14 };
const label: React.CSSProperties = {
  display: "block",
  fontSize: 13,
  fontWeight: 600,
  marginBottom: 6,
  color: "#333",
};
const input: React.CSSProperties = {
  display: "block",
  width: "100%",
  padding: "10px 12px",
  fontSize: 14,
  border: "1px solid #ccc",
  borderRadius: 4,
  fontFamily: "inherit",
  boxSizing: "border-box",
  background: "#fff",
};
const help: React.CSSProperties = { fontSize: 11, color: "#888", marginTop: 4 };
const cancelBtn: React.CSSProperties = {
  padding: "10px 18px",
  background: "#fff",
  color: "#333",
  border: "1px solid #ccc",
  borderRadius: 4,
  cursor: "pointer",
  fontSize: 14,
  boxShadow: "none",
};
const submitBtn = (enabled: boolean, accent: string): React.CSSProperties => ({
  padding: "10px 18px",
  background: enabled ? accent : "#aaa",
  color: "#fff",
  border: "none",
  borderRadius: 4,
  cursor: enabled ? "pointer" : "not-allowed",
  fontSize: 14,
  fontWeight: 600,
  display: "inline-flex",
  alignItems: "center",
  gap: 8,
  boxShadow: "none",
});
