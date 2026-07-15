import type { SourceEntry, SourceDetail, SourceProvider, SourceType, ListCtx } from '../types';

export function makeMockProvider(
  cfg: Pick<SourceProvider, 'id' | 'nameKey' | 'category' | 'kind' | 'count'> & { connected?: boolean },
  fixture: { entries: SourceEntry[]; details: Record<string, SourceDetail> },
): SourceProvider {
  const delay = <T,>(v: T) => new Promise<T>((r) => setTimeout(() => r(v), 250));
  return {
    ...cfg,
    // A mock/catalog provider's id IS its type (one static entry per type).
    type: cfg.id as SourceType,
    connected: cfg.connected ?? true,
    attachable: false,
    list: (ctx: ListCtx) => delay(fixture.entries.filter((e) => (ctx.path ?? '') === '' ? true : e.path?.startsWith(ctx.path!))),
    recent: () => delay([...fixture.entries].sort((a, b) => b.modifiedAt.localeCompare(a.modifiedAt)).slice(0, 20)),
    detail: (id: string) => {
      const d = fixture.details[id];
      return d ? delay(d) : Promise.reject(new Error(`no detail for ${id}`));
    },
  };
}
