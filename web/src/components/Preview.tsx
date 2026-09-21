// The authenticated desktop preview. Captures are queued so a full page of sessions cannot flood the server.
import type { Api, Session } from '../types.ts';
import { useEffect, useRef, useState } from 'react';
import { Monitor } from 'lucide-react';
import { cx } from './ui.tsx';

let active = 0;
const queue: (() => Promise<void>)[] = [];
function enqueue(task: () => Promise<void>) {
  queue.push(task);
  function drain() {
    while (active < 2 && queue.length) {
      active++;
      Promise.resolve(queue.shift()?.()).finally(() => {
        active--;
        drain();
      });
    }
  }
  drain();
}

/// `interval` is the refresh period in milliseconds; the server allows one capture every two seconds.
export function Preview({ session, api, interval = 5000, className = '', glyph = 'size-7', label = true }: { session: Session; api: Api; interval?: number; className?: string; glyph?: string; label?: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  const [url, setUrl] = useState('');
  const urlRef = useRef('');
  useEffect(
    () => () => {
      if (urlRef.current) URL.revokeObjectURL(urlRef.current);
    },
    [],
  );
  useEffect(() => {
    if (session.status !== 'running') {
      if (urlRef.current) URL.revokeObjectURL(urlRef.current);
      urlRef.current = '';
      setUrl('');
    }
    let visible = false,
      disposed = false,
      inflight = false;
    const controller = new AbortController();
    async function capture() {
      if (disposed || inflight || !visible || document.hidden || session.status !== 'running') return;
      inflight = true;
      enqueue(async () => {
        if (disposed || !visible || document.hidden) {
          inflight = false;
          return;
        }
        try {
          if (!ref.current) return;
          const width = Math.min(1600, Math.max(1, Math.ceil(ref.current.clientWidth * devicePixelRatio)));
          const r = await api(`/sessions/${session.id}/preview?width=${width}`, { signal: controller.signal });
          const blob = await r.blob();
          if (!disposed) {
            if (urlRef.current) URL.revokeObjectURL(urlRef.current);
            urlRef.current = URL.createObjectURL(blob);
            setUrl(urlRef.current);
          }
        } catch {
          // Keep the last frame through rate limits and temporary capture failures.
        } finally {
          inflight = false;
        }
      });
    }
    const observer = new IntersectionObserver(entries => {
      visible = entries[0]?.isIntersecting ?? false;
      if (visible) capture();
    });
    if (ref.current) observer.observe(ref.current);
    const timer = setInterval(capture, interval);
    document.addEventListener('visibilitychange', capture);
    return () => {
      disposed = true;
      controller.abort();
      observer.disconnect();
      clearInterval(timer);
      document.removeEventListener('visibilitychange', capture);
    };
  }, [session.id, session.status, interval, className]);
  return (
    <div ref={ref} className={cx('flex items-center justify-center overflow-hidden bg-canvas', className)}>
      {url && session.status === 'running' ? (
        <img src={url} alt={`Desktop preview of ${session.name}`} className="size-full object-contain" />
      ) : (
        // Whether the desktop is reachable, and nothing about the work that gets it there.
        <div className="flex flex-col items-center gap-2 px-2 text-center text-ink-4">
          <Monitor className={glyph} strokeWidth={1.5} />
          {label && <span className="text-[11px] leading-tight">{session.status === 'running' ? 'Preview unavailable' : 'Offline'}</span>}
        </div>
      )}
    </div>
  );
}
