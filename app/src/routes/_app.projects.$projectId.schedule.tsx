import { createFileRoute } from '@tanstack/react-router';
import { EmptyState, SectionLabel } from '@/components/uiPrimitives';
import { Icon } from '@/components/Icon';

export const Route = createFileRoute('/_app/projects/$projectId/schedule')({
  component: SchedulePage,
});

function SchedulePage() {
  return (
    <section className="cw-page cw-simple-page cw-page-enter">
      <SectionLabel>Recurring runs</SectionLabel>
      <h1>Schedule</h1>
      <p>주기 실행과 알림 배달을 한곳에서 관리합니다.</p>
      <EmptyState
        title="Schedule 준비 중"
        body="스케줄 기능은 곧 추가됩니다."
        chip={<Icon name="calendar" size={16} />}
      />
    </section>
  );
}
