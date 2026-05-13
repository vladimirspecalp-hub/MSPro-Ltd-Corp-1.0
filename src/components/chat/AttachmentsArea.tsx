// Чипы прикреплённых файлов/папок над input bar.
// Показывает имя, размер, для папок — количество файлов внутри.
// Кнопка ✕ удаляет элемент (или всю папку bundle сразу).

import { FileText, Folder, X, AlertTriangle } from "lucide-react";
import {
  formatBytes,
  totalActiveSize,
  type AttachmentItem,
  type FolderBundle,
} from "../../lib/attachments";

interface Props {
  items: AttachmentItem[];
  folders: FolderBundle[];
  onRemove: (id: string) => void;
  onRemoveFolder: (rootName: string) => void;
}

export default function AttachmentsArea({
  items,
  folders,
  onRemove,
  onRemoveFolder,
}: Props) {
  if (items.length === 0) return null;

  // Group items: papka-bundle children рендерятся внутри bundle-чипа, отдельные file — по одному.
  const folderItemIds = new Set<string>(folders.flatMap((b) => b.itemIds));
  const standaloneItems = items.filter((i) => !folderItemIds.has(i.id));
  const totalSize = totalActiveSize(items);

  return (
    <div style={containerStyle}>
      <div style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
        {/* Папки */}
        {folders.map((bundle) => {
          const inFolder = items.filter((i) => bundle.itemIds.includes(i.id));
          const size = inFolder.reduce((s, i) => s + i.sizeBytes, 0);
          return (
            <FolderChip
              key={bundle.rootName}
              name={bundle.rootName}
              fileCount={inFolder.length}
              sizeBytes={size}
              onRemove={() => onRemoveFolder(bundle.rootName)}
            />
          );
        })}

        {/* Одиночные файлы */}
        {standaloneItems.map((i) =>
          i.kind === "unsupported" ? (
            <UnsupportedChip
              key={i.id}
              filename={i.filename}
              reason={i.unsupportedReason ?? "не поддерживается"}
              onRemove={() => onRemove(i.id)}
            />
          ) : (
            <FileChip
              key={i.id}
              filename={i.filename}
              sizeBytes={i.sizeBytes}
              truncated={i.truncated}
              onRemove={() => onRemove(i.id)}
            />
          ),
        )}
      </div>
      <div style={summaryStyle}>
        Прикреплено: {items.length} {pluralize(items.length, "файл", "файла", "файлов")} •{" "}
        {formatBytes(totalSize)} (отправится с сообщением)
      </div>
    </div>
  );
}

function FileChip({
  filename,
  sizeBytes,
  truncated,
  onRemove,
}: {
  filename: string;
  sizeBytes: number;
  truncated: boolean;
  onRemove: () => void;
}) {
  return (
    <div style={chipStyle("#e3f2fd", "#0d47a1")}>
      <FileText size={14} />
      <span style={{ maxWidth: 220, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
        {filename}
      </span>
      <span style={{ opacity: 0.7, fontSize: 11 }}>
        {formatBytes(sizeBytes)}
        {truncated ? " · обрезано" : ""}
      </span>
      <RemoveBtn onClick={onRemove} accent="#0d47a1" />
    </div>
  );
}

function FolderChip({
  name,
  fileCount,
  sizeBytes,
  onRemove,
}: {
  name: string;
  fileCount: number;
  sizeBytes: number;
  onRemove: () => void;
}) {
  return (
    <div style={chipStyle("#fff3e0", "#e65100")}>
      <Folder size={14} />
      <span style={{ maxWidth: 220, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
        {name}/
      </span>
      <span style={{ opacity: 0.7, fontSize: 11 }}>
        {fileCount} {pluralize(fileCount, "файл", "файла", "файлов")} · {formatBytes(sizeBytes)}
      </span>
      <RemoveBtn onClick={onRemove} accent="#e65100" />
    </div>
  );
}

function UnsupportedChip({
  filename,
  reason,
  onRemove,
}: {
  filename: string;
  reason: string;
  onRemove: () => void;
}) {
  return (
    <div style={chipStyle("#ffebee", "#b71c1c")} title={reason}>
      <AlertTriangle size={14} />
      <span style={{ maxWidth: 200, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
        {filename}
      </span>
      <span style={{ opacity: 0.7, fontSize: 11 }}>не отправится</span>
      <RemoveBtn onClick={onRemove} accent="#b71c1c" />
    </div>
  );
}

function RemoveBtn({ onClick, accent }: { onClick: () => void; accent: string }) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label="Убрать"
      style={{
        background: "transparent",
        border: "none",
        color: accent,
        cursor: "pointer",
        padding: 0,
        marginLeft: 2,
        display: "inline-flex",
        alignItems: "center",
        boxShadow: "none",
      }}
    >
      <X size={14} />
    </button>
  );
}

function pluralize(n: number, one: string, few: string, many: string): string {
  const mod10 = n % 10;
  const mod100 = n % 100;
  if (mod10 === 1 && mod100 !== 11) return one;
  if (mod10 >= 2 && mod10 <= 4 && (mod100 < 12 || mod100 > 14)) return few;
  return many;
}

const containerStyle: React.CSSProperties = {
  padding: "8px 32px",
  background: "#fafafa",
  borderTop: "1px solid #eee",
  flexShrink: 0,
};

const summaryStyle: React.CSSProperties = {
  fontSize: 11,
  color: "#888",
  marginTop: 6,
};

const chipStyle = (bg: string, fg: string): React.CSSProperties => ({
  display: "inline-flex",
  alignItems: "center",
  gap: 6,
  padding: "4px 8px 4px 10px",
  background: bg,
  color: fg,
  border: `1px solid ${fg}30`,
  borderRadius: 14,
  fontSize: 12,
  fontWeight: 500,
});
