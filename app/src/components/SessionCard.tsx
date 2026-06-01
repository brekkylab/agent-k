// Session card — rich session preview (title, snippet, time, share, unread).
// Shared by the project home (previously) and the "View all" sessions overlay.

import { SharePill } from '@/components/uiPrimitives';
import { SessionCardMenu } from '@/components/SessionCardMenu';
import { SessionTitleText } from '@/components/SessionTitleText';
import { timeAgo } from '@/lib/timeAgo';
import type { Session } from '@/domain/types';

export function SessionCard({
  session,
  canDelete,
  onOpen,
  onRequestDelete,
}: {
  session: Session;
  canDelete: boolean;
  onOpen: () => void;
  onRequestDelete: () => void;
}) {
  const isUnread = session.unreadCount > 0;
  const timeLabel = session.lastMessageAt ? timeAgo(session.lastMessageAt) : null;

  return (
    <div
      className={`cw-session-card${isUnread ? ' is-unread' : ''}`}
      onClick={onOpen}
      role="button"
      tabIndex={0}
    >
      <div className="cw-session-card-head">
        <span className="cw-session-card-title">
          <SessionTitleText title={session.title} />
        </span>
        <span className="cw-session-right">
          {isUnread && (
            <span className="cw-unread-badge" aria-label={`unread ${session.unreadCount}`}>
              <span className="dot" />
              <span className="n">{session.unreadCount}</span>
            </span>
          )}
          <SharePill mode={session.shareMode} compact />
          {canDelete && <SessionCardMenu onDelete={onRequestDelete} />}
        </span>
      </div>
      {session.lastMessageSnippet && (
        <p className="cw-session-last">{session.lastMessageSnippet}</p>
      )}
      <div className="cw-session-card-footer">
        {timeLabel && <span className="cw-card-time">{timeLabel}</span>}
      </div>
    </div>
  );
}
