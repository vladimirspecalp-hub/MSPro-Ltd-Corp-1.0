import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import HermesStatusBadge from "../chat/HermesStatusBadge";
import MessageActions from "../chat/MessageActions";
import VaultSaveModal, { type VaultKind } from "../chat/VaultSaveModal";

type BrainMode = "hermes" | "claude_external";

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
  maxHeight: 120,
  boxSizing: "border-box",
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
  const [hermesReady, setHermesReady] = useState(false);
  const [brainMode, setBrainMode] = useState<BrainMode>("hermes");
  const [externalAgentOn, setExternalAgentOn] = useState(false);
  // Step 7 Этап 2: модалка сохранения сообщения в файловую память Vault
  const [vaultModal, setVaultModal] = useState<
    { kind: VaultKind; content: string } | null
  >(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const streamingRef = useRef<StreamingState | null>(null);

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
        if (s.brain_mode === "claude_external" || s.brain_mode === "hermes") {
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
    // Same handler also fires when the Rust send_chat_message resolves (the
    // emit happens just before the command returns). Belt-and-braces: if the
    // event misses, the optimistic-flow Promise resolution will still update.
    return () => {
      offStartP.then((f) => f());
      offChunkP.then((f) => f());
      offDoneP.then((f) => f());
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
    if (!text || sending) return;
    setSending(true);
    setError(null);
    setInput("");
    // Optimistic owner bubble — we'll dedupe when send_chat_message returns.
    const optimisticUserId = `tmp-${Date.now()}`;
    setMessages((prev) => [
      ...prev,
      {
        id: optimisticUserId,
        role: "owner",
        content: text,
        created_at: new Date().toISOString(),
      },
    ]);
    try {
      const turn = await invoke<ChatTurn>("send_chat_message", { content: text });
      // Replace optimistic owner row with the persisted one (authoritative id+timestamp).
      setMessages((prev) =>
        prev.map((m) => (m.id === optimisticUserId ? turn.user : m))
      );
      // ceo row is added by the ceo-done listener.
    } catch (e) {
      setError(String(e));
      setMessages((prev) => prev.filter((m) => m.id !== optimisticUserId));
      setInput(text);
    } finally {
      setSending(false);
    }
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

  const ready =
    brainMode === "claude_external" ? externalAgentOn : hermesReady;
  const inputDisabled = sending || streaming !== null || !ready;
  const placeholder = !ready
    ? brainMode === "claude_external"
      ? "Включи External Agent в Настройках для Claude (Architect)"
      : "Сначала установи Hermes ⬆️"
    : streaming
      ? brainMode === "claude_external"
        ? "Жду Claude (Architect)…"
        : "Гендир думает…"
      : "Сообщение Гендиру… (Enter — отправить, Shift+Enter — перенос)";

  return (
    <div style={containerStyle}>
      <div style={headerStyle}>
        <h1 style={{ margin: 0, fontSize: 22 }}>💬 Гендир (CEO)</h1>
        <p style={{ margin: "4px 0 8px", color: "#666", fontSize: 13 }}>
          {brainMode === "claude_external"
            ? "🧑‍💼 Мозг: Claude (Architect mode) — отвечает живой эксперт через External Agent WS"
            : "🤖 Мозг: Hermes на WSL2 → DeepSeek-Reasoner"}
        </p>

        <div style={{ display: "flex", gap: 8, marginBottom: 8 }}>
          <button
            type="button"
            onClick={() => switchBrain("hermes")}
            style={brainBtnStyle(brainMode === "hermes")}
          >
            🤖 Hermes (DeepSeek)
          </button>
          <button
            type="button"
            onClick={() => switchBrain("claude_external")}
            style={brainBtnStyle(brainMode === "claude_external")}
          >
            🧑‍💼 Claude (Architect)
          </button>
        </div>

        {brainMode === "hermes" && (
          <HermesStatusBadge
            onStatusChange={(s) => setHermesReady(s?.kind === "available")}
          />
        )}
        {brainMode === "claude_external" && (
          <div
            style={{
              padding: "8px 14px",
              borderRadius: 6,
              fontSize: 12,
              border: "1px solid",
              background: externalAgentOn ? "#e8f5e9" : "#fff8e1",
              borderColor: externalAgentOn ? "#4caf50" : "#ffa000",
              color: externalAgentOn ? "#1b5e20" : "#e65100",
              display: "flex",
              alignItems: "center",
              gap: 10,
            }}
          >
            {externalAgentOn ? "🟢" : "🟡"}
            <span style={{ flex: 1 }}>
              {externalAgentOn
                ? "External Agent gateway включён — Claude (Architect) может подключиться и отвечать"
                : "Включи External Agent в Настройках — Claude подключается через тот же WS"}
            </span>
          </div>
        )}
      </div>

      <div ref={scrollRef} style={messagesStyle}>
        {messages.length === 0 && !streaming && !error && (
          <p style={{ color: "#999", textAlign: "center", marginTop: 60 }}>
            {hermesReady
              ? "Начни разговор. Гендир знает оргструктуру и ответит со ссылками на посты."
              : "Установи Hermes по подсказке выше — Гендир оживёт."}
          </p>
        )}
        <div style={{ display: "flex", flexDirection: "column" }}>
          {messages.map((m) => {
            const isCeo = m.role === "ceo";
            return (
              <div
                key={m.id}
                className={isCeo ? "msg-row" : undefined}
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
                {isCeo && (
                  <MessageActions
                    onPick={(kind) => setVaultModal({ kind, content: m.content })}
                  />
                )}
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

      <div style={inputBarStyle}>
        <textarea
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
            disabled={!input.trim() || inputDisabled}
            style={sendBtnStyle(!input.trim() || inputDisabled)}
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
