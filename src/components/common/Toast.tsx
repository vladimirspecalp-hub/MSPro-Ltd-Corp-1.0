// Минимальная toast-система: один Provider в корне, useToast() хук,
// fixed bottom-right, auto-dismiss 3 сек. Без внешних зависимостей.

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { CheckCircle2, XCircle, Info, X } from "lucide-react";

export type ToastKind = "success" | "error" | "info";

interface ToastPayload {
  kind?: ToastKind;
  text: string;
  ttlMs?: number; // override длительности (по умолчанию 3000)
}

interface ToastState extends ToastPayload {
  id: number;
}

interface ToastContextValue {
  toast: (p: ToastPayload) => void;
}

const ToastContext = createContext<ToastContextValue | null>(null);

export function useToast(): ToastContextValue {
  const ctx = useContext(ToastContext);
  if (!ctx) {
    throw new Error("useToast must be used inside <ToastProvider>");
  }
  return ctx;
}

const COLORS: Record<ToastKind, { bg: string; fg: string; border: string }> = {
  success: { bg: "#e8f5e9", fg: "#1b5e20", border: "#4caf50" },
  error: { bg: "#ffebee", fg: "#b71c1c", border: "#c62828" },
  info: { bg: "#e3f2fd", fg: "#0d47a1", border: "#1976d2" },
};

const ICONS: Record<ToastKind, typeof CheckCircle2> = {
  success: CheckCircle2,
  error: XCircle,
  info: Info,
};

export function ToastProvider({ children }: { children: ReactNode }) {
  const [current, setCurrent] = useState<ToastState | null>(null);
  const counterRef = useRef(0);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const dismiss = useCallback(() => {
    setCurrent(null);
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  const toast = useCallback((p: ToastPayload) => {
    counterRef.current += 1;
    const next: ToastState = {
      id: counterRef.current,
      kind: p.kind ?? "info",
      text: p.text,
      ttlMs: p.ttlMs ?? 3000,
    };
    setCurrent(next);
    if (timerRef.current) clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => setCurrent(null), next.ttlMs!);
  }, []);

  useEffect(() => {
    return () => {
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, []);

  return (
    <ToastContext.Provider value={{ toast }}>
      {children}
      {current && <ToastView state={current} onDismiss={dismiss} />}
    </ToastContext.Provider>
  );
}

function ToastView({ state, onDismiss }: { state: ToastState; onDismiss: () => void }) {
  const kind = state.kind ?? "info";
  const c = COLORS[kind];
  const Icon = ICONS[kind];
  return (
    <div
      role="status"
      aria-live="polite"
      style={{
        position: "fixed",
        bottom: 24,
        right: 24,
        zIndex: 2000,
        maxWidth: 420,
        padding: "12px 14px",
        background: c.bg,
        color: c.fg,
        border: `1px solid ${c.border}`,
        borderRadius: 8,
        boxShadow: "0 4px 16px rgba(0,0,0,0.15)",
        display: "flex",
        alignItems: "flex-start",
        gap: 10,
        fontSize: 13,
        lineHeight: 1.4,
        animation: "toast-slide-in 0.18s ease-out",
      }}
    >
      <Icon size={18} style={{ flexShrink: 0, marginTop: 1 }} />
      <div style={{ flex: 1, whiteSpace: "pre-wrap", wordBreak: "break-word" }}>{state.text}</div>
      <button
        type="button"
        onClick={onDismiss}
        aria-label="Закрыть"
        style={{
          background: "transparent",
          border: "none",
          color: c.fg,
          cursor: "pointer",
          padding: 0,
          marginTop: 1,
          display: "flex",
          alignItems: "center",
        }}
      >
        <X size={16} />
      </button>
    </div>
  );
}
