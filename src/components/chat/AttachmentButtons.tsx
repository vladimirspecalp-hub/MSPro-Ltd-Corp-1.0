// Кнопки 📎/📁 для прикрепления файлов / папок к сообщению Гендира.
// Прячут под капотом <input type="file"> + webkitdirectory.

import { useRef } from "react";
import { Paperclip, FolderUp } from "lucide-react";
import {
  readSingleFile,
  readFolderFiles,
  validateAdd,
  type AttachmentItem,
  type FolderBundle,
} from "../../lib/attachments";
import { useToast } from "../common/Toast";

interface Props {
  current: AttachmentItem[];
  onAdd: (items: AttachmentItem[], folderBundle?: FolderBundle) => void;
  disabled?: boolean;
}

export default function AttachmentButtons({ current, onAdd, disabled }: Props) {
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const folderInputRef = useRef<HTMLInputElement | null>(null);
  const { toast } = useToast();

  async function handleFiles(fl: FileList | null) {
    if (!fl || fl.length === 0) return;
    const items: AttachmentItem[] = [];
    for (let i = 0; i < fl.length; i++) {
      items.push(await readSingleFile(fl[i]));
    }
    const v = validateAdd(current, items);
    if (!v.ok) toast({ kind: "error", text: v.message ?? "Лимит превышен" });
    onAdd(items);
  }

  async function handleFolder(fl: FileList | null) {
    if (!fl || fl.length === 0) return;
    const { items, folderBundle, skipped } = await readFolderFiles(fl);
    if (skipped > 0) {
      toast({
        kind: "info",
        text: `Пропущено ${skipped} файлов: бинарные форматы или сегменты в игнор-листе (node_modules / .git / target / …).`,
      });
    }
    if (items.length === 0) {
      toast({ kind: "info", text: "В папке нет подходящих текстовых файлов." });
      return;
    }
    const v = validateAdd(current, items);
    if (!v.ok) toast({ kind: "error", text: v.message ?? "Лимит превышен" });
    onAdd(items, folderBundle ?? undefined);
  }

  return (
    <>
      <input
        ref={fileInputRef}
        type="file"
        multiple
        style={{ display: "none" }}
        onChange={(e) => {
          handleFiles(e.target.files);
          // reset value так чтобы выбрать тот же файл повторно можно было
          if (fileInputRef.current) fileInputRef.current.value = "";
        }}
      />
      <input
        ref={folderInputRef}
        type="file"
        // webkitdirectory не в стандартных HTMLAttributes
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        {...({ webkitdirectory: "", directory: "" } as any)}
        multiple
        style={{ display: "none" }}
        onChange={(e) => {
          handleFolder(e.target.files);
          if (folderInputRef.current) folderInputRef.current.value = "";
        }}
      />

      <button
        type="button"
        onClick={() => fileInputRef.current?.click()}
        disabled={disabled}
        title="Прикрепить файлы"
        style={iconBtnStyle(disabled)}
      >
        <Paperclip size={16} />
      </button>
      <button
        type="button"
        onClick={() => folderInputRef.current?.click()}
        disabled={disabled}
        title="Прикрепить папку"
        style={iconBtnStyle(disabled)}
      >
        <FolderUp size={16} />
      </button>
    </>
  );
}

const iconBtnStyle = (disabled?: boolean): React.CSSProperties => ({
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  width: 36,
  height: 36,
  padding: 0,
  background: "#fff",
  color: disabled ? "#bbb" : "#1a1a1a",
  border: "1px solid #ccc",
  borderRadius: 6,
  cursor: disabled ? "not-allowed" : "pointer",
  boxShadow: "none",
});
