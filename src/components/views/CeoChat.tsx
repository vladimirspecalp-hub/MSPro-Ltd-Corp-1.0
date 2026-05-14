import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import BrainStatusBadges from "../chat/BrainStatusBadges";
import MessageActions from "../chat/MessageActions";
import MessageHoverActions from "../chat/MessageHoverActions";
import VaultSaveModal, { type VaultKind } from "../chat/VaultSaveModal";
import AttachmentButtons from "../chat/AttachmentButtons";
import AttachmentsArea from "../chat/AttachmentsArea";
import {
  readSingleFile,
  validateAdd,
  toPayload,
  type AttachmentItem,
  type FolderBundle,
} from "../../lib/attachments";
import { useToast } from "../common/Toast";

type BrainMode = "claude_cli" | "qwen_local" | "claude_external";

interface AppSettings {
  brain_mode?: BrainMode;
  external_agent_enabled?: boolean;
  [k: string]: unknown;
}

interface ChatMessage {
  id: string;
  role: "owner" | "ceo";
  content: string;
  created_at: string;
}

interface ChatTurn {
  user: ChatMessage;
  ceo: ChatMessage;
}

const containerStyle: React.CSSProperties = {
  display: "flex",
  flexDirection: "column",
  flex: 1,
  background: "#f9f9f9",
  height: "100%",
  minHeight: 0, // критично для корректного flex-shrink дочернего messagesStyle
  position: "relative", // anchor для dropOverlayStyle absolute
};

const headerStyle: React.CSSProperties = {
  padding: "16px 32px",
  borderBottom: "1px solid #ddd",
  background: "#fff",
  flexShrink: 0,
};

const messagesStyle: React.CSSProperties = {
  flex: 1,
  overflowY: "auto",
  padding: "20px 32px",
  minHeight: 0,
};

const inputBarStyle: React.CSSProperties = {
  padding: "12px 32px",
  borderTop: "1px solid #ddd",
  background: "#fff",
  display: "flex",
  gap: 12,
  alignItems: "flex-end",
  flexShrink: 0,
};

const inputStyle: React.CSSProperties = {
  flex: 1,
  padding: "10px 14px",
  border: "1px solid #ccc",
  borderRadius: 6,
  fontSize: 14,
  fontFamily: "inherit",
  resize: "none",
  minHeight: 38,
  maxHeight: 140, // ≈5 строк при line-height 1.4 + padding
  lineHeight: 1.4,
  boxSizing: "border-box",
  overflowY: "auto",
};

const sendBtnStyle = (disabled: boolean): React.CSSProperties => ({
  padding: "10px 22px",
  background: disabled ? "#aaa" : "#1a1a1a",
  color: "#fff",
  border: "none",
  borderRadius: 6,
  cursor: disabled ? "not-allowed" : "pointer",
  fontSize: 14,
  fontWeight: 600,
});

const brainBtnStyle = (active: boolean): React.CSSProperties => ({
  padding: "6px 14px",
  background: active ? "#1a1a1a" : "#fff",
  color: active ? "#fff" : "#1a1a1a",
  border: "1px solid #1a1a1a",
  borderRadius: 6,
  cursor: "pointer",
  fontSize: 12,
  fontWeight: active ? 700 : 500,
});

const cancelBtnStyle: React.CSSProperties = {
  padding: "10px 16px",
  background: "#fff",
  color: "#c00",
  border: "1px solid #c00",
  borderRadius: 6,
  cursor: "pointer",
  fontSize: 13,
  fontWeight: 600,
};

const dropOverlayStyle: React.CSSProperties = {
  position: "absolute",
  inset: 0,
  background: "rgba(26, 115, 232, 0.92)",
  color: "#fff",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  fontSize: 22,
  fontWeight: 600,
  zIndex: 50,
  border: "4px dashed #fff",
  borderRadius: 12,
  pointerEvents: "none",
};

