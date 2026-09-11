const FONT_SYSTEM_SANS = 'HarmonyOS Sans';
const COLOR_INK: ResourceColor = $r('app.color.remote_ink');
const COLOR_INK_BODY: ResourceColor = $r('app.color.remote_ink_body');
const COLOR_MUTED: ResourceColor = $r('app.color.remote_muted');
const COLOR_ACCENT: ResourceColor = $r('app.color.remote_accent');
const COLOR_ACCENT_SOFT: ResourceColor = $r('app.color.remote_accent_soft');
const COLOR_RULE: ResourceColor = $r('app.color.remote_rule');
const COLOR_SURFACE_CODE: ResourceColor = $r('app.color.remote_surface_code');

type MarkdownBlockKind = 'paragraph' | 'heading' | 'quote' | 'unordered-list' | 'ordered-list' | 'code' | 'rule';

interface MarkdownBlock {
  kind: MarkdownBlockKind;
  text: string;
  level: number;
  marker: string;
}

interface MarkdownSpan {
  text: string;
  bold: boolean;
  italic: boolean;
  code: boolean;
  link: boolean;
}

function markdownSpan(text: string, bold: boolean = false, italic: boolean = false,
  code: boolean = false, link: boolean = false): MarkdownSpan {
  return { text, bold, italic, code, link };
}

