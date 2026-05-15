// Files — markup mirrors app-live FilesPage. Folder tree + file list pane,
// search, upload zone, knowledge promo card.

import { useDeferredValue, useRef, useState } from 'react';
import { createFileRoute } from '@tanstack/react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { createFolder, deleteDirent, downloadFile, listDirents, uploadFile } from '@/api/dirents';
import { getProject } from '@/api/projects';
import { Icon } from '@/components/Icon';
import { FileIcon } from '@/components/fileComponents';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { EmptyState, IconPocket } from '@/components/uiPrimitives';
import { useToastStore } from '@/components/Toast';
import { ApiError } from '@/api/client';
import { folderOf, getProjectFolders } from '@/domain/files';
import type { FileAsset } from '@/domain/types';

export const Route = createFileRoute('/_app/projects/$projectId/files')({
  component: FilesPage,
});

function FilesPage() {
  const { projectId } = Route.useParams();
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);

  const project = useQuery({ queryKey: ['project', projectId], queryFn: () => getProject(projectId) });
  const dirents = useQuery({
    queryKey: ['dirents', projectId, project.data?.name ?? ''],
    queryFn: () => listDirents(projectId, project.data?.name ?? 'project'),
    enabled: Boolean(project.data),
  });

  const fileInputRef = useRef<HTMLInputElement>(null);
  const [query, setQuery] = useState('');
  const deferred = useDeferredValue(query);

  const folders = getProjectFolders(dirents.data ?? [], projectId);
  const [selectedFolder, setSelectedFolder] = useState<string>('');
  const activeFolder = folders.some((folder) => folder.key === selectedFolder) ? selectedFolder : folders[0]?.key ?? 'General';

  const files = (dirents.data ?? []).filter(
    (file) => file.projectId === projectId
      && folderOf(file) === activeFolder
      && `${file.name} ${file.summary}`.toLowerCase().includes(deferred.toLowerCase()),
  );

  const uploadMutation = useMutation({
    mutationFn: async (file: File) => {
      const path = activeFolder ? `${activeFolder}/${file.name}` : file.name;
      await uploadFile(projectId, file, path);
    },
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['dirents', projectId] });
      showToast('파일이 업로드되었습니다');
    },
    onError: (err) => {
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'upload failed';
      showToast(`업로드 실패: ${msg}`);
    },
  });

  const folderMutation = useMutation({
    mutationFn: (name: string) =>
      createFolder(projectId, activeFolder && activeFolder !== 'General' ? `${activeFolder}/${name}` : name),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['dirents', projectId] });
      showToast('폴더가 생성되었습니다');
    },
    onError: (err) => {
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'mkdir failed';
      showToast(`폴더 생성 실패: ${msg}`);
    },
  });

  const [deleteTarget, setDeleteTarget] = useState<FileAsset | null>(null);

  const downloadMutation = useMutation({
    mutationFn: (file: FileAsset) => {
      const segments = file.path.split('/').filter(Boolean);
      const relative = segments.slice(1).join('/') || segments.join('/');
      return downloadFile(projectId, relative);
    },
    onError: (err) => {
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'download failed';
      showToast(`다운로드 실패: ${msg}`);
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (file: FileAsset) => {
      const segments = file.path.split('/').filter(Boolean);
      const relative = segments.slice(1).join('/') || segments.join('/');
      return deleteDirent(projectId, relative);
    },
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['dirents', projectId] });
      showToast('파일이 삭제되었습니다');
      setDeleteTarget(null);
    },
    onError: (err) => {
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'delete failed';
      showToast(`삭제 실패: ${msg}`);
    },
  });

  function openUpload() { fileInputRef.current?.click(); }
  function openFolderDialog() {
    const name = window.prompt('새 폴더 이름');
    if (!name?.trim()) return;
    folderMutation.mutate(name.trim());
  }

  return (
    <section className="cw-page cw-files-page cw-page-enter">
      <div className="cw-page-head">
        <div>
          <h1>Files</h1>
          <p>Shared with the whole project. Select files, pin them to a session, then generate artifacts from those citations.</p>
        </div>
        <div>
          <button className="cw-btn-secondary" onClick={openFolderDialog} disabled={folderMutation.isPending}>
            <IconPocket tone="add" icon="plus" compact /> New folder
          </button>
          <button className="cw-btn-primary" onClick={openUpload} disabled={uploadMutation.isPending}>
            <IconPocket tone="add" icon="upload" compact /> {uploadMutation.isPending ? 'Uploading…' : 'Upload'}
          </button>
          <input
            ref={fileInputRef}
            type="file"
            hidden
            onChange={(e) => {
              const file = e.target.files?.[0];
              if (file) uploadMutation.mutate(file);
              e.target.value = '';
            }}
          />
        </div>
      </div>

      <div className="cw-file-browser">
        <aside className="cw-folder-tree">
          <div className="cw-folder-root">
            <IconPocket tone="files" icon="folder" /> {project.data?.name ?? 'Project'}
          </div>
          {folders.map((folder) => (
            <button
              key={folder.key}
              className={activeFolder === folder.key ? 'is-active' : ''}
              onClick={() => setSelectedFolder(folder.key)}
            >
              <IconPocket tone="files" icon={activeFolder === folder.key ? 'folder-open' : 'folder'} />
              {folder.label}
              <span>{folder.count}</span>
            </button>
          ))}
        </aside>
        <section className="cw-file-list">
          <div className="cw-file-path">
            <span>
              <FileIcon
                file={{
                  id: 'folder-open',
                  projectId,
                  name: activeFolder,
                  path: `${project.data?.name ?? ''}/${activeFolder}`,
                  type: 'folder',
                  sizeLabel: '',
                  updatedAt: '',
                  summary: '',
                  groundTruth: [],
                }}
                compact
              />
              <b>{activeFolder}</b>
            </span>
            <label>
              <IconPocket tone="search" icon="search" compact />
              <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="파일 검색" />
            </label>
          </div>
          {files.length ? files.map((file: FileAsset) => (
            <div
              key={file.id}
              className="cw-file-row"
              role="button"
              tabIndex={0}
              onClick={() => downloadMutation.mutate(file)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  downloadMutation.mutate(file);
                }
              }}
              aria-disabled={downloadMutation.isPending}
              style={{ cursor: 'pointer' }}
            >
              <FileIcon file={file} />
              <span className="cw-file-main">
                <span className="name">{file.name}</span>
                <span className="meta">{file.summary} · {file.updatedAt} · {file.sizeLabel}</span>
              </span>
              <button
                type="button"
                className="cw-file-more"
                aria-label="파일 삭제"
                onClick={(e) => { e.stopPropagation(); setDeleteTarget(file); }}
                style={{ border: 0, background: 'transparent', cursor: 'pointer' }}
              >
                <Icon name="trash" size={14} />
              </button>
            </div>
          )) : (
            <EmptyState
              title="No files match"
              body="Try another folder, clear the search, or upload a backend file."
              action="Upload file"
              onAction={openUpload}
              chip={<Icon name="folder" size={16} />}
            />
          )}
          <div className="cw-dropzone" onClick={openUpload} role="button" tabIndex={0}>
            <IconPocket tone="add" icon="plus" />
            <span>파일을 드래그하거나 Upload를 누르세요. 업로드된 파일은 세션에서 ground truth로 선택할 수 있습니다.</span>
          </div>
          <div className="cw-knowledge">
            <h2><IconPocket tone="content" icon="sparkles" /> Knowledge <small>soon</small></h2>
            <p>인덱싱과 요약 기능이 추가되면, 선택한 파일을 세션의 ground truth로 pin할 수 있습니다.</p>
          </div>
        </section>
      </div>
      {deleteTarget && (
        <ConfirmDialog
          title="파일을 삭제하시겠어요?"
          body={`${deleteTarget.name} 을(를) 삭제하면 되돌릴 수 없습니다.`}
          confirmLabel="삭제"
          destructive
          pending={deleteMutation.isPending}
          onConfirm={() => deleteMutation.mutate(deleteTarget)}
          onClose={() => { if (!deleteMutation.isPending) setDeleteTarget(null); }}
        />
      )}
    </section>
  );
}
