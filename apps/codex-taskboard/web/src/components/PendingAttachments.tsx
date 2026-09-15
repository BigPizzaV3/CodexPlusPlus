import { LinearIcon } from "./LinearIcon";

export const MAX_ATTACHMENT_SIZE = 25 * 1024 * 1024;

export function fileKey(file: File): string {
  return `${file.name}:${file.size}:${file.lastModified}`;
}

export function clipboardImages(data: DataTransfer): File[] {
  const images = Array.from(data.items)
    .filter((item) => item.kind === "file" && item.type.startsWith("image/"))
    .flatMap((item) => {
      const file = item.getAsFile();
      return file ? [file] : [];
    });
  return images.length > 0
    ? images
    : Array.from(data.files).filter((file) => file.type.startsWith("image/"));
}

function fileSize(value: number): string {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(value < 10 * 1024 ? 1 : 0)} KB`;
  return `${(value / (1024 * 1024)).toFixed(value < 10 * 1024 * 1024 ? 1 : 0)} MB`;
}

interface PendingAttachmentsProps {
  files: File[];
  disabled: boolean;
  uploadLabel: string;
  ariaLabel: string;
  className?: string;
  onRemove: (index: number) => void;
}

export function PendingAttachments({
  files,
  disabled,
  uploadLabel,
  ariaLabel,
  className = "",
  onRemove,
}: PendingAttachmentsProps) {
  if (files.length === 0) return null;

  return (
    <div className={`pending-attachments ${className}`.trim()}>
      <ul className="composer-attachment-list" aria-label={ariaLabel}>
        {files.map((file, index) => (
          <li key={fileKey(file)}>
            <span className="composer-attachment-file-icon" aria-hidden="true"><LinearIcon name="file" /></span>
            <span className="composer-attachment-copy"><strong>{file.name}</strong><span>{fileSize(file.size)} · {uploadLabel}</span></span>
            <button type="button" disabled={disabled} aria-label={`移除 ${file.name}`} onClick={() => onRemove(index)}>
              <LinearIcon name="close" />
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
