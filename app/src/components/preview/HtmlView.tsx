interface Props { objectUrl: string; title: string }

// Live render of (possibly agent-generated, possibly untrusted) HTML.
// allow-scripts runs JS; allow-same-origin is deliberately omitted so the
// frame cannot reach our origin's cookies/JWT/DOM. blob URL opaque origin
// adds a second isolation layer. NEVER add allow-same-origin here.
export function HtmlView({ objectUrl, title }: Props) {
  return (
    <iframe
      className="cw-preview-html"
      src={objectUrl}
      title={title}
      sandbox="allow-scripts"
    />
  );
}
