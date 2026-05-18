import { memo } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import rehypeHighlight from 'rehype-highlight';

// Cowork-DS prose styling lives in chat.css; we only wire up plugins here.
interface MarkdownRendererProps {
  text: string;
}

export const MarkdownRenderer = memo(function MarkdownRenderer({ text }: MarkdownRendererProps) {
  if (!text) return null;
  return (
    <div className="cw-md">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[[rehypeHighlight, { detect: true, ignoreMissing: true }]]}
        components={{
          a: ({ href, children, ...rest }) => (
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
