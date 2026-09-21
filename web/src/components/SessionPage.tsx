// One session in full: what it is, how it is doing, and every action grouped by what it changes.
import type { ReactNode } from 'react';
import type { Api, Session, SessionActions, User } from '../types.ts';
import { useState } from 'react';
import { ExternalLink, Loader2, Play, RotateCw, SlidersHorizontal, Square, Trash2, Wrench } from 'lucide-react';
import { DataList, Disclosure, EmptyState, Loading, PageHeader, Section } from './ui.tsx';
import { Confirm } from './Dialog.tsx';
import { Link, navigate } from '../router.tsx';
import { Preview } from './Preview.tsx';
import { Logs } from './Logs.tsx';
import { Sharing } from './Sharing.tsx';
import { StatusBadge, busyState, distribution, elapsed, installLabel, pendingNote, screenLabel, settled, stageLabel } from './session.tsx';

/// The actions a section ends with.
const Actions = ({ children }: { children: ReactNode }) => <div className="flex flex-wrap items-center gap-2 border-t border-line px-4 py-3">{children}</div>;

export function SessionPage({ id, sessions, loaded, user, api, busy, now, onOpen, onAction }: SessionActions & { id: string; sessions: Session[]; loaded: boolean; user: User; api: Api; now: () => number }) {
  const [confirming, setConfirming] = useState('');
  const s = sessions.find(item => item.id === id);
  if (!s)
    return loaded ? (
      <EmptyState className="mt-10" title="Session Unavailable" description="It may have been destroyed, or your access to it may have been revoked.">
        <Link to="/" className="btn btn-outline btn-sm mt-1">
          Back to Sessions
        </Link>
      </EmptyState>
    ) : (
      <Loading>Loading session…</Loading>
    );
  const manages = s.access_role === 'manager';
  const working = busy[s.id];
  const label = installLabel(s);
  const stoppable = manages && s.status !== 'cancelled';
  return (
    <>
      <PageHeader
        back={{ to: '/', label: 'Sessions' }}
        title={s.name}
        badge={<StatusBadge session={s} />}
        action={
          <>
            {stoppable && (
              <button
                type="button"
                className="btn btn-outline btn-lg"
                disabled={working}
                onClick={() => onAction(s, s.status === 'stopped' ? 'start' : 'stop')}
              >
                {s.status === 'stopped' ? <Play className="size-4" strokeWidth={1.75} /> : <Square className="size-3.5" strokeWidth={2} />}
                {s.status === 'stopped' ? 'Start' : 'Stop'}
              </button>
            )}
            {/* Relaunching is what applies settings saved for the next launch, so it appears with them. */}
            {manages && s.status === 'running' && s.settings_pending && (
              <button type="button" className="btn btn-outline btn-lg" disabled={working} onClick={() => onAction(s, 'relaunch')}>
                <RotateCw className="size-4" strokeWidth={1.75} />
                Relaunch
              </button>
            )}
            {s.status === 'running' && (
              <button type="button" className="btn btn-primary btn-lg" disabled={working} onClick={() => onOpen(s)}>
                <ExternalLink className="size-4" strokeWidth={1.75} />
                Open Desktop
              </button>
            )}
          </>
        }
      />

      <div className="mt-6 flex flex-col gap-2 empty:mt-0">
        {busyState(s) && (
          <p role="status" className="callout flex items-center gap-2.5 border-warn/30 bg-warn/10 text-warn">
            <Loader2 className="size-3.5 shrink-0 animate-spin" />
            {stageLabel(s)}…
          </p>
        )}
        {s.error && <p role="alert" className="callout callout-bad [overflow-wrap:anywhere]">{s.error}</p>}
        {s.settings_pending && (
          <p role="status" className="callout callout-info">
            Settings pending · {pendingNote(s)}
          </p>
        )}
        {s.repair_available && (
          <p className="callout callout-info">Elsewhere installation is incomplete. Install the preferred version before starting.</p>
        )}
        {s.version_error && <p className="callout callout-info">{s.version_error}</p>}
      </div>

      <div className="mt-6 grid grid-cols-1 items-start gap-5 lg:grid-cols-[minmax(0,7fr)_minmax(0,5fr)]">
        <div className="card overflow-hidden">
          {/* The desktop keeps its shape whether or not it is up, so the page does not move when it comes back. */}
          <Preview session={s} api={api} interval={3000} className="aspect-video" glyph="size-9" />
        </div>

        <div className="flex min-w-0 flex-col gap-5">
          {s.status === 'running' && manages && (s.started_ms > 0 || s.port > 0) && (
            <Section title="Runtime">
              <DataList
                items={[
                  s.started_ms > 0 && { label: 'Launched', value: `${elapsed(s.started_ms, now)} ago` },
                  s.port > 0 && { label: 'Port', value: <code className="font-mono">{s.port}</code> },
                ]}
              />
            </Section>
          )}

          <Section
            title="Configuration"
            action={
              manages &&
              (settled(s) && !working ? (
                <Link to={`/sessions/${s.id}/settings`} className="btn btn-outline btn-sm">
                  <SlidersHorizontal className="size-3.5" strokeWidth={1.75} />
                  Edit Settings
                </Link>
              ) : (
                // Settings are only saved for a settled session, so the way in closes with them.
                <button type="button" className="btn btn-outline btn-sm" disabled>
                  <SlidersHorizontal className="size-3.5" strokeWidth={1.75} />
                  Edit Settings
                </button>
              ))
            }
          >
            <DataList
              items={[
                { label: 'Distribution', value: distribution(s) },
                manages && { label: 'Packages', value: s.packages.length ? s.packages.join(', ') : <span className="text-ink-4">None</span> },
                manages && { label: 'Screen size', value: screenLabel(s) },
                manages && { label: 'GPU access', value: s.gpu_access ? 'On' : 'Off' },
                manages && s.gpu && { label: 'GPU', value: `${s.gpu.driver} · ${s.gpu.id}` },
                manages && { label: 'Software video encoding', value: s.software_encoding ? 'On' : 'Off' },
                manages && { label: 'Kiosk mode', value: s.kiosk ? 'On' : 'Off' },
                manages && {
                  label: 'Startup command',
                  value: s.startup_command ? <code className="font-mono">{s.startup_command}</code> : <span className="text-ink-4">None</span>,
                },
                manages &&
                  s.docker_args.length > 0 && {
                    label: 'Docker options',
                    value: <code className="font-mono whitespace-pre-wrap">{s.docker_args.join('\n')}</code>,
                  },
              ]}
            />
          </Section>

          {manages && (
            <>
              <Section title="Elsewhere Version">
                <DataList
                  items={[
                    { label: 'Installed', value: s.installed_version || <span className="text-ink-4">Unavailable</span> },
                    { label: 'Preferred', value: s.expected_version },
                  ]}
                />
                <Actions>
                  <button type="button" className="btn btn-outline btn-sm" disabled={working || !settled(s)} onClick={() => setConfirming('install')}>
                    <Wrench className="size-3.5" strokeWidth={1.75} />
                    {label}
                  </button>
                </Actions>
              </Section>

            </>
          )}
        </div>
      </div>

      <div className="mt-5 flex flex-col gap-5 empty:mt-0">
        {manages && <Logs api={api} session={s} />}
        {user.role === 'administrator' && <Sharing api={api} machine={s} />}
        {manages && (
          <Disclosure label="Danger Zone" danger>
            <div className="flex flex-wrap items-center gap-2 px-4 py-3">
              <button type="button" className="btn btn-danger btn-sm" disabled={working} onClick={() => setConfirming('destroy')}>
                <Trash2 className="size-3.5" strokeWidth={1.75} />
                Destroy Session
              </button>
            </div>
          </Disclosure>
        )}
      </div>

      {confirming === 'install' && manages && (
        <Confirm
          title={`${label} ${s.name}`}
          label={label}
          disabled={working || !settled(s)}
          close={() => setConfirming('')}
          confirm={() => onAction(s, 'upgrade')}
        >
          This will close all running applications and stop the session. Unsaved changes may be lost.
        </Confirm>
      )}
      {confirming === 'destroy' && manages && (
        <Confirm
          title={`Destroy ${s.name}`}
          label="Destroy"
          tone="danger"
          disabled={working}
          close={() => setConfirming('')}
          confirm={() => {
            // The workspace is where a destroyed session belongs, and where a refusal is reported.
            navigate('/');
            onAction(s, 'destroy');
          }}
        >
          This permanently deletes the session and everything in its home directory. It cannot be undone.
        </Confirm>
      )}
    </>
  );
}
