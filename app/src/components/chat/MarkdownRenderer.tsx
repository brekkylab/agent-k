import { memo, type MouseEvent } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import rehypeHighlight from 'rehype-highlight';

// Cowork-DS prose styling lives in chat.css; we only wire up plugins here.
interface MarkdownRendererProps {
  text: string;
}

// In-page anchors (GFM footnote markers / back-refs): scroll to the target
// within the message list instead of fragment navigation, which would yank the
// layout and leave a sticky `#…` in the URL.
function scrollToAnchor(e: MouseEvent<HTMLAnchorElement>, href: string) {
  e.preventDefault();
  const target = document.getElementById(decodeURIComponent(href.slice(1)));
  target?.scrollIntoView({ behavior: 'smooth', block: 'center' });
}

export const MarkdownRenderer = memo(function MarkdownRenderer({ text }: MarkdownRendererProps) {
  if (!text) return null;
  return (
    <div className="cw-md">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[[rehypeHighlight, { detect: true, ignoreMissing: true }]]}
        components={{
          a: ({ href, children, ...rest }) =>
            // In-page anchor (footnote): scroll within the message list, no new tab.
            href?.startsWith('#') ? (
              <a href={href} onClick={(e) => scrollToAnchor(e, href)} {...rest}>{children}</a>
            ) : (
              <a href={href} target="_blank" rel="noreferrer noopener" {...rest}>{children}</a>
            ),
          code: ({ className, children, ...rest }) => {
            const isBlock = className?.includes('language-');
            if (isBlock) return <code className={className} {...rest}>{children}</code>;
            return <code className="cw-md-inline-code" {...rest}>{children}</code>;
          },
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
});