const bubbleStyle = (role: "owner" | "ceo", streaming = false): React.CSSProperties => ({
  maxWidth: "75%",
  padding: "10px 14px",
  borderRadius: 12,
  fontSize: 14,
  lineHeight: 1.4,
  marginBottom: 4,
  whiteSpace: "pre-wrap",
  wordBreak: "break-word",
  alignSelf: role === "owner" ? "flex-end" : "flex-start",
  background: role === "owner" ? "#1a73e8" : streaming ? "#f0f4ff" : "#e8e8e8",
  color: role === "owner" ? "#fff" : "#1a1a1a",
  border: streaming ? "1px dashed #1a73e8" : undefined,
});

/// Step 7 Этап 3 — отличает системное сообщение от обычной реплики CEO.
function isSystemMessage(m: ChatMessage): "success" | "error" | null {
  if (m.role !== "ceo") return null;
  if (m.content.startsWith("⚡")) return "success";
  if (m.content.startsWith("⚠️")) return "error";
  return null;
}

const systemBubbleStyle = (variant: "success" | "error"): React.CSSProperties => ({
  maxWidth: "90%",
  padding: "10px 14px",
  borderRadius: 8,
  fontSize: 13,
  lineHeight: 1.45,
  marginBottom: 4,
  whiteSpace: "pre-wrap",
  wordBreak: "break-word",
  alignSelf: "stretch",
  background: variant === "success" ? "#fff8e1" : "#ffebee",
  color: variant === "success" ? "#e65100" : "#b71c1c",
  border: `1px solid ${variant === "success" ? "#ffb300" : "#ef5350"}`,
  fontWeight: 500,
});

const timestampStyle: React.CSSProperties = {
  fontSize: 10,
  color: "#999",
  marginBottom: 8,
};

function formatTime(iso: string): string {
  try {
    const d = new Date(iso.replace(" ", "T") + (iso.includes("Z") ? "" : "Z"));
    return d.toLocaleTimeString("ru-RU", { hour: "2-digit", minute: "2-digit" });
  } catch {
    return iso;
  }
}

interface StreamingState {
  id: string;
  text: string;
}

