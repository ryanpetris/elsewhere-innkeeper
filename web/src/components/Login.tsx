// Sign in, or create the first Administrator when the instance has no accounts yet.
import type { LoginInput } from '../types.ts';
import { formText } from '../types.ts';
import { Loader2 } from 'lucide-react';
import { Field, Logo } from './ui.tsx';

export function Login({ required, error, submit }: { required: boolean | null; error: string; submit: (input: LoginInput) => Promise<void> }) {
  return (
    <main className="flex min-h-dvh items-center justify-center bg-canvas p-4 font-sans text-ink-2">
      <div className="w-[27rem] max-w-full animate-pop rounded-2xl border border-line-2 bg-surface p-6 shadow-pop sm:p-7">
        <div className="flex items-center gap-3">
          <Logo className="size-10" />
          <div className="min-w-0">
            <h1 className="text-base leading-tight font-semibold text-ink">Elsewhere Innkeeper</h1>
          </div>
        </div>
        {required !== null && (
          <p className="mt-5 text-sm leading-relaxed text-ink-2">
            {required ? 'Create the first Administrator account.' : 'Sign in to your account.'}
          </p>
        )}
        {required === null ? (
          <p role="status" className="mt-5 flex items-center gap-2 text-xs text-ink-4"><Loader2 className="size-3.5 animate-spin" /> Contacting the server…</p>
        ) : (
          <form
            className="mt-5 flex flex-col gap-4"
            onSubmit={async e => {
              e.preventDefault();
              const form = e.currentTarget;
              const fields = new FormData(form);
              const input: LoginInput = { username: formText(form, 'username'), password: formText(form, 'password') };
              if (required) {
                if (fields.get('confirmation') !== input.password) {
                  const confirmation = form.elements.namedItem('confirmation');
                  if (confirmation instanceof HTMLInputElement) {
                    confirmation.setCustomValidity('Passwords differ.');
                    confirmation.reportValidity();
                  }
                  return;
                }
                input.display_name = formText(form, 'display_name');
              }
              await submit(input);
            }}
          >
            <Field label="Username">
              <input className="input h-9" name="username" autoComplete="username" required maxLength={64} autoFocus />
            </Field>
            {required && (
              <Field label="Display name">
                <input className="input h-9" name="display_name" autoComplete="name" required maxLength={120} />
              </Field>
            )}
            <Field label="Password" hint={required ? 'At least 12 characters.' : undefined}>
              <input className="input h-9" type="password" name="password" autoComplete={required ? 'new-password' : 'current-password'} minLength={required ? 12 : undefined} required />
            </Field>
            {required && (
              <Field label="Confirm password">
                <input className="input h-9" type="password" name="confirmation" autoComplete="new-password" required onInput={e => e.currentTarget.setCustomValidity('')} />
              </Field>
            )}
            <button type="submit" className="btn btn-primary mt-1 h-10 w-full">{required ? 'Create Administrator' : 'Sign In'}</button>
          </form>
        )}
        {error && <p role="alert" className="callout callout-bad mt-5">{error}</p>}
      </div>
    </main>
  );
}
