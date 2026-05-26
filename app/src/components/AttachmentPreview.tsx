import { useEffect, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { fetchFileBlob, downloadFileByGlobalPath } from '@/api/dirents';
import { Icon } from './Icon';
import { FileTypeIcon } from './FileTypeIcon';

interface Props { globalPath: string; }

function isImagePath(path: string): boolean {
  return /\.(png|jpe?g|webp|gif|svg)$/i.test(path);
}

export function AttachmentPreview({ globalPath }: Props) {
  const filename = globalPath.split('/').pop() ?? globalPath;
  const isImage = isImagePath(filename);

  const blobQuery = useQuery({
    queryKey: ['file-blob', globalPath],
    queryFn: () => fetchFileBlob(globalPath),
    enabled: isImage,
    staleTime: Infinity,
    gcTime: 5 * 60 * 1000,
  });

  const [objectUrl, setObjectUrl] = useState<string | null>(null);

  useEffect(() => {
    if (!blobQuery.data) return;
    const url = URL.createObjectURL(blobQuery.data);
    setObjectUrl(url);
    return () => { URL.revokeObjectURL(url); };
  }, [blobQuery.data]);

  if (isImage) {
    return (
      <div className="cw-attach-preview">
        {objectUrl
          ? <img src={objectUrl} alt={filename} className="cw-attach-thumb" />
          : <div className="cw-attach-loading"><Icon name="image" size={16} /></div>}
        <span className="cw-attach-label">{filename}</span>
      </div>
    );
  }

  return (
    <button
      type="button"
      className="cw-attach-chip cw-attach-chip--file"
      onClick={() => void downloadFileByGlobalPath(globalPath)}
      title={`Download ${filename}`}
    >
      <FileTypeIcon filename={filename} size={16} />
      <span>{filename}</span>
    </button>
  );
}
