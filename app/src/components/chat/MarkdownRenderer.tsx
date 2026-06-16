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
//
// We scroll the message-list container directly rather than scrollIntoView:
// scrollIntoView walks every scrollable ancestor up to the page root, so a tall
// footnoted answer pushes the whole page up and parks the list above the
// viewport (the same break 5c647a8 fixed for session entry). Center the target
// in the scroller by hand instead.
function scrollToAnchor(e: MouseEvent<HTMLAnchorElement>, href: string) {
  e.preventDefault();
  const target = document.getElementById(decodeURIComponent(href.slice(1)));
  const scroller = target?.closest<HTMLElement>('.cw-messages-scroll');
  if (!target || !scroller) return;
  const offset =
    target.getBoundingClientRect().top -
    scroller.getBoundingClientRect().top +
    scroller.scrollTop;
  const top = offset - (scroller.clientHeight - target.offsetHeight) / 2;
  scroller.scrollTo({ top, behavior: 'smooth' });
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