export default function CeoChat() {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [streaming, setStreaming] = useState<StreamingState | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [claudeReady, setClaudeReady] = useState(false);
  const [qwenReady, setQwenReady] = useState(false);
  const [brainMode, setBrainMode] = useState<BrainMode>("claude_cli");
  const [externalAgentOn, setExternalAgentOn] = useState(false);
  // Step 7 Этап 2: модалка сохранения сообщения в файловую память Vault
  const [vaultModal, setVaultModal] = useState<
    { kind: VaultKind; content: string } | null
  >(null);
  // Step 8: прикреплённые файлы/папки к следующему сообщению (не persist'ятся)
  const [attachments, setAttachments] = useState<AttachmentItem[]>([]);
  const [folders, setFolders] = useState<FolderBundle[]>([]);
  const [isDraggingOver, setIsDraggingOver] = useState(false);
  const { toast } = useToast();
  const scrollRef = useRef<HTMLDivElement>(null);
  const streamingRef = useRef<StreamingState | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // v1.0.16: автоувеличение высоты textarea по содержимому до ~5 строк (capped maxHeight).
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 140)}px`;
  }, [input]);

  // Keep a ref in sync so the listen callbacks always see latest streaming state
  useEffect(() => {
    streamingRef.current = streaming;
  }, [streaming]);

  // Initial history load + settings snapshot
  useEffect(() => {
    (async () => {
      try {
        const history = await invoke<ChatMessage[]>("list_chat_history", { limit: 200 });
        setMessages(history);
        const s = await invoke<AppSettings>("get_settings");
        if (
          s.brain_mode === "claude_cli" ||
          s.brain_mode === "qwen_local" ||
          s.brain_mode === "claude_external"
        ) {
          setBrainMode(s.brain_mode);
        }
        setExternalAgentOn(!!s.external_agent_enabled);
      } catch (e) {
        setError(String(e));
      }
    })();
  }, []);

  async function switchBrain(next: BrainMode) {
    if (next === brainMode) return;
    if (next === "claude_external" && !externalAgentOn) {
      setError(
        "Сначала включи External Agent в Настройках — Claude (Architect) подключается через тот же WebSocket."
      );
      return;
    }
    if (next === "qwen_local" && !qwenReady) {
      setError(
        "Qwen 3 endpoint не отвечает. Запусти Ollama (`ollama serve`) или LM Studio и проверь endpoint в Настройках."
      );
      return;
    }
    try {
      await invoke("set_brain_mode", { mode: next });
      setBrainMode(next);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  // Tauri event subscriptions for streaming
  useEffect(() => {
    const offStartP = listen<string>("ceo-start", (e) => {
      setStreaming({ id: e.payload, text: "" });
    });
    const offChunkP = listen<string>("ceo-chunk", (e) => {
      const cur = streamingRef.current;
      if (!cur) return;
      const next = cur.text ? cur.text + "\n" + e.payload : e.payload;
      setStreaming({ id: cur.id, text: next });
    });
    const offDoneP = listen<ChatMessage>("ceo-done", (e) => {
      // Promote the (possibly empty) streaming bubble into the persisted CEO
      // row from the event payload. The payload always contains the full
      // final text — even when stdout came in one block and no chunks fired,
      // this guarantees the bubble shows the real answer.
      setMessages((prev) => {
        if (prev.some((m) => m.id === e.payload.id)) return prev;
        return [...prev, e.payload];
      });
      setStreaming(null);
    });
    // Step 7 Этап 3: системные сообщения о выполнении инструментов
    // (⚡ задача поставлена / ⚠️ ошибка инструмента). Эмитятся отдельно
    // от ceo-done, по одному на каждый tool_call.
    const offToolP = listen<ChatMessage>("ceo-tool-result", (e) => {
      setMessages((prev) => {
        if (prev.some((m) => m.id === e.payload.id)) return prev;
        return [...prev, e.payload];
      });
    });
    return () => {
      offStartP.then((f) => f());
      offChunkP.then((f) => f());
      offDoneP.then((f) => f());
      offToolP.then((f) => f());
    };
  }, []);

  // Auto-scroll on new content
  useEffect(() => {
    scrollRef.current?.scrollTo({
      top: scrollRef.current.scrollHeight,
      behavior: "smooth",
    });
  }, [messages.length, streaming?.text]);

  async function send() {
    const text = input.trim();
    if ((!text && attachments.length === 0) || sending) return;
    setSending(true);
    setError(null);

    // Сохраняем snapshot текущих attachments для отправки + сразу очищаем UI
    const attachmentsSnapshot = attachments;
    setInput("");
    setAttachments([]);
    setFolders([]);

    // Optimistic owner bubble — показываем только текст, не attached content
    const optimisticUserId = `tmp-${Date.now()}`;
    setMessages((prev) => [
      ...prev,
      {
        id: optimisticUserId,
        role: "owner",
        content: text || "(только вложения)",
        created_at: new Date().toISOString(),
      },
    ]);
    try {
      const payload = toPayload(attachmentsSnapshot);
      const turn = await invoke<ChatTurn>("send_chat_message", {
        content: text,
        attachments: payload,
      });
      setMessages((prev) =>
        prev.map((m) => (m.id === optimisticUserId ? turn.user : m))
      );
    } catch (e) {
      setError(String(e));
      setMessages((prev) => prev.filter((m) => m.id !== optimisticUserId));
      setInput(text);
      // Восстановить attachments чтобы Владелец мог исправить и переотправить
      setAttachments(attachmentsSnapshot);
    } finally {
      setSending(false);
    }
  }

  // v1.0.13: цитата сообщения в input bar (через MessageHoverActions)
  function handleQuote(text: string) {
    const quoted = text
      .split("\n")
      .map((line) => `> ${line}`)
      .join("\n");
    setInput((prev) => {
      const sep = prev.trim() ? "\n\n" : "";
      return prev + sep + quoted + "\n\n";
    });
  }

  // Step 8: drag-and-drop из проводника + handler добавления через picker
  function addAttachments(items: AttachmentItem[], bundle?: FolderBundle) {
    setAttachments((prev) => [...prev, ...items]);
    if (bundle) setFolders((prev) => [...prev, bundle]);
  }
  function removeAttachment(id: string) {
    setAttachments((prev) => prev.filter((i) => i.id !== id));
    // Если это файл из папки — обновить bundle
    setFolders((prev) =>
      prev
        .map((b) => ({ ...b, itemIds: b.itemIds.filter((x) => x !== id) }))
        .filter((b) => b.itemIds.length > 0),
    );
  }
  function removeFolder(rootName: string) {
    const bundle = folders.find((b) => b.rootName === rootName);
    if (!bundle) return;
    const idSet = new Set(bundle.itemIds);
    setAttachments((prev) => prev.filter((i) => !idSet.has(i.id)));
    setFolders((prev) => prev.filter((b) => b.rootName !== rootName));
  }

  async function handleDrop(e: React.DragEvent<HTMLDivElement>) {
    e.preventDefault();
    e.stopPropagation();
    setIsDraggingOver(false);
    const files = e.dataTransfer?.files;
    if (!files || files.length === 0) return;
    const items: AttachmentItem[] = [];
    for (let i = 0; i < files.length; i++) {
      items.push(await readSingleFile(files[i]));
    }
    const v = validateAdd(attachments, items);
    if (!v.ok) toast({ kind: "error", text: v.message ?? "Лимит превышен" });
    addAttachments(items);
  }
  function handleDragOver(e: React.DragEvent<HTMLDivElement>) {
    if (e.dataTransfer?.types?.includes("Files")) {
      e.preventDefault();
      setIsDraggingOver(true);
    }
  }
  function handleDragLeave(e: React.DragEvent<HTMLDivElement>) {
    // Реагируем только на выход за пределы контейнера, не за пределы вложенных эл-тов
    if (e.currentTarget === e.target) setIsDraggingOver(false);
  }

  async function cancel() {
    try {
      await invoke("cancel_chat_response");
    } catch (e) {
      console.warn("cancel failed:", e);
    }
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      send();
    }
  }

  // Шаг 10: ready = соответствующий контур доступен.
  // claude_cli ready если CLI установлен ИЛИ включён auto-fallback (тогда Qwen
  // подхватит). qwen_local ready если Qwen endpoint отвечает.
  const ready =
    brainMode === "claude_external"
      ? externalAgentOn
      : brainMode === "qwen_local"
        ? qwenReady
        : claudeReady || qwenReady; // claude_cli: ok если хоть один из контуров жив

  const inputDisabled = sending || streaming !== null || !ready;
  const placeholder = !ready
    ? brainMode === "claude_external"
      ? "Включи External Agent в Настройках для Claude (Architect)"
      : brainMode === "qwen_local"
        ? "Qwen 3 не отвечает — запусти Ollama / LM Studio (Настройки)"
        : "Установи Claude CLI или запусти Qwen 3 (Настройки)"
    : streaming
      ? "Гендир думает…"
      : "Сообщение Гендиру… (Enter — отправить, Shift+Enter — перенос)";

  return (
    <div
      style={containerStyle}
      onDrop={handleDrop}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
    >
      {isDraggingOver && (
        <div style={dropOverlayStyle}>
          📂 Отпусти — приложу файлы к следующему сообщению
        </div>
      )}
      <div style={headerStyle}>
        <h1 style={{ margin: 0, fontSize: 22 }}>💬 Гендир (CEO)</h1>
        <p style={{ margin: "4px 0 8px", color: "#666", fontSize: 13 }}>
          {brainMode === "claude_cli"
            ? "⭐ Мозг: Claude 4.7 Opus локально через CLI (основной контур)"
            : brainMode === "qwen_local"
              ? "🐉 Мозг: Qwen 3 локально (автономный/офлайн)"
              : "🧑‍💼 Мозг: Claude (Architect mode) через WS — legacy"}
        </p>

        <div style={{ display: "flex", gap: 8, marginBottom: 8 }}>
          <button
            type="button"
            onClick={() => switchBrain("claude_cli")}
            style={brainBtnStyle(brainMode === "claude_cli")}
          >
            ⭐ Claude 4.7 Opus
          </button>
          <button
            type="button"
            onClick={() => switchBrain("qwen_local")}
            style={brainBtnStyle(brainMode === "qwen_local")}
          >
            🐉 Qwen 3 (Автономный)
          </button>
        </div>

        <BrainStatusBadges
          onClaudeReady={setClaudeReady}
          onQwenReady={setQwenReady}
        />
      </div>

      <div ref={scrollRef} style={messagesStyle}>
        {messages.length === 0 && !streaming && !error && (
          <p style={{ color: "#999", textAlign: "center", marginTop: 60 }}>
            {ready
              ? "Начни разговор. Гендир знает оргструктуру и ответит со ссылками на посты."
              : "Запусти Claude CLI или Qwen 3 (Настройки → 🧠 Двухконтурный Мозг)."}
          </p>
        )}
        <div style={{ display: "flex", flexDirection: "column" }}>
          {messages.map((m) => {
            const sysVariant = isSystemMessage(m);
            if (sysVariant) {
              // Step 7 Этап 3: системное сообщение (⚡/⚠️) — отдельная плашка
              // на всю ширину, без bubble-стиля, без MessageActions.
              return (
                <div
                  key={m.id}
                  style={{
                    display: "flex",
                    flexDirection: "column",
                    alignItems: "stretch",
                    marginBottom: 6,
                  }}
                >
                  <div style={systemBubbleStyle(sysVariant)}>{m.content}</div>
                  <div style={{ ...timestampStyle, alignSelf: "flex-start" }}>
                    {formatTime(m.created_at)}
                  </div>
                </div>
              );
            }
            const isCeo = m.role === "ceo";
            return (
              <div
                key={m.id}
                className="msg-row"
                style={{
                  display: "flex",
                  flexDirection: "column",
                  alignItems: m.role === "owner" ? "flex-end" : "flex-start",
                  marginBottom: 4,
                }}
              >
                <div style={bubbleStyle(m.role)}>{m.content}</div>
                <div
                  style={{
                    ...timestampStyle,
                    alignSelf: m.role === "owner" ? "flex-end" : "flex-start",
                  }}
                >
                  {formatTime(m.created_at)}
                </div>
                {/* v1.0.13: hover-actions для всех (copy/quote) + Vault только для CEO */}
                <div
                  className="msg-actions"
                  style={{
                    display: "flex",
                    gap: 6,
                    flexWrap: "wrap",
                    marginTop: 4,
                    alignSelf: m.role === "owner" ? "flex-end" : "flex-start",
                  }}
                >
                  <MessageHoverActions content={m.content} onQuote={handleQuote} />
                  {isCeo && (
                    <MessageActions
                      onPick={(kind) => setVaultModal({ kind, content: m.content })}
                    />
                  )}
                </div>
              </div>
            );
          })}
          {streaming && (
            <div style={{ display: "flex", flexDirection: "column", alignItems: "flex-start" }}>
              <div style={bubbleStyle("ceo", true)}>
                {streaming.text || "🧠 Думает…"}
                <span style={{ opacity: 0.5, marginLeft: 4 }}>▊</span>
              </div>
              <div style={timestampStyle}>сейчас</div>
            </div>
          )}
        </div>
        {error && (
          <div
            style={{
              padding: 12,
              background: "#fee",
              border: "1px solid #c00",
              borderRadius: 4,
              fontSize: 13,
              marginTop: 12,
              whiteSpace: "pre-wrap",
            }}
          >
            {error}
          </div>
        )}
      </div>

      <AttachmentsArea
        items={attachments}
        folders={folders}
        onRemove={removeAttachment}
        onRemoveFolder={removeFolder}
      />

      <div style={inputBarStyle}>
        <AttachmentButtons
          current={attachments}
          onAdd={addAttachments}
          disabled={inputDisabled}
        />
        <textarea
          ref={textareaRef}
          style={inputStyle}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={placeholder}
          rows={1}
          disabled={inputDisabled}
        />
        {streaming ? (
          <button type="button" onClick={cancel} style={cancelBtnStyle}>
            ⏹ Прервать
          </button>
        ) : (
          <button
            type="button"
            onClick={send}
            disabled={(!input.trim() && attachments.length === 0) || inputDisabled}
            style={sendBtnStyle((!input.trim() && attachments.length === 0) || inputDisabled)}
          >
            {sending ? "…" : "Отправить"}
          </button>
        )}
      </div>

      {vaultModal && (
        <VaultSaveModal
          initialKind={vaultModal.kind}
          initialContent={vaultModal.content}
          onClose={() => setVaultModal(null)}
        />
      )}
    </div>
  );
}
