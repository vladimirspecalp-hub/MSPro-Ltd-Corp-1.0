// Hover-кнопки на всех сообщениях чата (owner + ceo): копировать в буфер,
// цитировать в input bar. Появляются через CSS :hover на .msg-row (App.css).

import { Copy, Quote } from "lucide-react";
import { useToast } from "../common/Toast";

interface Props {
  content: string;
  onQuote: (text: string) => void;
}

export default function MessageHoverActions({ content, onQuote }: Props) {
  const { toast } = useToast();

  async function copy() {
    try {
      await navigator.clipboard.writeText(content);
      toast({ kind: "success", text: "Скопировано в буфер обмена" });
    } catch (e) {
      // Fallback на старый API (если WebView2 откажет)
      try {
        const ta = document.createElement("textarea");
        ta.value = content;
        ta.style.position = "fixed";
        ta.style.opacity = "0";
        document.body.appendChild(ta);
        ta.select();
        document.execCommand("copy");
        document.body.removeChild(ta);
        toast({ kind: "success", text: "Скопировано в буфер обмена" });
      } catch {
        toast({ kind: "error", text: `Не удалось скопировать: ${String(e)}` });
      }
    }
  }

  function quote() {
    onQuote(content);
  }

  return (
    <div
      style={{
        display: "inline-flex",
        gap: 6,
        alignItems: "center",
      }}
      aria-label="Действия с сообщением"
    >
      <ActionButton
        icon={<Copy size={13} />}
        label="Копировать"
        onClick={copy}
        accent="#455a64"
      />
      <ActionButton
        icon={<Quote size={13} />}
        label="Цитировать"
        onClick={quote}
        accent="#37474f"
      />
    </div>
  );
}

function ActionButton({
  icon,
  label,
  onClick,
  accent,
}: {
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
  accent: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={label}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 4,
        padding: "3px 8px",
        background: "#fff",
        color: accent,
        border: `1px solid ${accent}33`,
        borderRadius: 10,
        cursor: "pointer",
        fontSize: 11,
        fontWeight: 500,
        boxShadow: "0 1px 2px rgba(0,0,0,0.04)",
        transition: "background 0.12s",
      }}
      onMouseEnter={(e) => {
        (e.currentTarget as HTMLButtonElement).style.background = accent + "08";
      }}
      onMouseLeave={(e) => {
        (e.currentTarget as HTMLButtonElement).style.background = "#fff";
      }}
    >
      {icon}
      <span>{label}</span>
    </button>
  );
}
