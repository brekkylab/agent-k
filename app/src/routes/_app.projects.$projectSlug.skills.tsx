import { createFileRoute } from '@tanstack/react-router';
import { useTranslation } from 'react-i18next';
import { EmptyState, SectionLabel } from '@/components/uiPrimitives';
import { Icon } from '@/components/Icon';

export const Route = createFileRoute('/_app/projects/$projectSlug/skills')({
  component: SkillsPage,
});

function SkillsPage() {
  const { t } = useTranslation('project');
  return (
    <section className="cw-page cw-simple-page cw-page-enter">
      <SectionLabel>{t('skills_page_meta.section_label')}</SectionLabel>
      <h1>{t('skills_page_meta.title')}</h1>
      <p>{t('skills_page.subtitle')}</p>
      <EmptyState
        title={t('skills_page.empty_title')}
        body={t('skills_page.empty_body')}
        chip={<Icon name="zap" size={16} />}
      />
    </section>
  );
}
