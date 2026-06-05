import { useEffect, useState } from 'react';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';
import { deleteProject, getProject, listMembers, listProjects, updateProject } from '@/api/projects';
import { Avatar, SectionLabel } from '@/components/uiPrimitives';
import { Icon } from '@/components/Icon';
import type { User } from '@/domain/types';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { useToastStore } from '@/components/Toast';
import { useAuthStore } from '@/stores/auth';
import { canEditProject } from '@/lib/permissions';
import { ApiError } from '@/api/client';
import { loadNs } from '@/i18n/loader';

export const Route = createFileRoute('/_app/projects/$projectSlug/settings')({
  loader: () => loadNs('project'),
  component: SettingsPage,
});

type ProjectT = TFunction<'project'>;

function messageOf(err: unknown, t: ProjectT): string {
  if (err instanceof ApiError) {
    if (err.status === 401) return t('settings_page.errors.login_expired');
    if (err.status === 403) return t('settings_page.errors.no_permission');
    if (err.status === 404) return t('settings_page.errors.not_found');
    if (err.status === 409) return t('settings_page.errors.name_conflict');
    if (err.status === 422 || err.status === 400) return t('settings_page.errors.validation_failed');
    return `${err.status} — ${err.message}`;
  }
  if (err instanceof Error) return err.message;
  return t('settings_page.errors.request_failed');
}

