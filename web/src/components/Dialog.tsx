// A modal dialog: the native element, so Escape, the backdrop and the focus trap come from the browser.
import type { ReactNode } from 'react';
import { useEffect, useRef } from 'react';
import { X } from 'lucide-react';
import { IconButton, cx } from './ui.tsx';

export function Dialog({ title, close, children }: { title: string; close: () => void; children: ReactNode }) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    ref.current?.showModal();
  }, []);
  return (
    <dialog
      ref={ref}
      aria-label={title}
      onCancel={close}
      // `open:` keeps the layout off the closed element, which the browser hides for us.
      className="m-auto w-[32rem] max-h-[86dvh] max-w-[calc(100vw-1.5rem)] flex-col overflow-hidden rounded-xl border border-line-2 bg-surface p-0 font-sans text-ink-2 shadow-pop open:flex open:animate-pop"
    >
      <div className="flex shrink-0 items-start gap-3 border-b border-line px-4 py-3">
        <div className="min-w-0 flex-1">
          <h2 className="truncate text-sm font-semibold text-ink">{title}</h2>
        </div>
        <IconButton icon={X} label="Close Dialog" onClick={close} />
      </div>
      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">{children}</div>
    </dialog>
  );
}

/// The question asked before an action that closes applications or discards data. `disabled` holds
/// the answer back while the machine's state no longer allows it.
export function Confirm({ title, label, tone = 'primary', disabled = false, close, confirm, children }: { title: string; label: string; tone?: 'primary' | 'danger'; disabled?: boolean; close: () => void; confirm: () => void; children: ReactNode }) {
  return (
    <Dialog title={title} close={close}>
      <p className="p-4 text-sm">{children}</p>
      <div className="flex justify-end gap-2 border-t border-line p-4">
        <button type="button" className="btn btn-outline btn-sm" onClick={close}>
          Cancel
        </button>
        <button
          type="button"
          className={cx('btn btn-sm', tone === 'danger' ? 'btn-danger' : 'btn-primary')}
          disabled={disabled}
          onClick={() => {
            close();
            confirm();
          }}
        >
          {label}
        </button>
      </div>
    </Dialog>
  );
}
