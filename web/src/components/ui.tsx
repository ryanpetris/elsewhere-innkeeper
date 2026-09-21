// Small shared pieces: buttons, badges, page furniture, and the brand mark.
import type { ButtonHTMLAttributes, HTMLAttributes, ReactElement, ReactNode, Ref } from 'react';
import type { LucideIcon } from 'lucide-react';
import { cloneElement, useId } from 'react';
import { ArrowLeft, ChevronRight, Loader2 } from 'lucide-react';
import { Link } from '../router.tsx';

export type Tone = keyof typeof TONES;

/// Class names, skipping the falsy ones.
export const cx = (...parts: (string | false | null | undefined)[]) => parts.filter(Boolean).join(' ');

/// An icon button. `active` marks a toggle that is on; leave it out on a button that is not a toggle.
export function IconButton({ icon: Icon, label, active, className = '', ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { icon: LucideIcon; label: string; active?: boolean }) {
  return (
    <button
      {...props}
      type="button"
      title={label}
      aria-label={label}
      aria-pressed={active}
      className={cx(
        'inline-flex size-7 shrink-0 items-center justify-center rounded-md transition-colors focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent disabled:opacity-40',
        active ? 'bg-accent/15 text-accent-2 ring-1 ring-accent/40 ring-inset' : 'text-ink-3 enabled:hover:bg-surface-3 enabled:hover:text-ink',
        className,
      )}
    >
      <Icon className="size-3.5" strokeWidth={1.75} />
    </button>
  );
}

const TONES = {
  neutral: 'border-line-2 bg-surface-3 text-ink-2',
  accent: 'border-accent/30 bg-accent/10 text-accent-2',
  ok: 'border-ok/30 bg-ok/10 text-ok',
  warn: 'border-warn/30 bg-warn/10 text-warn',
  bad: 'border-bad/30 bg-bad/10 text-bad',
};

/// A small status pill.
export function Badge({ tone = 'neutral', dot = false, pulse = false, className = '', children, ...props }: HTMLAttributes<HTMLSpanElement> & { tone?: Tone; dot?: boolean; pulse?: boolean }) {
  return (
    <span {...props} className={cx('inline-flex shrink-0 items-center gap-1.5 rounded-full border px-2 py-px text-[10px] font-medium whitespace-nowrap', TONES[tone], className)}>
      {dot && <span className={cx('size-1.5 rounded-full bg-current', pulse && 'animate-glow')} />}
      {children}
    </span>
  );
}

/// A vertical hairline between toolbar groups.
export const Divider = ({ className = '' }: { className?: string }) => <span aria-hidden="true" className={cx('mx-1 h-5 w-px shrink-0 bg-line-2', className)} />;

/// The brand mark: the monitor glyph on an accent tile.
export function Logo({ className = 'size-7' }: { className?: string }) {
  return (
    <span className={cx('inline-flex shrink-0 items-center justify-center rounded-lg bg-linear-to-br from-accent to-[#a78bfa] text-white shadow-card', className)} aria-hidden="true">
      <svg viewBox="0 0 24 24" className="size-[62%]" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <rect x="3" y="4" width="18" height="13" rx="2" />
        <path d="M8 20h8M12 17v3M7 9.5l2.5 2.5L7 14.5M12 14.5h4" />
      </svg>
    </span>
  );
}

/// A labelled control. The hint sits outside the label, so the control's name stays the label alone.
export function Field({ label, hint, className = '', children }: { label: string; hint?: ReactNode; className?: string; children: ReactElement<{ 'aria-describedby'?: string }> }) {
  const id = useId();
  return (
    <div className={className}>
      <label className="block">
        <span className="eyebrow mb-1.5 block">{label}</span>
        {hint ? cloneElement(children, { 'aria-describedby': id }) : children}
      </label>
      {hint && <p id={id} className="mt-1.5 text-[11px] leading-relaxed text-ink-4">{hint}</p>}
    </div>
  );
}

/// A form-wide message.
export const Alert = ({ children }: { children: ReactNode }) => <p role="alert" className="callout callout-bad">{children}</p>;

/// A waiting line.
export const Loading = ({ children }: { children: ReactNode }) => (
  <p role="status" className="flex items-center gap-2 text-xs text-ink-4">
    <Loader2 className="size-3.5 shrink-0 animate-spin" />
    {children}
  </p>
);

/// The title block a page opens with: where it sits, what it is, and its one primary action.
export function PageHeader({ back, title, badge, description, children, action }: { back?: { to: string; label: string }; title: string; badge?: ReactNode; description?: ReactNode; children?: ReactNode; action?: ReactNode }) {
  return (
    <div className="min-w-0">
      {back && (
        <Link to={back.to} className="-ml-1.5 mb-3 inline-flex items-center gap-1.5 rounded-md px-1.5 py-1 text-xs text-ink-3 transition-colors hover:text-ink focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent">
          <ArrowLeft className="size-3.5" strokeWidth={2} />
          {back.label}
        </Link>
      )}
      <div className="flex flex-wrap items-start justify-between gap-x-6 gap-y-4">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
            <h1 className="text-2xl font-semibold tracking-tight text-ink [overflow-wrap:anywhere]">{title}</h1>
            {badge}
          </div>
          {description && <p className="mt-1.5 text-sm text-ink-3">{description}</p>}
          {children}
        </div>
        {action && <div className="flex shrink-0 items-center gap-2">{action}</div>}
      </div>
    </div>
  );
}

/// A titled group of related settings or actions.
export function Section({ title, description, action, className = '', children }: { title: string; description?: ReactNode; action?: ReactNode; className?: string; children: ReactNode }) {
  return (
    <section className={cx('card overflow-hidden', className)}>
      <div className="flex items-start gap-3 border-b border-line px-4 py-3">
        <div className="min-w-0 flex-1">
          <h2 className="text-sm font-semibold text-ink">{title}</h2>
          {description && <p className="mt-0.5 text-xs leading-relaxed text-ink-3">{description}</p>}
        </div>
        {action && <div className="flex shrink-0 items-center gap-2">{action}</div>}
      </div>
      {children}
    </section>
  );
}

/// A section that stays folded until it is wanted. `panelRef` reaches the element to open it.
export function Disclosure({ label, panelRef, danger = false, children }: { label: string; panelRef?: Ref<HTMLDetailsElement>; danger?: boolean; children: ReactNode }) {
  return (
    <details ref={panelRef} className={cx('group card overflow-hidden', danger && 'bg-transparent shadow-none open:border-bad/25 open:bg-surface-2 open:shadow-card')}>
      <summary className="flex items-center gap-3 px-4 py-3 transition-colors select-none hover:bg-surface-3">
        <ChevronRight className="size-4 shrink-0 text-ink-3 transition-transform group-open:rotate-90" strokeWidth={2} />
        <h2 className={cx('min-w-0 flex-1 text-sm', danger ? 'font-medium text-ink-3 group-open:font-semibold group-open:text-ink' : 'font-semibold text-ink')}>{label}</h2>
      </summary>
      <div className="border-t border-line">{children}</div>
    </details>
  );
}

/// The rows a section uses to state facts. Rows with no value are left out.
export function DataList({ items, className = '' }: { items: ({ label: string; value: ReactNode } | false | null | undefined)[]; className?: string }) {
  return (
    <dl className={cx('divide-y divide-line', className)}>
      {items
        .filter((item): item is { label: string; value: ReactNode } => !!item && item.value !== null && item.value !== undefined && item.value !== '')
        .map(item => (
          <div key={item.label} className="flex flex-col gap-1 px-4 py-2.5 sm:flex-row sm:gap-4">
            <dt className="shrink-0 text-xs text-ink-4 sm:w-32">{item.label}</dt>
            <dd className="min-w-0 flex-1 text-xs text-ink-2 [overflow-wrap:anywhere]">{item.value}</dd>
          </div>
        ))}
    </dl>
  );
}

/// The placeholder a page shows when it has nothing to list.
export function EmptyState({ icon: Icon, title, description, children, className = '' }: { icon?: LucideIcon; title: string; description?: ReactNode; children?: ReactNode; className?: string }) {
  return (
    <div className={cx('flex flex-col items-center gap-3 rounded-xl border border-dashed border-line-2 px-6 py-16 text-center', className)}>
      {Icon && (
        <span className="flex size-12 items-center justify-center rounded-xl border border-line bg-surface-2 text-ink-4">
          <Icon className="size-6" strokeWidth={1.5} />
        </span>
      )}
      <h2 className="text-sm font-semibold text-ink">{title}</h2>
      {description && <p className="max-w-sm text-xs leading-relaxed text-ink-4">{description}</p>}
      {children}
    </div>
  );
}

/// The row a form ends with, carrying what saving does, the way out and the commit.
export function FormActions({ note, children }: { note?: ReactNode; children: ReactNode }) {
  return (
    <div className="card mt-1 flex flex-wrap items-center justify-end gap-x-4 gap-y-2 px-3 py-2.5">
      {note && <p className="mr-auto max-w-md text-[11px] leading-relaxed text-ink-4">{note}</p>}
      {children}
    </div>
  );
}
