// The top bar: the brand, where you can go, the signed-in account, and the way out.
import type { ReactNode } from 'react';
import type { User } from '../types.ts';
import type { Route } from '../router.tsx';
import { LogOut, UserRound } from 'lucide-react';
import { Divider, Logo, cx } from './ui.tsx';
import { Link } from '../router.tsx';

const NavLink = ({ to, active, children }: { to: string; active: boolean; children: ReactNode }) => (
  <Link to={to} className={cx('nav-link', active && 'nav-link-active')} aria-current={active ? 'page' : undefined}>
    {children}
  </Link>
);

export function Header({ user, route, onSignOut }: { user: User; route: Route; onSignOut: () => Promise<void> }) {
  const administrator = user.role === 'administrator';
  return (
    <header className="sticky top-0 z-20 flex h-12 shrink-0 items-center gap-2 border-b border-line bg-surface/95 px-3 backdrop-blur">
      <Link to="/" className="flex min-w-0 items-center gap-2.5 rounded-md focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent">
        <Logo />
        <span className="hidden min-w-0 truncate text-sm font-semibold tracking-tight text-ink sm:inline">
          Elsewhere <span className="font-normal text-ink-3">Innkeeper</span>
        </span>
      </Link>
      <nav aria-label="Sections" className="ml-1 flex items-center gap-1 sm:ml-3">
        <NavLink to="/" active={route.name.startsWith('session')}>
          Sessions
        </NavLink>
        {administrator && (
          <NavLink to="/users" active={route.name.startsWith('user')}>
            Users
          </NavLink>
        )}
      </nav>
      <div className="ml-auto flex shrink-0 items-center gap-1">
        <Link
          to="/account"
          className={cx('btn btn-ghost btn-sm max-w-[12rem]', route.name === 'account' && 'bg-surface-3 text-ink')}
          aria-current={route.name === 'account' ? 'page' : undefined}
        >
          <UserRound className="size-3.5 shrink-0" strokeWidth={1.75} />
          <span className="min-w-0 truncate">{user.display_name}</span>
        </Link>
        <Divider className="hidden sm:block" />
        <button type="button" className="btn btn-outline btn-sm" aria-label="Sign Out" onClick={onSignOut}>
          <LogOut className="size-3.5" strokeWidth={1.75} />
          <span className="hidden sm:inline">Sign Out</span>
        </button>
      </div>
    </header>
  );
}
