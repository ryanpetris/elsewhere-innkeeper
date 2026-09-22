// Creating a session, and changing the settings of one that exists, each on a page of its own.
import type { ReactNode } from 'react';
import type { Api, Gpu, Session, SessionSettings, User } from '../types.ts';
import { errorMessage, readJson } from '../types.ts';
import { useState } from 'react';
import { EmptyState, Loading, PageHeader } from './ui.tsx';
import { Link, leave } from '../router.tsx';
import { SessionForm } from './SessionForm.tsx';
import { pendingNote, settled } from './session.tsx';

/// Forms sit in a single column so the eye has one place to go.
const Column = ({ children }: { children: ReactNode }) => <div className="mx-auto flex w-full max-w-3xl flex-col gap-7">{children}</div>;

export function NewSessionPage({ api, user, refresh, loaded, gpus, gpuErrors }: { api: Api; user: User; refresh: () => Promise<void>; loaded: boolean; gpus: Gpu[]; gpuErrors: string[] }) {
  const [error, setError] = useState('');
  const here = '/sessions/new';
  if (!loaded) return <Loading>Loading session options…</Loading>;
  return (
    <Column>
      <PageHeader back={{ to: '/', label: 'Sessions' }} title="New Session" />
      <SessionForm
        gpus={gpus}
        gpuErrors={gpuErrors}
        administrator={user.role === 'administrator'}
        error={error}
        cancelTo="/"
        submit={async profile => {
          setError('');
          try {
            const created = await readJson<{ id: string }>(await api('/sessions', { method: 'POST', body: JSON.stringify(profile) }));
            await refresh();
            leave(here, `/sessions/${created.id}`);
          } catch (e) {
            setError(errorMessage(e));
          }
        }}
      />
    </Column>
  );
}

export function SessionSettingsPage({ id, sessions, loaded, api, refresh, user, gpus, gpuErrors }: { id: string; sessions: Session[]; loaded: boolean; api: Api; refresh: () => Promise<void>; user: User; gpus: Gpu[]; gpuErrors: string[] }) {
  const [error, setError] = useState('');
  const s = sessions.find(item => item.id === id);
  if (!s || s.access_role !== 'manager')
    return loaded ? (
      <EmptyState className="mt-10" title="Settings Unavailable" description="The session may have been destroyed, or you may not manage it.">
        <Link to="/" className="btn btn-outline btn-sm mt-1">
          Back to Sessions
        </Link>
      </EmptyState>
    ) : (
      <Loading>Loading settings…</Loading>
    );
  const back = `/sessions/${s.id}`;
  const here = `${back}/settings`;
  return (
    <Column>
      <PageHeader back={{ to: back, label: s.name }} title="Edit Settings">
        {s.settings_pending && (
          <p role="status" className="callout callout-info mt-4">
            Settings pending · {pendingNote(s)}
          </p>
        )}
        {!settled(s) && (
          <p role="status" className="callout callout-info mt-4">
            This session is {s.status}.
          </p>
        )}
      </PageHeader>
      <SessionForm
        initial={s}
        administrator={user.role === 'administrator'}
        gpus={gpus}
        gpuErrors={gpuErrors}
        error={error}
        cancelTo={back}
        note="The name applies immediately. Other settings apply on Start or Relaunch. GPU, package and Docker option changes recreate the container while preserving installed software and files."
        blocked={settled(s) ? '' : 'Settings can be saved once the session is running or stopped.'}
        submit={async profile => {
          setError('');
          try {
            await api(`/sessions/${s.id}/settings`, {
              method: 'PUT',
              body: JSON.stringify({
                name: profile.name,
                packages: profile.packages,
                docker_args: profile.docker_args,
                gpu_access: profile.gpu_access,
                gpu_id: profile.gpu_id,
                screen_size: profile.screen_size,
                kiosk: profile.kiosk,
                software_encoding: profile.software_encoding,
                startup_command: profile.startup_command,
              } satisfies SessionSettings),
            });
            await refresh();
            leave(here, back);
          } catch (e) {
            setError(errorMessage(e));
          }
        }}
      />
    </Column>
  );
}
