# Workspace Hub Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace app_v2's seed `/workspace` page with a multi-source hub — source rail + unified "전체" view + per-source views (local files real via webdav, 6 sources mocked) + "새 대화에서 쓰기" chat bridge.

**Architecture:** A single `SourceProvider` registry drives everything (rail, routing, unified merge). LocalProvider adapts the existing webdav module; MockProviders serve static Korean fixtures with simulated latency. Three view archetypes (files/items/threads) are shared across sources. Detail panel + existing FilePreviewModal for deep preview. Attachment hand-off to the home composer goes through a module-singleton store.

**Tech Stack:** React 19 + TanStack Router/Query (file-based routes), existing webdav client (`api/workspace.ts`), vitest + testing-library, ported cowork design system + theme.css indigo palette.

**Spec:** `docs/superpowers/specs/2026-07-03-workspace-hub-design.md` (user-approved; row spec B, hybrid layout).

## Global Constraints

- Worktree `/Users/jeffrey/workspace/agent-k-revise`, dir `app_v2/`, branch `feat/app-v2`. Local commits only, NEVER push. pnpm. English code comments.
- **No backend changes. No new npm deps** (brand icons are hand-written inline SVG components; no icon package).
- All user-facing strings via i18n (en+ko, BOTH locales). Workspace-surface strings → `files` namespace. Strings rendered by the HOME surface (`routes/index.tsx`) → `session` namespace (home uses `useTranslation('session')`; all `home.*` keys live in `session.json`).
- Mock providers use real `setTimeout(250)` latency — provider/view/detail tests MUST run on real timers (do NOT use `vi.useFakeTimers()`; it would hang the awaits).
- Visual: use existing design-system classes/tokens (`--cw-*`); no new hardcoded colors except the 7 brand icon fills (brand colors are content, not theme).
- '전체' row spec (B): fixed 18px brand icon + fixed 76px source-name column + title (ellipsis) + right-aligned meta. All rows identical height.
- Verification per task: `export PATH="/opt/homebrew/bin:$PATH" && cd /Users/jeffrey/workspace/agent-k-revise/app_v2 && pnpm lint && pnpm test:unit` (add `pnpm build` in Tasks 2-6).
- Existing suite (39 tests) must stay green. Note: no existing test covers `routes/workspace.tsx` (see Task 2 Step 4) — all new UI tests are net-new.

## File Structure (locked)

```
app_v2/src/workspace/
├── types.ts                 # SourceEntry, SourceDetail, ListCtx, SourceProvider, SourceCategory/Kind
├── providers.ts             # PROVIDERS registry + allRecent() + getProvider(id)
├── providers/local.ts       # LocalProvider (adapts api/workspace.ts)
├── providers/mock.ts        # makeMockProvider(config, fixture)
├── fixtures/gdrive.ts | s3.ts | confluence.ts | jira.ts | gmail.ts | slack.ts
├── icons.tsx                # 7 brand SVG components + SourceIcon dispatcher (18px slot)
└── components/
    ├── WorkspaceShell.tsx   # shared shell component: [SourceRail | main slot | DetailPanel slot]
    ├── SourceRail.tsx       # 전체 / categories / sources / +소스 추가(mock dialog)
    ├── UnifiedList.tsx      # '전체' view (row spec B) + client text filter
    ├── FileBrowserView.tsx  # files archetype (breadcrumb + folders; local real)
    ├── ItemListView.tsx     # items archetype (Confluence/Jira)
    ├── ThreadListView.tsx   # threads archetype (Gmail/Slack)
    └── DetailPanel.tsx      # meta + mini preview + actions
app_v2/src/stores/pendingAttachment.ts
app_v2/src/routes/workspace.tsx          # rewritten LEAF route (/workspace): renders <WorkspaceShell> with UnifiedList
app_v2/src/routes/workspace.$sourceId.tsx # SIBLING leaf route: renders <WorkspaceShell> with the archetype view
app_v2/src/routes/index.tsx              # modify: attachment chip + prefix injection in HomePage (NOT in ProjectHomeComposer)

ROUTING CONTRACT (explicit — matches this repo's flat file-based convention, e.g. sessions.$sessionId):
`workspace.tsx` and `workspace.$sourceId.tsx` are independent SIBLING routes, NOT nested — there is no
layout route and no <Outlet/>. Each route renders its own <WorkspaceShell> instance. Consequently the
selected-entry/detail-panel state lives INSIDE WorkspaceShell (component state, per route instance) and
is intentionally reset when navigating between sources — the detail panel clearing on source change is
the desired behavior, and the rail remount cost is negligible.
```

