import { Icon, type IconName } from './Icon';
import type { Artifact, FileAsset } from '../domain/types';
import { byId } from './uiPrimitives';

export function FileIcon({ file, compact = false }: { file: FileAsset; compact?: boolean }) {
  const icon: IconName = file.type === 'sheet'
    ? 'sheet'
    : file.type === 'image'
      ? 'image'
      : file.type === 'folder'
        ? 'folder'
        : 'file-text';

  return <span className={`cw-pocket cw-file-${file.type} ${compact ? 'is-compact' : ''}`}><Icon name={icon} size={compact ? 12 : 16} /></span>;
}

export function FileTiny({ file }: { file: FileAsset }) {
  return <div className="cw-side-file"><span><FileIcon file={file} />{file.name}</span><small>{file.sizeLabel}</small></div>;
}

export function Evidence({ ids, files }: { ids: string[]; files: FileAsset[] }) {
  return <div className="cw-evidence">{ids.map((id) => <span key={id}><FileIcon file={byId(files, id)} />{byId(files, id).name}</span>)}</div>;
}

export function ArtifactPreview({ artifact, files }: { artifact: Artifact; files: FileAsset[] }) {
  return (
    <div className="cw-artifact-preview">
      <h3><Icon name="artifact" /> {artifact.title}</h3>
      {artifact.sections.map((section) => (
        <section key={section.label}>
          <b>{section.label}</b>
          <p>{section.body}</p>
          {section.evidence && <Evidence ids={section.evidence} files={files} />}
        </section>
      ))}
    </div>
  );
}
