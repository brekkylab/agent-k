// Members — invite + leave/remove. Layout mirrors the wireframe
// (Member / Role / Joined / ⋯) but uses Cowork DS tokens directly.

import { useState } from 'react';
import { createFileRoute } from '@tanstack/react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { getProject, listMembers, removeMember } from '@/api/projects';
import { Avatar, EmptyState, SectionLabel } from '@/components/uiPrimitives';
import { Icon } from '@/components/Icon';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { SessionCardMenu } from '@/components/SessionCardMenu';
import { InviteDialog } from '@/components/InviteDialog';
import { useToastStore } from '@/components/Toast';
import { useAuthStore } from '@/stores/auth';
import { canInviteMembers, canLeaveProject, canRemoveMember } from '@/lib/permissions';
import { ApiError } from '@/api/client';
import type { User } from '@/domain/types';

export const Route = createFileRoute('/_app/projects/$projectId/members')({
  component: MembersPage,
});

interface RemoveTarget { user: User; isSelfLeave: boolean; }

function MembersPage() {
  const { projectId } = Route.useParams();
  const queryClient = useQueryClient();
  const currentUser = useAuthStore((s) => s.currentUser);
  const showToast = useToastStore((s) => s.show);

  const project = useQuery({ queryKey: ['project', projectId], queryFn: () => getProject(projectId) });
  const members = useQuery({ queryKey: ['members', projectId], queryFn: () => listMembers(projectId) });

  const [inviteOpen, setInviteOpen] = useState(false);
  const [removeTarget, setRemoveTarget] = useState<RemoveTarget | null>(null);

  const removeMutation = useMutation({
    mutationFn: (userId: string) => removeMember(projectId, userId),
    onSuccess: async (_, removedId) => {
      await queryClient.invalidateQueries({ queryKey: ['members', projectId] });
      if (removeTarget?.isSelfLeave) {
        // I left the project — drop me back to /projects.
        await queryClient.invalidateQueries({ queryKey: ['projects'] });
        showToast('프로젝트에서 나왔습니다');
        // Note: navigate happens via the parent layout's route guard when
        // the project disappears from the user's list; for now just toast.
      } else {
        showToast('멤버를 제거했습니다');
      }
      setRemoveTarget(null);
      void removedId;
    },
    onError: (err) => {
      const msg = err instanceof ApiError
        ? (err.status === 400 ? err.message : err.status === 403 ? '권한이 없습니다' : err.message)
        : err instanceof Error ? err.message : 'remove failed';
      showToast(`실패: ${msg}`);
    },
  });

  const proj = project.data;
  const memberList = members.data ?? [];

  // Compose the full member list including the owner so the table shows them
  // with an Owner badge. Backend's /members endpoint may or may not include
  // the owner row depending on how `add_project_member` is wired — we look it
  // up explicitly to be safe.
  const owner: User | null = memberList.find((m) => m.id === proj?.ownerId)
    ?? (proj && currentUser?.id === proj.ownerId ? currentUser : null);

  const ownerRow: User | null = owner ?? (proj
    ? { id: proj.ownerId, name: '(owner)', roleLabel: 'Owner', avatar: 'OW', color: 'var(--cw-ink)' }
    : null);

  const rows: User[] = ownerRow
    ? [ownerRow, ...memberList.filter((m) => m.id !== ownerRow.id)]
    : memberList;

  const inviteAllowed = canInviteMembers(proj, currentUser);
  const showInviteCta = inviteAllowed && rows.length <= 1;

  return (
    <section className="cw-page cw-simple-page cw-page-enter">
      <header className="cw-page-head">
        <div>
          <SectionLabel>Team access</SectionLabel>
          <h1>Members</h1>
          <p>{proj?.name ?? '...'} · 프로젝트 멤버를 관리합니다.</p>
        </div>
        <div className="cw-hero-actions">
          {inviteAllowed && (
            <button className="cw-btn-primary" onClick={() => setInviteOpen(true)}>
              <Icon name="plus" size={12} /> Invite
            </button>
          )}
        </div>
      </header>

      <div style={{
        border: '1px solid var(--cw-line)',
        borderRadius: 12,
        background: 'var(--cw-paper-2)',
        overflow: 'hidden',
      }}>
        <div style={{
          display: 'grid',
          gridTemplateColumns: 'minmax(0, 1fr) 90px 110px 40px',
          gap: 12,
          padding: '10px 16px',
          background: 'var(--cw-paper-3)',
          borderBottom: '1px solid var(--cw-line)',
          fontFamily: 'var(--cw-font-mono)',
          fontSize: 10,
          textTransform: 'uppercase',
          letterSpacing: '0.08em',
          color: 'var(--cw-ink-3)',
        }}>
          <span>Member</span>
          <span>Role</span>
          <span>Joined</span>
          <span />
        </div>

        {members.isLoading && (
          <div style={{ padding: 16, color: 'var(--cw-ink-3)', fontSize: 12 }}>불러오는 중…</div>
        )}

        {rows.map((user, idx) => (
          <MemberRow
            key={user.id}
            user={user}
            isOwner={user.id === proj?.ownerId}
            isMe={user.id === currentUser?.id}
            joinedAt={proj?.id ? '—' : '—'}
            showMenu={canRemoveMember(proj, user, currentUser)}
            isLastRow={idx === rows.length - 1}
            onRemove={() => setRemoveTarget({
              user,
              isSelfLeave: user.id === currentUser?.id,
            })}
          />
        ))}
      </div>

      {showInviteCta && (
        <button
          type="button"
          onClick={() => setInviteOpen(true)}
          style={{
            marginTop: 14,
            width: '100%',
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'center',
            gap: 8,
            padding: '28px 18px',
            border: '1.5px dashed var(--cw-paper-4)',
            borderRadius: 14,
            background: 'var(--cw-paper-2)',
            color: 'var(--cw-ink-3)',
            cursor: 'pointer',
          }}
          onMouseEnter={(e) => { (e.currentTarget as HTMLButtonElement).style.borderColor = 'var(--cw-ink-4)'; }}
          onMouseLeave={(e) => { (e.currentTarget as HTMLButtonElement).style.borderColor = 'var(--cw-paper-4)'; }}
        >
          <Icon name="users" size={22} />
          <span style={{ fontSize: 13, fontWeight: 500, color: 'var(--cw-ink-2)' }}>첫 멤버를 초대해보세요</span>
          <span style={{ fontSize: 11, color: 'var(--cw-ink-4)' }}>+ Invite 버튼으로 팀원을 추가할 수 있어요</span>
        </button>
      )}

      {!inviteAllowed && canLeaveProject(proj, currentUser) && (
        <p style={{ marginTop: 18, textAlign: 'right', fontSize: 12 }}>
          <button
            type="button"
            onClick={() => currentUser && setRemoveTarget({ user: currentUser, isSelfLeave: true })}
            style={{
              border: 0,
              background: 'transparent',
              color: 'var(--cw-destructive)',
              fontSize: 12,
              cursor: 'pointer',
              padding: 0,
              textDecoration: 'underline',
              textUnderlineOffset: 2,
            }}
          >
            이 프로젝트에서 나가기
          </button>
        </p>
      )}

      {memberList.length === 0 && !members.isLoading && !showInviteCta && !inviteAllowed && (
        <EmptyState
          title="멤버 정보를 불러올 수 없습니다"
          body="권한이 없거나 프로젝트가 비어있을 수 있습니다."
          chip={<Icon name="users" size={16} />}
        />
      )}

      {inviteOpen && (
        <InviteDialog projectId={projectId} onClose={() => setInviteOpen(false)} />
      )}

      {removeTarget && (
        <ConfirmDialog
          title={removeTarget.isSelfLeave ? '프로젝트에서 나가시겠어요?' : '멤버를 제거하시겠어요?'}
          body={
            removeTarget.isSelfLeave
              ? `"${proj?.name ?? 'project'}" 프로젝트에 더 이상 접근할 수 없게 됩니다.`
              : `"${removeTarget.user.name}"님을 프로젝트에서 제거합니다. 이 작업은 되돌릴 수 없습니다.`
          }
          confirmLabel={removeTarget.isSelfLeave ? '나가기' : '제거'}
          destructive
          pending={removeMutation.isPending}
          onConfirm={() => removeMutation.mutate(removeTarget.user.id)}
          onClose={() => setRemoveTarget(null)}
        />
      )}
    </section>
  );
}

