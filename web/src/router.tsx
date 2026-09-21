// Client-side routing over the History API. Innkeeper serves the same document for every in-app path.
import type { AnchorHTMLAttributes } from 'react';
import { useEffect, useState } from 'react';

export type RouteName = 'sessions' | 'session-new' | 'session-settings' | 'session' | 'account' | 'users' | 'user-new' | 'user';
export type Route = { path: string } & (
  | { name: 'session' | 'session-settings' | 'user'; id: string }
  | { name: Exclude<RouteName, 'session' | 'session-settings' | 'user'> | 'missing' }
);

const CHANGED = 'innkeeper:navigate';

/// Move to another in-app path. `replace` leaves the current entry out of the history stack.
export function navigate(to: string, { replace = false }: { replace?: boolean } = {}) {
  if (to === location.pathname) return;
  history[replace ? 'replaceState' : 'pushState'](null, '', to);
  // A page opens at its top. Going back keeps the browser's own restored position instead.
  scrollTo(0, 0);
  dispatchEvent(new Event(CHANGED));
}

function usePath() {
  const [path, setPath] = useState(() => location.pathname);
  useEffect(() => {
    const update = () => setPath(location.pathname);
    addEventListener('popstate', update);
    addEventListener(CHANGED, update);
    return () => {
      removeEventListener('popstate', update);
      removeEventListener(CHANGED, update);
    };
  }, []);
  return path;
}

// Ordered so that a literal segment wins over the identifier that shares its shape.
const ROUTES: [RouteName, RegExp][] = [
  ['sessions', /^\/$/],
  ['session-new', /^\/sessions\/new$/],
  ['session-settings', /^\/sessions\/([\w-]+)\/settings$/],
  ['session', /^\/sessions\/([\w-]+)$/],
  ['account', /^\/account$/],
  ['users', /^\/users$/],
  ['user-new', /^\/users\/new$/],
  ['user', /^\/users\/([\w-]+)$/],
];

/// The route the address bar names, and the identifier it carries. A trailing slash names the
/// same page, so a link that picks one up still arrives.
export function useRoute(): Route {
  const path = usePath();
  const named = path.length > 1 ? path.replace(/\/+$/, '') || '/' : path;
  for (const [name, pattern] of ROUTES) {
    const found = named.match(pattern);
    if (!found) continue;
    if (name === 'session' || name === 'session-settings' || name === 'user') {
      if (found[1]) return { name, id: found[1], path };
    } else return { name, path };
  }
  return { name: 'missing', path };
}

/// Where a finished form sends the browser, unless the reader has already left it for somewhere else.
export function leave(from: string, to: string) {
  if (location.pathname === from) navigate(to);
}

/// An in-app link. Modified clicks keep the browser's own behaviour, so opening a tab still works.
export function Link({ to, replace = false, onClick, ...props }: Omit<AnchorHTMLAttributes<HTMLAnchorElement>, 'href'> & { to: string; replace?: boolean }) {
  return (
    <a
      {...props}
      href={to}
      onClick={event => {
        onClick?.(event);
        if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
        event.preventDefault();
        navigate(to, { replace });
      }}
    />
  );
}
