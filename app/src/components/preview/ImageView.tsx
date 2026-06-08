interface Props { objectUrl: string; alt: string }

export function ImageView({ objectUrl, alt }: Props) {
  return (
    <div className="cw-preview-image">
      <img src={objectUrl} alt={alt} />
    </div>
  );
}
