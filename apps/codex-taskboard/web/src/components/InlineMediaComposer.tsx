import {
  forwardRef,
  useImperativeHandle,
  useLayoutEffect,
  useRef,
  useState,
  type ClipboardEvent,
  type KeyboardEventHandler,
} from "react";
import type { Attachment } from "../types";
import { attachmentContentUrl } from "../api";
import { clipboardImages, fileKey, MAX_ATTACHMENT_SIZE } from "./PendingAttachments";
import { LinearIcon } from "./LinearIcon";

interface InlineTextSegment {
  id: string;
  type: "text";
  text: string;
}

interface InlineImageSegment {
  id: string;
  type: "pending-image";
  token: string;
  file: File;
}

export type InlineMediaSegment = InlineTextSegment | InlineImageSegment;
export type PendingInlineImage = InlineImageSegment;

export interface InlineMediaComposerHandle {
  focus: () => void;
}

interface InlineMediaComposerProps {
  segments: InlineMediaSegment[];
  placeholder: string;
  ariaLabel: string;
  disabled?: boolean;
  className?: string;
  onChange: (segments: InlineMediaSegment[]) => void;
  onError: (message: string | null) => void;
  onKeyDown?: KeyboardEventHandler<HTMLTextAreaElement>;
}

let segmentSequence = 0;

function segmentId(prefix: string): string {
  segmentSequence += 1;
  return `${prefix}-${Date.now().toString(36)}-${segmentSequence.toString(36)}`;
}

function textSegment(text = ""): InlineTextSegment {
  return { id: segmentId("text"), type: "text", text };
}

function imageSegment(file: File): InlineImageSegment {
  const id = segmentId("image");
  return {
    id,
    type: "pending-image",
    token: `<!--taskboard-inline-image:${id}-->`,
    file,
  };
}

export function createInlineMediaSegments(text = ""): InlineMediaSegment[] {
  return [textSegment(text)];
}

export function inlineMediaImages(segments: InlineMediaSegment[]): PendingInlineImage[] {
  return segments.filter((segment): segment is PendingInlineImage => segment.type === "pending-image");
}

export function inlineMediaText(segments: InlineMediaSegment[]): string {
  return segments.flatMap((segment) => segment.type === "text" ? [segment.text] : []).join("");
}

export function serializeInlineMedia(segments: InlineMediaSegment[]): string {
  return segments.map((segment) => (
    segment.type === "text" ? segment.text : `\n\n${segment.token}\n\n`
  )).join("");
}

export function resolveInlineMediaMarkdown(
  value: string,
  images: PendingInlineImage[],
  attachments: Attachment[],
): string {
  return images.reduce((markdown, image, index) => {
    const attachment = attachments[index];
    if (!attachment) return markdown;
    const alt = image.file.name.replace(/[\\[\]]/g, "\\$&");
    return markdown.replace(
      image.token,
      `![${alt}](${attachmentContentUrl(attachment)})`,
    );
  }, value);
}

function normalizeSegments(segments: InlineMediaSegment[]): InlineMediaSegment[] {
  const normalized: InlineMediaSegment[] = [];
  for (const segment of segments) {
    const previous = normalized.at(-1);
    if (segment.type === "text" && previous?.type === "text") {
      normalized[normalized.length - 1] = {
        ...previous,
        text: previous.text + segment.text,
      };
    } else {
      normalized.push(segment);
    }
  }
  if (normalized.length === 0) return [textSegment()];
  if (normalized[0].type !== "text") normalized.unshift(textSegment());
  if (normalized.at(-1)?.type !== "text") normalized.push(textSegment());
  return normalized;
}

function resizeTextarea(element: HTMLTextAreaElement | null) {
  if (!element) return;
  element.style.height = "0px";
  element.style.height = `${element.scrollHeight}px`;
}