---

### Task 1: Data layer — types, icons, fixtures, providers, registry

**Files:**
- Create: `app_v2/src/workspace/types.ts`, `icons.tsx`, `providers/local.ts`, `providers/mock.ts`, `providers.ts`, `fixtures/{gdrive,s3,confluence,jira,gmail,slack}.ts`
- Test: `app_v2/src/workspace/__tests__/providers.test.ts`

**Interfaces (Produces — later tasks rely on these exact shapes):**

```ts
// types.ts
export type SourceCategory = 'files' | 'docs' | 'messages';
export type SourceKind = 'files' | 'items' | 'threads';

export interface SourceEntry {
  id: string;               // provider-unique
  sourceId: string;         // provider id
  title: string;
  subtitle?: string;        // sender/space/status line for items & threads
  kind: 'file' | 'folder' | 'item' | 'thread';
  size?: number;            // bytes, files only
  modifiedAt: string;       // ISO
  path?: string;            // files: webdav-relative path e.g. "/reports/q2.pdf"
}

export interface SourceDetail {
  entry: SourceEntry;
  bodyPreview?: string;     // items/threads: text body; threads render as bubbles (speaker + text lines separated by \n)
  externalUrl?: string;     // mock sources: '원본 열기' target (may be '#')
}

export interface ListCtx { path?: string }   // files archetype: current folder ('' = root)

export interface SourceProvider {
  id: 'local' | 'gdrive' | 's3' | 'confluence' | 'jira' | 'gmail' | 'slack';
  nameKey: string;          // i18n key in `files` ns, e.g. 'workspace.src.local'
  category: SourceCategory;
  kind: SourceKind;
  connected: boolean;       // all true in v1 (mocks demo as connected)
  attachable: boolean;      // local only
  count: number | null;     // rail badge; null → '—'
  list(ctx: ListCtx): Promise<SourceEntry[]>;
  recent(): Promise<SourceEntry[]>;     // ≤20, newest first
  detail(id: string): Promise<SourceDetail>;
}

// providers.ts
export const PROVIDERS: SourceProvider[];              // order = rail order: local, gdrive, s3, confluence, jira, gmail, slack
export function getProvider(id: string): SourceProvider | undefined;
export async function allRecent(): Promise<SourceEntry[]>;  // merge all recent(), sort modifiedAt desc

// icons.tsx
export function SourceIcon({ sourceId, size = 18 }: { sourceId: string; size?: number }): JSX.Element;
```

- [ ] **Step 1: Write failing tests** — `providers.test.ts`:

```ts
import { describe, expect, it, vi } from 'vitest';

// LocalProvider reaches webdav → workspaceClient() → getProjectId(), which throws
// "bootstrap has not run" in tests. Mock BOTH modules (mirror the pattern in
// src/api/__tests__/workspace.test.ts) BEFORE importing the registry.
vi.mock('@/api/workspace', () => ({
  listDirectory: vi.fn().mockResolvedValue([]),
  getFileBlob: vi.fn(),
  putFile: vi.fn(),
  deleteEntry: vi.fn(),
  createDirectory: vi.fn(),
  workspaceClient: vi.fn(),
}));
vi.mock('@/stores/project', () => ({
  getProjectId: vi.fn(() => 'test-pid'),
  setProjectId: vi.fn(),
}));

import { PROVIDERS, allRecent, getProvider } from '../providers';

describe('provider registry', () => {
  it('registers 7 providers in rail order', () => {
    expect(PROVIDERS.map((p) => p.id)).toEqual(['local', 'gdrive', 's3', 'confluence', 'jira', 'gmail', 'slack']);
  });
  it('only local is attachable', () => {
    expect(PROVIDERS.filter((p) => p.attachable).map((p) => p.id)).toEqual(['local']);
  });
  it('groups by the three categories', () => {
    const cats = new Set(PROVIDERS.map((p) => p.category));
    expect(cats).toEqual(new Set(['files', 'docs', 'messages']));
  });
  it('allRecent merges newest-first across providers', async () => {
    const merged = await allRecent();
    expect(merged.length).toBeGreaterThan(10);
    const times = merged.map((e) => e.modifiedAt);
    expect([...times].sort().reverse()).toEqual(times);
    expect(new Set(merged.map((e) => e.sourceId)).size).toBeGreaterThanOrEqual(6);
  });
  it('mock provider list/detail round-trip', async () => {
    const jira = getProvider('jira')!;
    const list = await jira.list({});
    const d = await jira.detail(list[0].id);
    expect(d.entry.id).toBe(list[0].id);
    expect(d.bodyPreview).toBeTruthy();
  });
});
```

