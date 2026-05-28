import { createFileRoute } from '@tanstack/react-router';
import { useTranslation } from 'react-i18next';
import { EmptyState, SectionLabel } from '@/components/uiPrimitives';
import { Icon } from '@/components/Icon';

export const Route = createFileRoute('/_app/projects/$projectSlug/schedule')({
  component: SchedulePage,
});

function SchedulePage() {
  const { t } = useTranslation('project');
  return (
    <section className="cw-page cw-simple-page cw-page-enter">
      <SectionLabel>Recurring runs</SectionLabel>
      <h1>Schedule</h1>
      <p>{t('schedule_page.subtitle')}</p>
      <EmptyState
        title={t('schedule_page.empty_title')}
        body={t('schedule_page.empty_body')}
        chip={<Icon name="calendar" size={16} />}
      />
    </section>
  );
}
