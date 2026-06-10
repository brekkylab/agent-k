interface Props { content: string }

export function TextView({ content }: Props) {
  return <pre className="cw-preview-doc cw-preview-text">{content}</pre>;
}