(The `vi.mock` blocks above are REQUIRED, not optional — without them the import of `providers` explodes on `getProjectId()`. With local mocked to `[]`, the `>10 entries` assertion is carried entirely by the 6 mock fixtures — size the fixtures accordingly.)

- [ ] **Step 2: Run tests, verify FAIL** — `pnpm test:unit -- providers` → module-not-found failures.

- [ ] **Step 3: Implement** — order: `types.ts` → `icons.tsx` (7 hand-written inline SVGs, official brand colors as fills: Drive #34A853-triangle style multi-color simplified OK, S3 #E25444, Confluence #1868DB, Jira #0052CC, Gmail #EA4335, Slack 4-color pinwheel or #611F69 mono, Local uses the design-system folder icon in indigo; all inside an 18×18 viewBox slot component) → `fixtures/*.ts` (each exports `entries: SourceEntry[]` (8-15개, natural Korean business content: 회의록/계약/배포/이슈/스레드; spread modifiedAt over the last 2 weeks with FIXED ISO timestamps — no Date.now) and `details: Record<string, SourceDetail>`; threads' bodyPreview = `발신자: 내용\n발신자: 내용` lines. **gdrive and s3 fixtures MUST include folder entries** (`kind:'folder'` with `path` values like `/reports`, and file entries whose `path` starts with those folders) so FileBrowser folder navigation works over mock data too — confluence/jira/gmail/slack are flat lists where `path` is omitted and `ctx.path` is ignored) → `providers/mock.ts`:

```ts
export function makeMockProvider(
  cfg: Pick<SourceProvider, 'id' | 'nameKey' | 'category' | 'kind' | 'count'>,
  fixture: { entries: SourceEntry[]; details: Record<string, SourceDetail> },
): SourceProvider {
  const delay = <T,>(v: T) => new Promise<T>((r) => setTimeout(() => r(v), 250)); // simulate latency for loading states
  return {
    ...cfg,
    connected: true,
    attachable: false,
    list: (ctx) => delay(fixture.entries.filter((e) => (ctx.path ?? '') === '' ? true : e.path?.startsWith(ctx.path!))),
    recent: () => delay([...fixture.entries].sort((a, b) => b.modifiedAt.localeCompare(a.modifiedAt)).slice(0, 20)),
    detail: (id) => {
      const d = fixture.details[id];
      return d ? delay(d) : Promise.reject(new Error(`no detail for ${id}`));
    },
  };
}
```

→ `providers/local.ts` (adapt `listDirectory`/`getFileBlob` from `@/api/workspace`; map `FileStat` → `SourceEntry` with these EXACT rules:
  - `kind: stat.type === 'directory' ? 'folder' : 'file'` — webdav's type value is `'directory'`, NOT `'folder'`; checking `=== 'folder'` would silently make every folder a file
  - `path`: preserve the leading-slash normalization the current route uses (`stat.filename.startsWith('/') ? stat.filename : '/' + stat.filename`) — webdav servers vary, and the Task-5 attach concatenation `'/root/shared' + entry.path` breaks without it (`/root/sharedreports/…`)
  - `modifiedAt`: `new Date(stat.lastmod).toISOString()` (lastmod is an RFC string)
  `recent()` = list root, files only, sort desc, slice 20; `detail(id)` = entry meta only (blob fetching stays in DetailPanel/lightbox); `count: null`) → `providers.ts` registry + `allRecent` (Promise.all + flat + sort desc by modifiedAt; a failed provider contributes `[]` via `.catch(() => [])` — log console.warn).

- [ ] **Step 4: Run tests → PASS**; `pnpm lint` clean.
- [ ] **Step 5: Commit** — `feat(app_v2): workspace source provider layer with mock fixtures`

---

### Task 2: Shell — SourceRail, routes, UnifiedList ('전체', row spec B)

**Files:**
- Create: `app_v2/src/workspace/components/WorkspaceShell.tsx`, `SourceRail.tsx`, `UnifiedList.tsx`
- Rewrite: `app_v2/src/routes/workspace.tsx` (leaf: `<WorkspaceShell>` + UnifiedList); Create: `app_v2/src/routes/workspace.$sourceId.tsx` (sibling leaf: `<WorkspaceShell>` + archetype view — see ROUTING CONTRACT in File Structure)
- Modify: `app_v2/src/styles/globals.css` (workspace hub classes), locales `en/ko files.json`
- Test: `app_v2/src/workspace/__tests__/unified.test.tsx`

**Interfaces:**
- Consumes: `PROVIDERS`, `allRecent`, `SourceIcon` (Task 1).
- Produces: shell contract for Tasks 3-4 — `WorkspaceShell({ activeSourceId, children })` renders `[SourceRail | children(main view) | DetailPanel slot]` and owns `selected: SourceEntry | null` COMPONENT state (per route instance — resets on route change by design); it passes `onSelect(entry)` down to the main view via a render-prop or context (pick render-prop: `children` is `(onSelect) => JSX` OR simpler: shell exports `useWorkspaceSelection()` context — implementer's choice, document in code). '전체' lives at `/workspace`; `/workspace/$sourceId` validates via `getProvider`.

**Layout/visual requirements (from approved mockups):**
- Rail ~190px: top item **전체** (accent style when active) → category labels (파일 / 문서 · 티켓 / 메시지, uppercase small) → source rows (SourceIcon + name + right count badge) → bottom dashed **＋ 소스 추가** row opening a small mock dialog (design-system dialog style; body: i18n "커넥터는 준비 중이에요" + planned-source list; Close).
- Unified row (B): `18px SourceIcon` + `76px fixed source-name column (muted, truncate)` + `title (ellipsis, flex-1)` + `meta (muted, nowrap, right)`. Same row height; hover bg; click → onSelect.
- Toolbar: search input (client filter over title+subtitle, i18n placeholder "모든 소스에서 검색…") + 업로드 button only when viewing local or 전체 (reuses existing upload logic → refactor the current upload handler out of old workspace.tsx into the shell).
- Loading (250ms mock latency) shows the design-system muted loading row; error state per provider row silently omitted from merge (already handled in allRecent).

- [ ] **Step 1: Write failing tests** — `unified.test.tsx`: renders UnifiedList with `entries` prop of 3 fixture entries from different sources → asserts (a) each row shows source name in the fixed column (query by text), (b) rows ordered as given, (c) typing in the search input filters by title, (d) click fires onSelect with the entry. Mock i18n `t:(k)=>k`.
- [ ] **Step 2: Run → FAIL** (component missing).
- [ ] **Step 3: Implement** — components + routes + CSS (`.cw-ws-*` prefix; reuse `--cw-*` tokens; the rail mirrors sidebar visual family). Route `/workspace` loads `allRecent` via TanStack Query `['ws','all']`; `/workspace/$sourceId` validates id via `getProvider` (unknown → redirect `/workspace`). i18n keys to BOTH locales: `workspace.all`, `workspace.cat.files|docs|messages`, `workspace.src.local|gdrive|s3|confluence|jira|gmail|slack`, `workspace.addSource`, `workspace.addSourceBody`, `workspace.searchAll`.
- [ ] **Step 4: Tests PASS + `pnpm lint && pnpm test:unit && pnpm build`** — NOTE: no existing test covers `routes/workspace.tsx` (the only workspace test is the webdav API wrapper `src/api/__tests__/workspace.test.ts`, untouched by this task). Rewriting the route breaks nothing; `unified.test.tsx` is net-new coverage. Do NOT hunt for a route test to adapt — there isn't one.
- [ ] **Step 5: Commit** — `feat(app_v2): workspace hub shell with source rail and unified view`

---

### Task 3: Source views — FileBrowserView / ItemListView / ThreadListView

**Files:**
- Create: `app_v2/src/workspace/components/FileBrowserView.tsx`, `ItemListView.tsx`, `ThreadListView.tsx`
- Modify: `workspace.$sourceId.tsx` (dispatch view by `provider.kind`), locales
- Test: `app_v2/src/workspace/__tests__/views.test.tsx`

**Interfaces:**
- Consumes: `provider.list(ctx)`, `SourceEntry`, shell `onSelect`.
- Produces: each view = `({ provider, onSelect }) => JSX` — self-fetching via TanStack Query `['ws', provider.id, path]`.

**Requirements:**
- FileBrowser: breadcrumb (root = provider name; folder click descends by setting `path`; crumb click ascends), rows: type icon + name + size + modified (this replaces the old flat table for local — keep 업로드 wired for local via shell toolbar; folders sort first).
- ItemList: rows = title + status/label chip (`subtitle` first token as chip, rest as text) + modified. Chip uses design-system badge style.
- ThreadList: rows = bold sender/channel (subtitle) + title one-liner + time.
- All three: loading state, empty state (`EmptyState`), click → onSelect.

- [ ] **Step 1: Failing tests** — views.test.tsx: (a) FileBrowser with mocked provider.list: folder click calls list with descended path & breadcrumb grows; (b) ItemList renders chip from subtitle; (c) ThreadList renders sender bold + title. Per-test QueryClient retry:false.
- [ ] **Step 2: Run → FAIL.**
- [ ] **Step 3: Implement** (dispatch in `$sourceId` route: `kind==='files'?FileBrowserView: kind==='items'?ItemListView:ThreadListView`).
- [ ] **Step 4: PASS + lint/build.**
- [ ] **Step 5: Commit** — `feat(app_v2): archetype views for files, items, and threads`

---

### Task 4: DetailPanel + lightbox integration

**Files:**
- Create: `app_v2/src/workspace/components/DetailPanel.tsx`
- Modify: shell (right slot renders panel when an entry is selected; ESC/✕ clears), locales
- Test: `app_v2/src/workspace/__tests__/detail.test.tsx`

**Interfaces:**
- Consumes: `getProvider(entry.sourceId).detail(entry.id)`, `FilePreviewModal` (existing, props `{path, name, onClose}`), `deleteEntry`/blob download from `@/api/workspace` (local only).
- Produces: `onAttach(entry)` callback prop — Task 5 wires it; until then the button shows the mock toast for ALL sources (local flips to real in Task 5).

**Requirements:**
- Panel (~260px, right border, `--cw-paper-2` bg): title, source badge (icon+name) + meta line (size/date), body: files → mini preview box that opens `FilePreviewModal` on click (local only; mock files show a static placeholder box); items/threads → `bodyPreview` (threads: split lines into speaker-labeled bubbles reusing chat bubble classes).
- Actions stack: primary **새 대화에서 쓰기** (always visible; disabled-looking never — mock sources show toast on click per spec); secondary for local files: 다운로드(blob anchor), 삭제(ConfirmDialog → deleteEntry → invalidate queries + clear panel); mock sources: 원본 열기 (externalUrl, `target="_blank"`, `#` ok).
- i18n keys: `workspace.detail.openChat`, `.download`, `.delete`, `.openOriginal`, `.mockAttachToast`.

- [ ] **Step 1: Failing tests** — detail.test.tsx: (a) renders meta from provider.detail (mocked); (b) thread bodyPreview renders one bubble per line; (c) local file delete flow calls deleteEntry after confirm; (d) mock source attach click renders the toast text.
- [ ] **Step 2: FAIL → Step 3: Implement → Step 4: PASS + lint/build.**
- [ ] **Step 5: Commit** — `feat(app_v2): workspace detail panel with preview and actions`

---

### Task 5: Chat bridge — pendingAttachment store + home composer chip

**Files:**
- Create: `app_v2/src/stores/pendingAttachment.ts`
- Modify: `app_v2/src/routes/index.tsx` (HomePage: chip + prefix injection), `DetailPanel.tsx` (local attach = real), locales `session.json` en+ko
- NOT modified: `components/chat/ProjectHomeComposer.tsx` (sealed; see Requirements)
- Test: `app_v2/src/stores/__tests__/pendingAttachment.test.ts`, extend home-composer test

**Interfaces:**

```ts
// stores/pendingAttachment.ts — module singleton like stores/project.ts
export interface PendingAttachment { name: string; sharedPath: string }  // sharedPath e.g. "/root/shared/reports/q2.pdf"
export function setPendingAttachment(a: PendingAttachment): void;
export function takePendingAttachment(): PendingAttachment | null;      // read-and-clear
```

**Requirements:**
- DetailPanel local attach: `setPendingAttachment({ name: entry.title, sharedPath: '/root/shared' + entry.path })` → `navigate({ to: '/' })`.
- **All home-side changes live in `HomePage` (`routes/index.tsx`) — `ProjectHomeComposer` is NOT modified.** It is a sealed form with no attachment prop (its header comment says attachment machinery was deliberately stripped); `agentType` state and `handleSubmit` already live in `HomePage`. Concretely:
  - Consuming the store — **StrictMode-safe pattern REQUIRED** (`main.tsx` wraps the app in `<StrictMode>`, which double-invokes state initializers in dev; the read-and-clear store would be consumed by the discarded first call and the chip would silently never render — exactly where Task 6 verifies). Do NOT write `useState(() => takePendingAttachment())`. Instead:
    ```tsx
    const [attachment, setAttachment] = useState<PendingAttachment | null>(null);
    useEffect(() => {
      // Guarded read-and-clear: under StrictMode the second effect run gets
      // null from the store and is a no-op, so the chip survives.
      const a = takePendingAttachment();
      if (a) setAttachment(a);
    }, []);
    ```
  - Chip render: inside the `cw-agent-composer-wrap` block, immediately ABOVE `<ProjectHomeComposer>`: file icon + `attachment.name` + ✕ button (✕ clears the state). Below/beside it the hint `t('home.attachmentHint')`.
  - Prefix injection: in `HomePage.handleSubmit`, before `sendMessage(session.id, text)`: `const finalText = attachment ? '[첨부 파일: ' + attachment.sharedPath + ']\n' + text : text;` — clear the chip state after successful send.
  - Agent preset: `agentType` already defaults to `'coworker'` in HomePage — the only requirement is do NOT override the user's choice if they switch after the chip appears (i.e., no effect that force-sets agentType).
- Mock-source attach stays the toast (from Task 4).
- i18n: add `home.attachmentHint` to **`session.json`** (en+ko) — the home surface uses `useTranslation('session')`, NOT the files namespace. ko "이 파일을 참조해 대화를 시작해요" / en "Start the chat with this file attached".

- [ ] **Step 1: Failing tests** — store: set→take returns value once then null. Home test: with pending attachment set, renders chip; send calls sendMessage with text starting `[첨부 파일: /root/shared/…]`; chip ✕ removes and send omits the prefix.
- [ ] **Step 2: FAIL → Step 3: Implement → Step 4: PASS + lint/build (full suite).**
- [ ] **Step 5: Commit** — `feat(app_v2): attach workspace files into new chats via shared mount path`

---

### Task 6: Live E2E verification (spec success criteria)

**Files:** none (verification only; screenshots to repo root of main worktree or scratchpad).

- [ ] **Step 1:** Ensure backend :8080 (binary direct, `BIND_ADDR=127.0.0.1:8080`, env from repo `.env`) and app_v2 dev :4210 are running.
- [ ] **Step 2:** Browser (playwright MCP): `/workspace` → '전체' shows ≥6 sources merged newest-first, uniform rows (icon + 76px source col). Screenshot.
- [ ] **Step 3:** Rail: enter each archetype once (로컬 → folders real; Jira → chips; Slack → threads). Screenshot each.
- [ ] **Step 4:** Local file detail → 미리보기 → lightbox; 다운로드 intact.
- [ ] **Step 5:** Local file → 새 대화에서 쓰기 → home chip → send "이 파일 내용을 한 줄로 요약해줘" → coworker reads `/root/shared/...` and answers from file content. Screenshot.
- [ ] **Step 6:** Mock source attach → toast. '＋ 소스 추가' → dialog.
- [ ] **Step 7:** Record results in `.superpowers/sdd/progress.md`; report BLOCKED items honestly (sandbox boot depends on machine state).

---

## Self-Review (done at write time)

- Spec coverage: IA(rail/전체/아키타입/패널/라이트박스)→T2-4; row spec B→T2; provider 추상화→T1; 채팅 연결→T5; 목업 토스트→T4/T5; +소스 추가 목업→T2; 브랜드 아이콘→T1; 성공 기준→T6. 제외 범위(커넥터/백엔드/서버검색/페이지네이션) 플랜에 부재 — 일치.
- Placeholder scan: clean (no TBD; every step has concrete content or exact behavioral assertions).
- Type consistency: `SourceEntry.path` webdav-relative with leading slash — LocalProvider(T1), FileBrowser ctx.path(T3), attach `'/root/shared' + entry.path`(T5) all consistent. `onSelect(entry)`(T2)⇄views(T3)⇄panel(T4) consistent. `FilePreviewModal {path,name,onClose}` matches existing component.
