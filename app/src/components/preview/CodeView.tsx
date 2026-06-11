import { MarkdownRenderer } from '../chat/MarkdownRenderer';

interface Props { content: string; lang: string }

// Choose a fence longer than any backtick run in the content so code that
// itself contains ``` does not break out of the code block.
export function fenceFor(content: string): string {
  const runs = content.match(/`+/g) ?? [];
  const longest = runs.reduce((m, r) => Math.max(m, r.length), 0);
  return '`'.repeat(Math.max(3, longest + 1));
}

export function CodeView({ content, lang }: Props) {
  // The fence isolates the file body from GFM parsing/link-rewriting — content
  // inside a code block is not interpreted as markdown, so autolinking and the
  // target="_blank" anchor override never fire on code.
  const fence = fenceFor(content);
  const md = `${fence}${lang}\n${content}\n${fence}`;
  return <div className="cw-preview-doc cw-preview-code"><MarkdownRenderer text={md} /></div>;
}
