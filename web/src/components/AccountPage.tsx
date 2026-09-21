// Your own account: the name other people see, and the password that reaches it.
import type { Api, User } from '../types.ts';
import { errorMessage, formText, readJson } from '../types.ts';
import { useState } from 'react';
import { Alert, Badge, DataList, Field, PageHeader, Section } from './ui.tsx';

export function AccountPage({ api, user, changed }: { api: Api; user: User; changed: (user: User) => void }) {
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  async function perform(work: () => Promise<void>) {
    setBusy(true);
    setError('');
    try {
      await work();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="mx-auto flex w-full max-w-2xl flex-col gap-7">
      <PageHeader title="Your Account" badge={user.role === 'administrator' && <Badge tone="accent">Administrator</Badge>} />
      {error && <Alert>{error}</Alert>}

      <Section title="Profile">
        <DataList
          items={[
            {
              label: 'Username',
              value: (
                <>
                  <code className="font-mono">{user.username}</code>
                  <span className="mt-0.5 block text-ink-4">Only an Administrator can change it.</span>
                </>
              ),
            },
          ]}
        />
        <form
          className="flex flex-col gap-4 border-t border-line p-4"
          onSubmit={e => {
            e.preventDefault();
            const display_name = formText(e.currentTarget, 'display_name');
            perform(async () => {
              const result = await readJson<{ user: User }>(await api('/me', { method: 'PATCH', body: JSON.stringify({ display_name }) }));
              changed(result.user);
            });
          }}
        >
          <Field label="Display name" hint="Appears wherever this account is listed.">
            <input className="input" name="display_name" defaultValue={user.display_name} required maxLength={120} />
          </Field>
          <button className="btn btn-primary btn-sm self-start" disabled={busy}>
            Save Display Name
          </button>
        </form>
      </Section>

      <Section title="Password" description="Changing your password signs this account out on every device, including this one.">
        <form
          className="flex flex-col gap-4 p-4"
          onSubmit={e => {
            e.preventDefault();
            const input = { current_password: formText(e.currentTarget, 'current_password'), password: formText(e.currentTarget, 'password') };
            perform(async () => {
              await api('/me/password', {
                method: 'PUT',
                body: JSON.stringify(input),
              });
              location.reload();
            });
          }}
        >
          <Field label="Current password">
            <input className="input" type="password" name="current_password" autoComplete="current-password" required />
          </Field>
          <Field label="New password" hint="At least 12 characters.">
            <input className="input" type="password" name="password" autoComplete="new-password" minLength={12} required />
          </Field>
          <button className="btn btn-outline btn-sm self-start" disabled={busy}>
            Change Password and Sign Out
          </button>
        </form>
      </Section>
    </div>
  );
}
