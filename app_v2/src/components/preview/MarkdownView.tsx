import { MarkdownRenderer } from '../chat/MarkdownRenderer';

interface Props { content: string }

// Markdown files render with the exact same plugin config the chat uses.
export function MarkdownView({ content }: Props) {
  return <div className="cw-preview-doc"><MarkdownRenderer text={content} /></div>;
}