interface MemberRowProps {
  user: User;
  isOwner: boolean;
  isMe: boolean;
  joinedAt: string;
  showMenu: boolean;
  isLastRow: boolean;
  onRemove: () => void;
}

function MemberRow({ user, isOwner, isMe, joinedAt, showMenu, isLastRow, onRemove }: MemberRowProps) {
  return (
    <div style={{
      display: 'grid',
      gridTemplateColumns: 'minmax(0, 1fr) 90px 110px 40px',
      gap: 12,
      alignItems: 'center',
      padding: '12px 16px',
      borderBottom: isLastRow ? 'none' : '1px solid var(--cw-line-soft)',
      background: 'var(--cw-paper)',
    }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, minWidth: 0 }}>
        <Avatar user={user} small />
        <div style={{ minWidth: 0 }}>
          <p style={{ margin: 0, fontSize: 13, fontWeight: 500, color: 'var(--cw-ink)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {user.name}
            {isMe && (
              <span style={{ marginLeft: 6, color: 'var(--cw-ink-4)', fontWeight: 400 }}>(you)</span>
            )}
          </p>
        </div>
      </div>

      <div>
        {isOwner ? (
          <span className="cw-role-badge owner" style={{ fontSize: 10 }}>Owner</span>
        ) : (
          <span className="cw-role-badge member" style={{ fontSize: 10 }}>Member</span>
        )}
      </div>

      <span style={{ fontSize: 11, color: 'var(--cw-ink-4)', fontFamily: 'var(--cw-font-mono)' }}>{joinedAt}</span>

      <div style={{ display: 'flex', justifyContent: 'flex-end' }}>
        {showMenu && <SessionCardMenu onDelete={onRemove} />}
      </div>
    </div>
  );
}
