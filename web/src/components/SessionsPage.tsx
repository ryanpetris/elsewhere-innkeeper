// The workspace: every session the signed-in account may reach, as a grid of previews or a dense list.
import type { Api, Layout, Session, SessionActions } from '../types.ts';
import { useState } from 'react';
import { ChevronRight, ExternalLink, Grid2X2, List, Loader2, Monitor, Play, Plus, Search } from 'lucide-react';
import { EmptyState, IconButton, Loading, PageHeader } from './ui.tsx';
import { Link } from '../router.tsx';
import { Preview } from './Preview.tsx';
import { StatusBadge, busyState, distribution, stageLabel } from './session.tsx';

type CardProps = SessionActions & { s: Session; api: Api };

const FILTERS = [
  ['all', 'All States'],
  ['running', 'Running'],
  ['stopped', 'Stopped'],
  ['preparing', 'Preparing'],
  ['upgrading', 'Upgrading'],
  ['failed', 'Failed'],
  ['cancelled', 'Cancelled'],
];

export function SessionsPage({ sessions, loaded, api, layout, setLayout, busy, onOpen, onAction }: SessionActions & { sessions: Session[]; loaded: boolean; api: Api; layout: Layout; setLayout: (layout: Layout) => void }) {
  const [search, setSearch] = useState('');
  const [state, setState] = useState('all');
  const query = search.trim().toLowerCase();
  const shown = sessions.filter(
    s =>
      (state === 'all' || s.status === state) &&
      (!query || s.name.toLowerCase().includes(query) || (s.packages ?? []).some(p => p.toLowerCase().includes(query))),
  );
  const running = sessions.filter(s => s.status === 'running').length;
  return (
    <>
      <PageHeader
        title="Sessions"
        action={
          <Link to="/sessions/new" className="btn btn-primary btn-lg">
            <Plus className="size-4" strokeWidth={2} />
            New Session
          </Link>
        }
      />
      {!loaded ? (
        <div className="mt-8">
          <Loading>Loading sessions…</Loading>
        </div>
      ) : !sessions.length ? (
        <EmptyState
          className="mt-8"
          icon={Monitor}
          title="No Sessions Yet"
          description="Pick a distribution, add the packages you want, and Innkeeper builds the machine."
        >
          <Link to="/sessions/new" className="btn btn-primary btn-sm mt-1">
            Create a Session
          </Link>
        </EmptyState>
      ) : (
        <>
          <div className="mt-7 flex flex-wrap items-center gap-x-3 gap-y-2 border-b border-line pb-3">
            <div className="relative min-w-0 basis-full sm:max-w-64 sm:flex-1 sm:basis-auto">
              <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-ink-4" strokeWidth={1.75} />
              <input
                type="search"
                className="input pl-8"
                aria-label="Filter sessions"
                placeholder="Filter by name or package"
                value={search}
                onChange={e => setSearch(e.target.value)}
              />
            </div>
            <select className="select select-md" aria-label="Filter by state" value={state} onChange={e => setState(e.target.value)}>
              {FILTERS.map(([value, label]) => (
                <option key={value} value={value}>
                  {label}
                </option>
              ))}
            </select>
            <p className="ml-auto text-xs whitespace-nowrap text-ink-3">
              <span className="font-medium text-ink">{running} running</span> / {sessions.length} total
            </p>
            <div className="flex items-center gap-0.5 rounded-lg border border-line bg-surface-2 p-0.5">
              <IconButton icon={Grid2X2} label="Grid View" active={layout === 'grid'} onClick={() => setLayout('grid')} />
              <IconButton icon={List} label="List View" active={layout === 'list'} onClick={() => setLayout('list')} />
            </div>
          </div>
          {!shown.length ? (
            <EmptyState className="mt-5" icon={Search} title="No Matches" />
          ) : layout === 'grid' ? (
            <div className="mt-5 grid grid-cols-[repeat(auto-fill,minmax(18rem,1fr))] gap-4">
              {shown.map(s => (
                <SessionTile key={s.id} s={s} api={api} busy={busy} onOpen={onOpen} onAction={onAction} />
              ))}
            </div>
          ) : (
            <div className="card mt-5 overflow-hidden">
              <div className="hidden items-center gap-3 border-b border-line bg-surface px-3 py-2 lg:flex">
                <span className="w-28 shrink-0" />
                <span className="eyebrow min-w-0 flex-1">Session</span>
                <span className="eyebrow w-28 shrink-0">Elsewhere</span>
                <span className="eyebrow w-24 shrink-0">State</span>
                <span className="w-[6.5rem] shrink-0" />
                <span className="w-4 shrink-0" />
              </div>
              {shown.map(s => (
                <SessionRow key={s.id} s={s} api={api} busy={busy} onOpen={onOpen} onAction={onAction} />
              ))}
            </div>
          )}
        </>
      )}
    </>
  );
}