function PendingImageBlock({
  segment,
  disabled,
  onRemove,
}: {
  segment: PendingInlineImage;
  disabled: boolean;
  onRemove: () => void;
}) {
  const [previewUrl, setPreviewUrl] = useState("");

  useLayoutEffect(() => {
    const url = URL.createObjectURL(segment.file);
    setPreviewUrl(url);
    return () => URL.revokeObjectURL(url);
  }, [segment.file]);

  return (
    <figure className="inline-media-image">
      {previewUrl && <img src={previewUrl} alt={segment.file.name} />}
      <button
        type="button"
        disabled={disabled}
        aria-label={`移除 ${segment.file.name}`}
        onClick={onRemove}
      >
        <LinearIcon name="close" />
      </button>
    </figure>
  );
}

export const InlineMediaComposer = forwardRef<InlineMediaComposerHandle, InlineMediaComposerProps>(
  function InlineMediaComposer({
    segments,
    placeholder,
    ariaLabel,
    disabled = false,
    className = "",
    onChange,
    onError,
    onKeyDown,
  }, ref) {
    const textareas = useRef(new Map<string, HTMLTextAreaElement>());
    const pendingFocus = useRef<{ id: string; offset: number } | null>(null);

    useLayoutEffect(() => {
      for (const element of textareas.current.values()) resizeTextarea(element);
      const focus = pendingFocus.current;
      if (!focus) return;
      const target = textareas.current.get(focus.id);
      if (!target) return;
      target.focus();
      target.setSelectionRange(focus.offset, focus.offset);
      pendingFocus.current = null;
    }, [segments]);

    useImperativeHandle(ref, () => ({
      focus() {
        const firstText = segments.find((segment) => segment.type === "text");
        if (firstText) textareas.current.get(firstText.id)?.focus();
      },
    }), [segments]);

    function changeText(id: string, text: string) {
      onChange(segments.map((segment) => (
        segment.id === id && segment.type === "text" ? { ...segment, text } : segment
      )));
    }

    function pasteImages(
      event: ClipboardEvent<HTMLTextAreaElement>,
      segment: InlineTextSegment,
    ) {
      const clipboardFiles = clipboardImages(event.clipboardData);
      if (clipboardFiles.length === 0) return;
      event.preventDefault();

      const oversized = clipboardFiles.find((file) => file.size > MAX_ATTACHMENT_SIZE);
      if (oversized) {
        onError(`“${oversized.name}” 超过 25 MB，无法上传。`);
        return;
      }

      const existing = new Set(inlineMediaImages(segments).map((image) => fileKey(image.file)));
      const images = clipboardFiles.filter((file) => {
        const key = fileKey(file);
        if (existing.has(key)) return false;
        existing.add(key);
        return true;
      });
      if (images.length === 0) return;
      onError(null);

      const textarea = event.currentTarget;
      const start = textarea.selectionStart;
      const end = textarea.selectionEnd;
      const before = { ...segment, text: segment.text.slice(0, start) };
      const after = textSegment(segment.text.slice(end));
      const insertion = images.map(imageSegment);
      const next = segments.flatMap((candidate) => (
        candidate.id === segment.id ? [before, ...insertion, after] : [candidate]
      ));
      pendingFocus.current = { id: after.id, offset: 0 };
      onChange(next);
    }

    function removeImage(id: string) {
      onChange(normalizeSegments(segments.filter((segment) => segment.id !== id)));
    }

    const isEmpty = segments.every((segment) => (
      segment.type === "text" ? segment.text.length === 0 : false
    ));

    return (
      <div className={`inline-media-composer ${className}`.trim()} aria-label={ariaLabel}>
        {segments.map((segment, index) => (
          segment.type === "text" ? (
            <textarea
              key={segment.id}
              ref={(element) => {
                if (element) textareas.current.set(segment.id, element);
                else textareas.current.delete(segment.id);
              }}
              value={segment.text}
              rows={1}
              disabled={disabled}
              aria-label={index === 0 ? ariaLabel : `${ariaLabel}续写`}
              placeholder={isEmpty && index === 0 ? placeholder : undefined}
              onChange={(event) => {
                changeText(segment.id, event.target.value);
                resizeTextarea(event.currentTarget);
              }}
              onPaste={(event) => pasteImages(event, segment)}
              onKeyDown={onKeyDown}
            />
          ) : (
            <PendingImageBlock
              key={segment.id}
              segment={segment}
              disabled={disabled}
              onRemove={() => removeImage(segment.id)}
            />
          )
        ))}
      </div>
    );
  },
);
