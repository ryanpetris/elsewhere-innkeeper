// Who else may reach this machine, and how far their access goes.
import type { Api, Assignment, Session, User } from '../types.ts';
import { errorMessage, readJson } from '../types.ts';
import { useEffect, useState } from 'react';
import { Alert, Badge, Loading, Section } from './ui.tsx';

export function Sharing({ api, machine }: { api: Api; machine: Session }) {
  const [users, setUsers] = useState<User[] | null>(null);
  const [assignments, setAssignments] = useState<Assignment[]>([]);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  async function reload() {
    const [u, a] = await Promise.all([api('/users').then(readJson<{ users: User[] }>), api(`/sessions/${machine.id}/access`).then(readJson<{ assignments: Assignment[] }>)]);
    setUsers(u.users);
    setAssignments(a.assignments);
  }
  // Assignments another administrator changes appear on return to the tab.
  useEffect(() => {
    const refresh = () => {
      if (!document.hidden) reload().catch((e: unknown) => setError(errorMessage(e)));
    };
    refresh();
    addEventListener('focus', refresh);
    document.addEventListener('visibilitychange', refresh);
    return () => {
      removeEventListener('focus', refresh);
      document.removeEventListener('visibilitychange', refresh);
    };
  }, [machine.id]);
  return (
    <Section title="People with Access">
      <div className="flex flex-col gap-2 p-4">
        {error && <Alert>{error}</Alert>}
        {users === null && !error && <Loading>Loading accounts…</Loading>}
        {(users ?? []).map(user => (
          <label
            key={user.id}
            className="flex flex-col gap-3 rounded-lg border border-line bg-surface px-3 py-2 transition-colors hover:border-line-2 sm:flex-row sm:items-center"
          >
            <span className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
              <span className="text-sm text-ink [overflow-wrap:anywhere]">{user.display_name}</span>
              {user.role === 'administrator' && <Badge tone="accent">Administrator</Badge>}
            </span>
            <select
              className="select select-md w-full min-w-0 shrink-0 sm:w-64"
              aria-label={`Access for ${user.display_name}`}
              disabled={busy}
              value={assignments.find(a => a.user_id === user.id)?.role || ''}
              onChange={async e => {
                const role = e.target.value;
                if (role !== '' && role !== 'viewer' && role !== 'interactive' && role !== 'manager') return;
                setBusy(true);
                setError('');
                try {
                  await api(`/sessions/${machine.id}/access/${user.id}`, {
                    method: role ? 'PUT' : 'DELETE',
                    body: role ? JSON.stringify({ role }) : undefined,
                  });
                  await reload();
                } catch (e) {
                  setError(errorMessage(e));
                } finally {
                  setBusy(false);
                }
              }}
            >
              <option value="">No access</option>
              <option value="viewer">Viewer</option>
              <option value="interactive">Interactive</option>
              <option value="manager">Manager</option>
            </select>
          </label>
        ))}
      </div>
    </Section>
  );
}