function SettingsPage() {
  const { projectSlug } = Route.useParams();
  const { t } = useTranslation('project');
  const { t: tCommon } = useTranslation('common');
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);
  const currentUser = useAuthStore((s) => s.currentUser);

  const project = useQuery({ queryKey: ['project', projectSlug], queryFn: () => getProject(projectSlug) });
  const members = useQuery({ queryKey: ['members', projectSlug], queryFn: () => listMembers(projectSlug) });

  const editable = canEditProject(project.data, currentUser);

  const ownerUser: User | null = (() => {
    if (!project.data) return null;
    const fromMembers = (members.data ?? []).find((m) => m.id === project.data!.ownerId);
    if (fromMembers) return fromMembers;
    if (currentUser?.id === project.data.ownerId) return currentUser;
    return null;
  })();

  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [editMode, setEditMode] = useState(false);
  const [descEditMode, setDescEditMode] = useState(false);

  // Seed inputs once the server data arrives, keyed by project id so an
  // external refetch during edit doesn't clobber what the user typed.
  useEffect(() => {
    if (project.data) {
      setName(project.data.name);
      setDescription(project.data.description ?? '');
    }
  }, [project.data?.id]);

  const updateMutation = useMutation({
    mutationFn: () => updateProject(projectSlug, { name: name.trim() }),
    onSuccess: async (updated) => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ['project', projectSlug] }),
        queryClient.invalidateQueries({ queryKey: ['projects'] }),
      ]);
      setName(updated.name);
      setSubmitError(null);
      setEditMode(false);
      showToast(t('settings_page.toasts.rename_success'));
    },
    onError: (err) => setSubmitError(messageOf(err, t)),
  });

  const descMutation = useMutation({
    mutationFn: () => updateProject(projectSlug, {
      description: description.trim() ? description.trim() : null,
    }),
    onSuccess: async (updated) => {
      await queryClient.invalidateQueries({ queryKey: ['project', projectSlug] });
      setDescription(updated.description ?? '');
      setSubmitError(null);
      setDescEditMode(false);
      showToast(t('settings_page.toasts.description_success'));
    },
    onError: (err) => setSubmitError(messageOf(err, t)),
  });

  const deleteMutation = useMutation({
    mutationFn: () => deleteProject(projectSlug),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['projects'] });
      queryClient.removeQueries({ queryKey: ['project', projectSlug] });
      const remaining = (await queryClient.fetchQuery({ queryKey: ['projects'], queryFn: listProjects })) ?? [];
      showToast(t('settings_page.toasts.delete_success'));
      setDeleteOpen(false);
      if (remaining[0]) {
        navigate({ to: '/projects/$projectSlug', params: { projectSlug: remaining[0].slug } });
      } else {
        navigate({ to: '/projects' });
      }
    },
    onError: (err) => {
      showToast(t('settings_page.errors.delete_failed', { message: messageOf(err, t) }));
    },
  });

  const trimmedName = name.trim();
  const dirty = project.data ? trimmedName !== project.data.name : false;
  const saveDisabled = !editable || !dirty || trimmedName.length === 0 || updateMutation.isPending;

  const descDirty = project.data
    ? description.trim() !== (project.data.description ?? '')
    : false;
  const descSaveDisabled = !editable || !descDirty || descMutation.isPending;

  return (
    <section className="cw-page cw-page-enter">
      <SectionLabel>{t('settings_page.section_label')}</SectionLabel>
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 18,
          flexWrap: 'wrap',
          marginTop: 4,
        }}
      >
        <div style={{ display: 'inline-flex', alignItems: 'center', gap: 10 }}>
          {editable && editMode ? (
            <>
              <input
                autoFocus
                value={name}
                onChange={(e) => { setName(e.target.value); if (submitError) setSubmitError(null); }}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') {
                    e.preventDefault();
                    if (!saveDisabled) updateMutation.mutate();
                  } else if (e.key === 'Escape') {
                    e.preventDefault();
                    if (project.data) setName(project.data.name);
                    setSubmitError(null);
                    setEditMode(false);
                  }
                }}
                disabled={updateMutation.isPending}
                maxLength={100}
                aria-label={t('settings_page.edit_name_aria')}
                style={{
                  margin: 0,
                  padding: '4px 10px',
                  border: '1px solid var(--cw-line)',
                  borderRadius: 8,
                  background: 'var(--cw-paper)',
                  color: 'var(--cw-ink)',
                  fontSize: 'var(--cw-text-2xl)',
                  lineHeight: 1.12,
                  letterSpacing: '-0.025em',
                  fontWeight: 650,
                  fontFamily: 'inherit',
                  minWidth: 280,
                }}
              />
              <button
                type="button"
                className="cw-btn-primary"
                onClick={() => updateMutation.mutate()}
                disabled={saveDisabled}
              >
                {updateMutation.isPending ? tCommon('state.saving') : tCommon('actions.save')}
              </button>
              <button
                type="button"
                className="cw-btn-secondary"
                onClick={() => {
                  if (project.data) setName(project.data.name);
                  setSubmitError(null);
                  setEditMode(false);
                }}
                disabled={updateMutation.isPending}
              >
                {tCommon('actions.cancel')}
              </button>
            </>
          ) : (
            <>
              <h1 style={{ margin: 0 }}>{project.data?.name ?? '…'}</h1>
              {editable && project.data && (
                <button
                  type="button"
                  onClick={() => {
                    setSubmitError(null);
                    setEditMode(true);
                  }}
                  aria-label={t('settings_page.edit_name_aria')}
                  title={t('settings_page.edit_title')}
                  style={{
                    display: 'inline-flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                    width: 30,
                    height: 30,
                    border: '1px solid var(--cw-line)',
                    borderRadius: 8,
                    background: 'var(--cw-paper-2)',
                    color: 'var(--cw-ink-3)',
                    cursor: 'pointer',
                    padding: 0,
                  }}
                >
                  <Icon name="writing" size={14} />
                </button>
              )}
            </>
          )}
        </div>
        {ownerUser && (
          <span
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 8,
              fontSize: 13,
              color: 'var(--cw-ink-3)',
            }}
          >
            <span>{t('settings_page.owned_by')}</span>
            <Avatar user={ownerUser} small />
            <b style={{ color: 'var(--cw-ink-2)', fontWeight: 600 }}>{ownerUser.name}</b>
          </span>
        )}
      </div>
      <p
        style={{
          margin: '6px 0 24px',
          fontSize: 12,
          color: 'var(--cw-ink-4)',
          fontFamily: 'var(--cw-font-mono)',
          fontStyle: 'italic',
        }}
      >
        {projectSlug}
      </p>

      {project.data && (
        <div style={{ marginBottom: 24 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 6 }}>
            <SectionLabel>{t('settings_page.description_label')}</SectionLabel>
            {editable && !descEditMode && (
              <button
                type="button"
                onClick={() => {
                  setSubmitError(null);
                  setDescEditMode(true);
                }}
                aria-label={t('settings_page.edit_desc_aria')}
                title={t('settings_page.edit_title')}
                style={{
                  display: 'inline-flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  width: 22,
                  height: 22,
                  border: '1px solid var(--cw-line)',
                  borderRadius: 6,
                  background: 'var(--cw-paper-2)',
                  color: 'var(--cw-ink-3)',
                  cursor: 'pointer',
                  padding: 0,
                }}
              >
                <Icon name="writing" size={12} />
              </button>
            )}
          </div>

          {editable && descEditMode ? (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
              <textarea
                autoFocus
                value={description}
                onChange={(e) => { setDescription(e.target.value); if (submitError) setSubmitError(null); }}
                onKeyDown={(e) => {
                  if (e.key === 'Escape') {
                    e.preventDefault();
                    setDescription(project.data?.description ?? '');
                    setSubmitError(null);
                    setDescEditMode(false);
                  }
                }}
                placeholder={t('settings_page.description_placeholder')}
                disabled={descMutation.isPending}
                rows={3}
                style={{
                  resize: 'vertical',
                  minHeight: 80,
                  fontFamily: 'inherit',
                  fontSize: 14,
                  lineHeight: 1.6,
                  padding: '10px 12px',
                  border: '1px solid var(--cw-line)',
                  borderRadius: 8,
                  background: 'var(--cw-paper)',
                  color: 'var(--cw-ink)',
                }}
              />
              <div style={{ display: 'flex', gap: 10, justifyContent: 'flex-end' }}>
                <button
                  type="button"
                  className="cw-btn-secondary"
                  onClick={() => {
                    setDescription(project.data?.description ?? '');
                    setSubmitError(null);
                    setDescEditMode(false);
                  }}
                  disabled={descMutation.isPending}
                >
                  {tCommon('actions.cancel')}
                </button>
                <button
                  type="button"
                  className="cw-btn-primary"
                  onClick={() => descMutation.mutate()}
                  disabled={descSaveDisabled}
                >
                  {descMutation.isPending ? tCommon('state.saving') : tCommon('actions.save')}
                </button>
              </div>
            </div>
          ) : (
            <p
              style={{
                margin: 0,
                color: project.data.description ? 'var(--cw-ink-2)' : 'var(--cw-ink-4)',
                fontSize: 14,
                lineHeight: 1.6,
              }}
            >
              {project.data.description || t('settings_page.description_empty')}
            </p>
          )}
        </div>
      )}

      {submitError && (
        <div className="cw-dialog-warn" role="alert" style={{ marginBottom: 20 }}>{submitError}</div>
      )}

      {editable && (
        <div
          style={{
            marginTop: 24,
            border: '1px solid var(--cw-destructive)',
            borderRadius: 12,
            background: 'var(--cw-paper-2)',
            padding: 20,
          }}
        >
          <SectionLabel>{t('settings_page.danger_zone')}</SectionLabel>
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 16, marginTop: 8 }}>
            <div style={{ minWidth: 0 }}>
              <h2 style={{ margin: 0, fontSize: 14, color: 'var(--cw-ink)' }}>{t('settings_page.delete_project_heading')}</h2>
              <p style={{ margin: '4px 0 0', color: 'var(--cw-ink-3)', fontSize: 12, lineHeight: 1.55 }}>
                {t('settings_page.delete_project_help')}
              </p>
            </div>
            <button
              type="button"
              className="cw-btn-primary"
              style={{ background: 'var(--cw-destructive)', borderColor: 'var(--cw-destructive)' }}
              onClick={() => setDeleteOpen(true)}
            >
              {t('settings_page.delete_project_button')}
            </button>
          </div>
        </div>
      )}

      {deleteOpen && project.data && (
        <ConfirmDialog
          title={t('settings_page.delete_dialog_title')}
          body={t('settings_page.delete_dialog_body', { name: project.data.name })}
          confirmLabel={tCommon('actions.delete')}
          destructive
          pending={deleteMutation.isPending}
          onConfirm={() => deleteMutation.mutate()}
          onClose={() => setDeleteOpen(false)}
        />
      )}
    </section>
  );
}
