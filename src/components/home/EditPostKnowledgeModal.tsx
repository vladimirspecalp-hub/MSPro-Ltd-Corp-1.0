// v1.0.19 — Per-post Knowledge editor.
//
// Открывается из карточки поста (DepartmentCard) кнопкой «🧠 Знания».
// Позволяет Владельцу:
//   1. Задать пер-постовый системный промпт (markdown).
//   2. Импортировать готовую папку .md (например ObsidianVault/) в
//      <app_data>/Vault/posts/<slug>/.
//   3. Открыть папку поста в проводнике для ручного редактирования.
//
// Импорт пути — через текстовое поле (плагин dialog не подключён, лишние
// разрешения не добавляем). Владелец копирует путь из проводника.

import { useEffect, useState, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";

interface Props {
  slug: string;
  title: string;
  onClose: () => void;
}

interface PostKnowledge {
  slug: string;
  title: string;
  system_prompt_md: string | null;
  vault_subdir: string | null;
  vault_abs_path: string | null;
  claude_agent_name: string | null;
  updated_at: string | null;
}

interface ImportResult {
  copied: number;
  vault_abs_path: string;
}

const SYSTEM_PROMPT_MAX = 100_000;

const overlayStyle: React.CSSProperties = {
  position: "fixed",
  inset: 0,
  background: "rgba(0,0,0,0.55)",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  zIndex: 1000,
};

const modalStyle: React.CSSProperties = {
  background: "#fff",
  borderRadius: 8,
  padding: 24,
  width: "min(820px, 96vw)",
  maxHeight: "92vh",
  overflowY: "auto",
  boxShadow: "0 20px 60px rgba(0,0,0,0.35)",
};

const labelStyle: React.CSSProperties = {
  display: "block",
  fontSize: 13,
  fontWeight: 600,
  marginBottom: 6,
  color: "#333",
};

const helpStyle: React.CSSProperties = {
  fontSize: 12,
  color: "#777",
  marginTop: 4,
  marginBottom: 10,
};

const inputStyle: React.CSSProperties = {
  display: "block",
  width: "100%",
  padding: "8px 10px",
  fontSize: 13,
  border: "1px solid #ccc",
  borderRadius: 4,
  fontFamily: "inherit",
  boxSizing: "border-box",
};

const codeStyle: React.CSSProperties = {
  background: "#f5f5f5",
  padding: "2px 6px",
  borderRadius: 3,
  fontFamily: "ui-monospace, monospace",
  fontSize: 11,
  color: "#444",
};

export default function EditPostKnowledgeModal({ slug, title, onClose }: Props) {
  const [data, setData] = useState<PostKnowledge | null>(null);
  const [prompt, setPrompt] = useState("");
  const [importPath, setImportPath] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  // Load on mount
  useEffect(() => {
    (async () => {
      try {
        const k = await invoke<PostKnowledge>("get_post_knowledge", { slug });
        setData(k);
        setPrompt(k.system_prompt_md ?? "");
      } catch (e) {
        setError(String(e));
      }
    })();
  }, [slug]);

  // Auto-grow textarea
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 480)}px`;
  }, [prompt]);

  async function onSave() {
    if (prompt.length > SYSTEM_PROMPT_MAX) {
      setError(`Промпт слишком большой: ${prompt.length} байт (макс ${SYSTEM_PROMPT_MAX})`);
      return;
    }
    setBusy(true);
    setError(null);
    setToast(null);
    try {
      const k = await invoke<PostKnowledge>("update_post_knowledge", {
        input: { slug, system_prompt_md: prompt.trim() ? prompt : null },
      });
      setData(k);
      setToast("Сохранено ✓");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function onImport() {
    if (!importPath.trim()) {
      setError("Укажите путь к исходной папке (вставьте из проводника)");
      return;
    }
    setBusy(true);
    setError(null);
    setToast(null);
    try {
      const res = await invoke<ImportResult>("import_post_vault", {
        input: { slug, src_path: importPath.trim() },
      });
      setToast(`Скопировано .md файлов: ${res.copied} → ${res.vault_abs_path}`);
      // Обновим knowledge, чтобы показать новый vault_abs_path если был NULL
      const k = await invoke<PostKnowledge>("get_post_knowledge", { slug });
      setData(k);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function onOpenInExplorer() {
    setError(null);
    try {
      await invoke("open_post_vault_in_explorer", { slug });
    } catch (e) {
      setError(String(e));
    }
  }

  const promptBytes = prompt.length;
  const promptOver = promptBytes > SYSTEM_PROMPT_MAX;

  return (
    <div style={overlayStyle} onClick={onClose}>
      <div style={modalStyle} onClick={(e) => e.stopPropagation()}>
        <div style={{ display: "flex", alignItems: "baseline", gap: 10, marginBottom: 12 }}>
          <h2 style={{ margin: 0, fontSize: 18 }}>🧠 Знания поста</h2>
          <code style={codeStyle}>{slug}</code>
          <span style={{ fontSize: 13, color: "#555" }}>{title}</span>
        </div>

        {error && (
          <div
            style={{
              background: "#fff2f0",
              border: "1px solid #f5b8b1",
              padding: "8px 12px",
              borderRadius: 4,
              marginBottom: 12,
              fontSize: 13,
              color: "#a51b1b",
              whiteSpace: "pre-wrap",
            }}
          >
            {error}
          </div>
        )}
        {toast && (
          <div
            style={{
              background: "#f0fff4",
              border: "1px solid #b6e3c0",
              padding: "8px 12px",
              borderRadius: 4,
              marginBottom: 12,
              fontSize: 13,
              color: "#1f6f3b",
            }}
          >
            {toast}
          </div>
        )}

        {/* Системный промпт */}
        <label style={labelStyle}>Системный промпт поста (markdown)</label>
        <div style={helpStyle}>
          Что пост умеет, в каком стиле работает, какие инструкции от Владельца. Гендир видит
          этот текст когда ставит посту задачу через <code style={codeStyle}>dispatch_task</code>{" "}
          или вызывает <code style={codeStyle}>read_post_knowledge</code>. Лимит:{" "}
          {SYSTEM_PROMPT_MAX.toLocaleString("ru")} байт.
        </div>
        <textarea
          ref={textareaRef}
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          placeholder={"# Ты — Менеджер MS Office.\n\nТвоя задача — готовить деловые документы (договоры, письма, сметы) в стиле МСПро..."}
          style={{
            ...inputStyle,
            minHeight: 160,
            maxHeight: 480,
            fontFamily: "ui-monospace, Menlo, Consolas, monospace",
            fontSize: 13,
            lineHeight: 1.45,
            resize: "vertical",
            overflowY: "auto",
          }}
        />
        <div style={{ display: "flex", justifyContent: "space-between", marginTop: 4 }}>
          <span style={{ fontSize: 11, color: promptOver ? "#c00" : "#888" }}>
            {promptBytes.toLocaleString("ru")} / {SYSTEM_PROMPT_MAX.toLocaleString("ru")} байт
          </span>
          <button
            type="button"
            disabled={busy || promptOver}
            onClick={onSave}
            style={{
              padding: "6px 14px",
              background: promptOver ? "#999" : "#1a1a1a",
              color: "#fff",
              border: "none",
              borderRadius: 4,
              cursor: busy || promptOver ? "not-allowed" : "pointer",
              fontSize: 13,
              fontWeight: 600,
            }}
          >
            {busy ? "Сохраняю..." : "Сохранить промпт"}
          </button>
        </div>

        <hr style={{ margin: "20px 0", border: "none", borderTop: "1px solid #eee" }} />

        {/* Папка опыта */}
        <label style={labelStyle}>Папка опыта (Vault поста)</label>
        <div style={helpStyle}>
          Изолированная папка с накопленными паттернами (
          <code style={codeStyle}>02-Patterns/</code>) и победами (
          <code style={codeStyle}>04-Wins/</code>) этого поста. Гендир и сам пост (когда
          spawn будет реализован) подмешивают её в каждый запрос.
        </div>
        <div style={{ display: "flex", gap: 8, alignItems: "center", marginBottom: 14 }}>
          <code
            style={{
              ...codeStyle,
              flex: 1,
              padding: "6px 10px",
              fontSize: 12,
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
            title={data?.vault_abs_path ?? "не создано"}
          >
            {data?.vault_abs_path ?? "(будет создано при первом сохранении)"}
          </code>
          <button
            type="button"
            onClick={onOpenInExplorer}
            style={{
              padding: "6px 12px",
              background: "#fff",
              border: "1px solid #ccc",
              borderRadius: 4,
              cursor: "pointer",
              fontSize: 12,
            }}
          >
            📂 Открыть в проводнике
          </button>
        </div>

        {/* Импорт стартового Vault */}
        <label style={labelStyle}>Импорт стартового Vault (опционально)</label>
        <div style={helpStyle}>
          Скопировать все <code style={codeStyle}>.md</code> из готовой папки (например{" "}
          <code style={codeStyle}>C:\CODE\manager\ObsidianVault</code>). Симлинки и
          бинарные файлы игнорируются. Лимит — 500 файлов. Существующие файлы перезаписываются.
        </div>
        <div style={{ display: "flex", gap: 8 }}>
          <input
            type="text"
            value={importPath}
            onChange={(e) => setImportPath(e.target.value)}
            placeholder={"C:\\CODE\\manager\\ObsidianVault"}
            style={{ ...inputStyle, flex: 1 }}
          />
          <button
            type="button"
            disabled={busy || !importPath.trim()}
            onClick={onImport}
            style={{
              padding: "8px 14px",
              background: "#1a1a1a",
              color: "#fff",
              border: "none",
              borderRadius: 4,
              cursor: busy || !importPath.trim() ? "not-allowed" : "pointer",
              fontSize: 13,
              fontWeight: 600,
              whiteSpace: "nowrap",
            }}
          >
            📥 Импортировать
          </button>
        </div>

        <div style={{ marginTop: 24, display: "flex", justifyContent: "flex-end", gap: 8 }}>
          <button
            type="button"
            onClick={onClose}
            style={{
              padding: "8px 16px",
              background: "#fff",
              border: "1px solid #ccc",
              borderRadius: 4,
              cursor: "pointer",
              fontSize: 13,
            }}
          >
            Закрыть
          </button>
        </div>
      </div>
    </div>
  );
}