function markdownBlocks(markdown: string): MarkdownBlock[] {
  const blocks: MarkdownBlock[] = [];
  const lines: string[] = markdown.replace(/\r\n?/g, '\n').split('\n');
  let paragraphLines: string[] = [];
  let codeLines: string[] = [];
  let inCodeBlock: boolean = false;

  const flushParagraph = (): void => {
    if (paragraphLines.length > 0) {
      blocks.push({ kind: 'paragraph', text: paragraphLines.join('\n'), level: 0, marker: '' });
      paragraphLines = [];
    }
  };
  const flushCode = (): void => {
    if (codeLines.length > 0) {
      blocks.push({ kind: 'code', text: codeLines.join('\n'), level: 0, marker: '' });
      codeLines = [];
    }
  };

  for (let index: number = 0; index < lines.length; index++) {
    const line: string = lines[index];
    const trimmed: string = line.trim();
    if (trimmed.startsWith('```')) {
      flushParagraph();
      if (inCodeBlock) {
        flushCode();
      }
      inCodeBlock = !inCodeBlock;
      continue;
    }
    if (inCodeBlock) {
      codeLines.push(line);
      continue;
    }
    if (trimmed.length === 0) {
      flushParagraph();
      continue;
    }

    const heading: RegExpExecArray | null = /^(#{1,6})\s+(.+)$/.exec(trimmed);
    if (heading !== null) {
      flushParagraph();
      blocks.push({ kind: 'heading', text: heading[2], level: heading[1].length, marker: '' });
      continue;
    }
    const ordered: RegExpExecArray | null = /^(\d+)\.\s+(.+)$/.exec(trimmed);
    if (ordered !== null) {
      flushParagraph();
      blocks.push({ kind: 'ordered-list', text: ordered[2], level: 0, marker: `${ordered[1]}.` });
      continue;
    }
    const unordered: RegExpExecArray | null = /^[-*+]\s+(.+)$/.exec(trimmed);
    if (unordered !== null) {
      flushParagraph();
      blocks.push({ kind: 'unordered-list', text: unordered[1], level: 0, marker: '•' });
      continue;
    }
    const quote: RegExpExecArray | null = /^>\s?(.*)$/.exec(trimmed);
    if (quote !== null) {
      flushParagraph();
      blocks.push({ kind: 'quote', text: quote[1], level: 0, marker: '' });
      continue;
    }
    if (/^[-*_]{3,}$/.test(trimmed)) {
      flushParagraph();
      blocks.push({ kind: 'rule', text: '', level: 0, marker: '' });
      continue;
    }
    paragraphLines.push(line);
  }
  flushParagraph();
  if (inCodeBlock) {
    flushCode();
  }
  return blocks;
}

function inlineMarkdownSpans(value: string): MarkdownSpan[] {
  const spans: MarkdownSpan[] = [];
  let cursor: number = 0;
  while (cursor < value.length) {
    if (value.startsWith('**', cursor) || value.startsWith('__', cursor)) {
      const marker: string = value.slice(cursor, cursor + 2);
      const closing: number = value.indexOf(marker, cursor + 2);
      if (closing > cursor + 2) {
        spans.push(markdownSpan(value.slice(cursor + 2, closing), true));
        cursor = closing + 2;
        continue;
      }
    }
    if (value[cursor] === '`') {
      const closing: number = value.indexOf('`', cursor + 1);
      if (closing > cursor + 1) {
        spans.push(markdownSpan(value.slice(cursor + 1, closing), false, false, true));
        cursor = closing + 1;
        continue;
      }
    }
    if (value[cursor] === '[') {
      const labelEnd: number = value.indexOf('](', cursor + 1);
      if (labelEnd > cursor + 1) {
        const urlEnd: number = value.indexOf(')', labelEnd + 2);
        if (urlEnd > labelEnd + 2) {
          spans.push(markdownSpan(value.slice(cursor + 1, labelEnd), false, false, false, true));
          cursor = urlEnd + 1;
          continue;
        }
      }
    }
    if (value[cursor] === '*' || value[cursor] === '_') {
      const marker: string = value[cursor];
      const closing: number = value.indexOf(marker, cursor + 1);
      if (closing > cursor + 1) {
        spans.push(markdownSpan(value.slice(cursor + 1, closing), false, true));
        cursor = closing + 1;
        continue;
      }
    }

    let next: number = cursor + 1;
    while (next < value.length && value[next] !== '*' && value[next] !== '_' &&
      value[next] !== '`' && value[next] !== '[') {
      next++;
    }
    spans.push(markdownSpan(value.slice(cursor, next)));
    cursor = next;
  }
  return spans;
}

@Component
export struct MarkdownPreview {
  @Prop content: string = '';

  private headingSize(level: number): number {
    if (level === 1) {
      return 24;
    }
    if (level === 2) {
      return 20;
    }
    return 18;
  }

  @Builder
  private inlineText(value: string, fontSize: number, color: ResourceColor): void {
    Text('') {
      ForEach(inlineMarkdownSpans(value), (span: MarkdownSpan, index: number) => {
        Span(span.text)
          .fontFamily(span.code ? 'monospace' : FONT_SYSTEM_SANS)
          .fontSize(span.code ? Math.max(14, fontSize - 1) : fontSize)
          // Remote Markdown never opens URLs. Keep link labels visually identical to text so the
          // preview does not promise an action that this isolated reader intentionally cannot perform.
          .fontColor(color)
          .fontWeight(span.bold ? FontWeight.Bold : FontWeight.Regular)
          .fontStyle(span.italic ? FontStyle.Italic : FontStyle.Normal)
          .backgroundColor(span.code ? COLOR_SURFACE_CODE : Color.Transparent)
      }, (span: MarkdownSpan, index: number) => `${index}-${span.text}`)
    }
    .width('100%')
  }

  @Builder
  private renderBlock(block: MarkdownBlock): void {
    if (block.kind === 'heading') {
      this.inlineText(block.text, this.headingSize(block.level), COLOR_INK)
    } else if (block.kind === 'code') {
      Text(block.text)
        .fontFamily('monospace')
        .fontSize(14)
        .fontColor(COLOR_INK_BODY)
        .width('100%')
        .padding(12)
        .backgroundColor(COLOR_SURFACE_CODE)
        .borderRadius(2)
    } else if (block.kind === 'rule') {
      Divider()
        .color(COLOR_RULE)
        .strokeWidth(1)
        .margin({ top: 4, bottom: 4 })
    } else if (block.kind === 'quote') {
      Row() {
        Divider().vertical(true).color(COLOR_ACCENT).strokeWidth(4).height('100%')
        this.inlineText(block.text, 16, COLOR_MUTED)
      }
      .alignItems(VerticalAlign.Top)
      .width('100%')
      .padding({ left: 10, right: 4, top: 4, bottom: 4 })
      .backgroundColor(COLOR_ACCENT_SOFT)
      .borderRadius(0)
    } else if (block.kind === 'unordered-list' || block.kind === 'ordered-list') {
      Row({ space: 8 }) {
        Text(block.marker)
          .fontFamily(FONT_SYSTEM_SANS)
          .fontSize(16)
          .fontColor(COLOR_ACCENT)
          .fontWeight(FontWeight.Medium)
          .width(24)
          .textAlign(TextAlign.End)
        this.inlineText(block.text, 16, COLOR_INK_BODY)
      }
      .alignItems(VerticalAlign.Top)
      .width('100%')
    } else {
      this.inlineText(block.text, 16, COLOR_INK_BODY)
    }
  }

  build() {
    Column({ space: 10 }) {
      ForEach(markdownBlocks(this.content), (block: MarkdownBlock, index: number) => {
        this.renderBlock(block)
      }, (block: MarkdownBlock, index: number) => `${index}-${block.kind}-${block.text}`)
    }
    .alignItems(HorizontalAlign.Start)
    .width('100%')
  }
}
