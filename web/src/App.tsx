// Innkeeper: the signed-in shell, the session data every page reads, and the routes between them.
import type { ReactNode } from 'react';
import type { Gpu, Layout, LoginResponse, Session, SessionAction, SessionClock, SessionsResponse, User } from './types.ts';
import { errorMessage, readJson, responseError } from './types.ts';
import { useEffect, useRef, useState } from 'react';
import { ShieldAlert, Compass } from 'lucide-react';
import { EmptyState } from './components/ui.tsx';
import { Header } from './components/Header.tsx';
import { Login } from './components/Login.tsx';
import { SessionsPage } from './components/SessionsPage.tsx';
import { SessionPage } from './components/SessionPage.tsx';
import { NewSessionPage, SessionSettingsPage } from './components/SessionFormPage.tsx';
import { AccountPage } from './components/AccountPage.tsx';
import { NewUserPage, UserPage, UsersPage } from './components/Users.tsx';
import { Link, useRoute } from './router.tsx';

export function App() {
  const [version, setVersion] = useState('');
  const [localElsewhere, setLocalElsewhere] = useState(false);
  const [gpus, setGpus] = useState<Gpu[]>([]);
  const [gpuErrors, setGpuErrors] = useState<string[]>([]);
  const [token, setToken] = useState('');
  const [user, setUser] = useState<User | null>(null);
  const [setupRequired, setSetupRequired] = useState<boolean | null>(null);
  const [authenticated, setAuthenticated] = useState(false);
  const [sessions, setSessions] = useState<Session[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [layout, setLayout] = useState<Layout>(() => localStorage.getItem('innkeeper-layout') === 'list' ? 'list' : 'grid');
  const [error, setError] = useState('');
  const [busy, setBusy] = useState<Record<string, boolean>>({});
  const loginClock = useRef<{ remaining: number; at: number } | null>(null);
  const clockOffset = useRef(0);
  const currentToken = useRef(token);
  currentToken.current = token;
  const route = useRoute();
  /// The instant the server would report now, so an elapsed time does not depend on this clock.
  const serverNow = () => Date.now() + clockOffset.current;
  function acceptLogin(data: LoginResponse) {
    setUser(data.user);
    setToken(data.csrf_token);
    setAuthenticated(true);
    clockOffset.current = data.server_time_ms - Date.now();
    loginClock.current = { remaining: data.session_expires_at_ms - data.server_time_ms, at: performance.now() };
  }
  useEffect(() => {
    let live = true;
    (async () => {
      try {
        const response = await fetch('/api/me');
        if (response.ok) {
          const data = await readJson<LoginResponse>(response);
          if (live) acceptLogin(data);
        } else if (response.status === 401) {
          const setup = await fetch('/api/setup');
          if (!setup.ok) throw new Error('Account setup is unavailable.');
          const data = await readJson<{ required: boolean }>(setup);
          if (live) setSetupRequired(data.required);
        } else throw new Error('Account service is unavailable.');
      } catch (e) {
        if (live) {
          setSetupRequired(false);
          setError(errorMessage(e));
        }
      }
    })();
    return () => {
      live = false;
    };
  }, []);
  // The token comes from the ref, so a long-lived poller never sends a value the login has replaced.
  async function api(path: string, options: RequestInit = {}): Promise<Response> {
    const sent = currentToken.current;
    const response = await fetch(`/api${path}`, {
      ...options,
      body: options.body ?? (options.method && options.method !== 'GET' ? '{}' : undefined),
      headers: {
        'X-Innkeeper-CSRF': sent,
        'Content-Type': 'application/json',
        ...options.headers,
      },
    });
    if (currentToken.current !== sent) throw new Error('Session ended.');
    if (response.status === 401) {
      setAuthenticated(false);
      setUser(null);
      setToken('');
      setSetupRequired(false);
      throw new Error('Your session has ended. Sign in again.');
    }
    if (!response.ok) {
      const body = await responseError(response);
      throw new Error(body.message || body.error || `Request failed (${response.status})`);
    }
    return response;
  }
  async function refresh(signal?: AbortSignal) {
    const response = await api('/sessions', { signal });
    const data = await readJson<SessionsResponse>(response);
    if (currentToken.current !== token || signal?.aborted) return;
    setSessions(data.sessions);
    setGpus(data.gpus);
    setGpuErrors(data.gpu_errors);
    setVersion(data.version);
    setLocalElsewhere(Boolean(data.local_elsewhere));
    setAuthenticated(true);
    setLoaded(true);
  }
  useEffect(() => {
    if (!token) return;
    let live = true,
      inflight = false;
    const controller = new AbortController();
    const tick = async () => {
      if (document.hidden || inflight) return;
      inflight = true;
      try {
        await refresh(controller.signal);
      } catch (e) {
        if (live) setError(errorMessage(e));
      } finally {
        inflight = false;
      }
    };
    tick();
    const timer = setInterval(tick, 3000);
    return () => {
      live = false;
      controller.abort();
      clearInterval(timer);
    };
  }, [token]);
  useEffect(() => {
    localStorage.setItem('innkeeper-layout', layout);
  }, [layout]);
  /// Run one machine action, keeping the session marked busy until the list reflects it.
  async function action(session: Session, kind: SessionAction) {
    setBusy(b => ({ ...b, [session.id]: true }));
    setError('');
    try {
      await api(`/sessions/${session.id}${kind === 'destroy' ? '' : `/${kind}`}`, { method: kind === 'destroy' ? 'DELETE' : 'POST' });
      await refresh();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(b => {
        const next = { ...b };
        delete next[session.id];
        return next;
      });
    }
  }
  function openSession(session: Session) {
    window.open(`/api/sessions/${session.id}/connect`, '_blank', 'noopener,noreferrer');
  }
  useEffect(() => {
    if (!token) return;
    let live = true,
      inflight = false;
    async function renew() {
      if (inflight || !live) return;
      inflight = true;
      try {
        const response = await api('/me');
        const data = await readJson<LoginResponse>(response);
        if (!live) return;
        setUser(data.user);
        clockOffset.current = data.server_time_ms - Date.now();
        if (data.csrf_token !== token) {
          setToken(data.csrf_token);
          return;
        }
        loginClock.current = { remaining: data.session_expires_at_ms - data.server_time_ms, at: performance.now() };
        if (loginClock.current.remaining > 0 && loginClock.current.remaining <= 2 * 86400000) {
          const renewal = await api('/session/renew', { method: 'POST' });
          const renewed = await readJson<SessionClock>(renewal);
          if (live) loginClock.current = { remaining: renewed.session_expires_at_ms - renewed.server_time_ms, at: performance.now() };
        }
      } catch (e) {
        if (live) setError(errorMessage(e));
      } finally {
        inflight = false;
      }
    }
    renew();
    const timer = setInterval(renew, 60000);
    window.addEventListener('focus', renew);
    window.addEventListener('online', renew);
    document.addEventListener('visibilitychange', renew);
    return () => {
      live = false;
      clearInterval(timer);
      window.removeEventListener('focus', renew);
      window.removeEventListener('online', renew);
      document.removeEventListener('visibilitychange', renew);
    };
  }, [token]);
  if (!authenticated || !user)
    return (
      <Login
        required={setupRequired}
        error={error}
        submit={async input => {
          setError('');
          try {
            const response = await fetch(`/api/${setupRequired ? 'setup' : 'login'}`, {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify(input),
            });
            if (!response.ok) {
              const data = await responseError(response);
              if (data.error === 'setup_complete') setSetupRequired(false);
              throw new Error(data.message || 'Sign in failed.');
            }
            acceptLogin(await readJson<LoginResponse>(response));
            const destination = new URLSearchParams(location.search).get('return');
            if (destination && /^\/api\/sessions\/[0-9a-f-]{36}\/connect$/.test(destination)) location.replace(destination);
          } catch (e) {
            setError(errorMessage(e));
          }
        }}
      />
    );
  const administrator = user.role === 'administrator';
  const guarded = (page: ReactNode) =>
    administrator ? (
      page
    ) : (
      <EmptyState className="mt-10" icon={ShieldAlert} title="Administrators Only" description="Ask an Administrator to manage accounts for you.">
        <Link to="/" className="btn btn-outline btn-sm mt-1">
          Back to Sessions
        </Link>
      </EmptyState>
    );
  const page = () => {
    switch (route.name) {
      case 'sessions':
        return (
          <SessionsPage
            sessions={sessions}
            loaded={loaded}
            api={api}
            layout={layout}
            setLayout={setLayout}
            busy={busy}
            onOpen={openSession}
            onAction={action}
          />
        );
      case 'session':
        return (
          <SessionPage
            key={route.id}
            id={route.id}
            sessions={sessions}
            loaded={loaded}
            user={user}
            api={api}
            busy={busy}
            now={serverNow}
            onOpen={openSession}
            onAction={action}
          />
        );
      case 'session-new':
        return <NewSessionPage api={api} user={user} refresh={refresh} loaded={loaded} gpus={gpus} gpuErrors={gpuErrors} />;
      case 'session-settings':
        return <SessionSettingsPage key={route.id} id={route.id} sessions={sessions} loaded={loaded} api={api} refresh={refresh} />;
      case 'account':
        return <AccountPage api={api} user={user} changed={setUser} />;
      case 'users':
        return guarded(<UsersPage api={api} user={user} />);
      case 'user-new':
        return guarded(<NewUserPage api={api} />);
      case 'user':
        return guarded(<UserPage key={route.id} id={route.id} api={api} user={user} changed={setUser} />);
      default:
        return (
          <EmptyState className="mt-10" icon={Compass} title="Page Not Found" description="The address does not match anything in Innkeeper.">
            <Link to="/" className="btn btn-outline btn-sm mt-1">
              Back to Sessions
            </Link>
          </EmptyState>
        );
    }
  }
  return (
    <div className="flex min-h-dvh flex-col bg-canvas font-sans text-ink-2">
      <Header
        user={user}
        route={route}
        onSignOut={async () => {
          try {
            await api('/logout', { method: 'POST' });
          } catch (e) {
            setError(errorMessage(e));
            return;
          }
          currentToken.current = '';
          setToken('');
          setUser(null);
          setSetupRequired(false);
          setAuthenticated(false);
          setSessions([]);
          setLoaded(false);
        }}
      />
      <main className="mx-auto w-full max-w-7xl flex-1 px-4 py-8 sm:px-6 sm:py-10">
        {error && (
          <div role="alert" className="callout callout-bad mb-6 flex items-center gap-3">
            <span className="min-w-0 flex-1 [overflow-wrap:anywhere]">{error}</span>
            <button type="button" className="btn btn-outline btn-xs" onClick={() => setError('')}>
              Dismiss
            </button>
          </div>
        )}
        {page()}
      </main>
      <footer className="border-t border-line px-4 py-4 sm:px-6">
        <p className="mx-auto flex max-w-7xl flex-wrap items-center gap-x-2 text-[11px] text-ink-4">
          Elsewhere Innkeeper <code className="font-mono text-ink-3">v{version}</code>
          {localElsewhere && <span>· Local Elsewhere build</span>}
        </p>
      </footer>
    </div>
  );
}
