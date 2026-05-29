import { createFileRoute } from '@tanstack/react-router';
import { useTranslation } from 'react-i18next';
import { EmptyState, SectionLabel } from '@/components/uiPrimitives';
import { Icon } from '@/components/Icon';
import { loadNs } from '@/i18n/loader';

export const Route = createFileRoute('/_app/projects/$projectSlug/schedule')({
  loader: () => loadNs('project'),
  component: SchedulePage,
});

function SchedulePage() {
  const { t } = useTranslation('project');
  return (
    <section className="cw-page cw-simple-page cw-page-enter">
      <SectionLabel>{t('schedule_page_meta.section_label')}</SectionLabel>
      <h1>{t('schedule_page_meta.title')}</h1>
      <p>{t('schedule_page.subtitle')}</p>
      <EmptyState
        title={t('schedule_page.empty_title')}
        body={t('schedule_page.empty_body')}
        chip={<Icon name="calendar" size={16} />}
      />
    </section>
  );
}