/// The one action worth reaching for without opening the session.
function QuickAction({ s, busy, onOpen, onAction }: Omit<CardProps, 'api'>) {
  if (s.status === 'running')
    return (
      <button type="button" className="btn btn-primary btn-sm relative" disabled={busy[s.id]} onClick={() => onOpen(s)}>
        <ExternalLink className="size-3.5" strokeWidth={1.75} />
        Open
      </button>
    );
  if (s.status === 'stopped' && s.access_role === 'manager')
    return (
      <button type="button" className="btn btn-outline btn-sm relative" disabled={busy[s.id]} onClick={() => onAction(s, 'start')}>
        <Play className="size-3.5" strokeWidth={1.75} />
        Start
      </button>
    );
  return null;
}

/// The step a session announces while it is working, in place of its package list.
const Stage = ({ s }: { s: Session }) => (
  <p role="status" className="flex items-center gap-1.5 truncate text-xs text-warn">
    <Loader2 className="size-3 shrink-0 animate-spin" />
    {stageLabel(s)}…
  </p>
);

// `session` is the marker the browser checks select cards by; it carries no styling.
function SessionTile({ s, api, busy, onOpen, onAction }: CardProps) {
  return (
    <article className="session card group relative flex flex-col overflow-hidden transition-colors hover:border-line-2 [overflow-wrap:anywhere]">
      <Preview session={s} api={api} className="aspect-video border-b border-line" />
      <div className="flex min-w-0 flex-1 flex-col p-3.5">
        <div className="flex items-start gap-2">
          <h2 className="min-w-0 flex-1 text-sm font-semibold text-ink">
            <Link to={`/sessions/${s.id}`} className="row-link transition-colors group-hover:text-accent-2">
              {s.name}
            </Link>
          </h2>
          <StatusBadge session={s} />
        </div>
        {busyState(s) ? (
          <div className="mt-1.5">
            <Stage s={s} />
          </div>
        ) : (
          <p className="mt-1.5 line-clamp-2 text-xs text-ink-3">
            {distribution(s)}
            {s.packages && s.packages.length > 0 && <span className="text-ink-4"> · {s.packages.join(', ')}</span>}
          </p>
        )}
        <div className="mt-4 flex min-h-7 items-center gap-2 pt-0.5">
          <QuickAction s={s} busy={busy} onOpen={onOpen} onAction={onAction} />
          <span className="ml-auto inline-flex shrink-0 items-center gap-0.5 text-xs text-ink-4 transition-colors group-hover:text-ink-2">
            Details
            <ChevronRight className="size-3.5 transition-transform group-hover:translate-x-0.5" strokeWidth={2} />
          </span>
        </div>
      </div>
    </article>
  );
}

function SessionRow({ s, api, busy, onOpen, onAction }: CardProps) {
  return (
    <article className="session group relative flex items-center gap-3 border-b border-line px-3 py-2.5 transition-colors last:border-0 hover:bg-surface-3 [overflow-wrap:anywhere]">
      <Preview session={s} api={api} className="aspect-video w-20 shrink-0 rounded border border-line sm:w-28" glyph="size-4" label={false} />
      <div className="min-w-0 flex-1">
        <h2 className="truncate text-sm font-semibold text-ink">
          <Link to={`/sessions/${s.id}`} className="row-link transition-colors group-hover:text-accent-2">
            {s.name}
          </Link>
        </h2>
        {busyState(s) ? (
          <Stage s={s} />
        ) : (
          <p className="mt-0.5 truncate text-xs text-ink-4">
            {distribution(s)}
            {s.packages && s.packages.length > 0 && <span> · {s.packages.join(', ')}</span>}
          </p>
        )}
      </div>
      <p className="hidden w-28 shrink-0 truncate text-xs text-ink-3 lg:block">{s.installed_version || '—'}</p>
      <div className="w-auto shrink-0 lg:w-24">
        <StatusBadge session={s} />
      </div>
      <div className="flex shrink-0 justify-end lg:w-[6.5rem]">
        <QuickAction s={s} busy={busy} onOpen={onOpen} onAction={onAction} />
      </div>
      <ChevronRight className="size-4 shrink-0 text-ink-4 transition-transform group-hover:translate-x-0.5" strokeWidth={2} />
    </article>
  );
}
