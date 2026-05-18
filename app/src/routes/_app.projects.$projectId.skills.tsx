import { createFileRoute } from '@tanstack/react-router';
import { EmptyState, SectionLabel } from '@/components/uiPrimitives';
import { Icon } from '@/components/Icon';

export const Route = createFileRoute('/_app/projects/$projectId/skills')({
  component: SkillsPage,
});

function SkillsPage() {
  return (
    <section className="cw-page cw-simple-page cw-page-enter">
      <SectionLabel>Reusable prompts & tools</SectionLabel>
      <h1>Skills</h1>
      <p>재사용 가능한 프롬프트 템플릿과 도구 바인딩을 한곳에서 관리합니다.</p>
      <EmptyState
        title="Skills 준비 중"
        body="Skills 기능은 곧 추가됩니다."
        chip={<Icon name="zap" size={16} />}
      />
    </section>
  );
}
