// Live download, setup and runtime output for one session, polled without overlapping requests.
import type { Api, ManagedSession } from '../types.ts';
import { errorMessage, readJson } from '../types.ts';
import { useEffect, useRef, useState } from 'react';
import { Disclosure } from './ui.tsx';

export function Logs({ api, session }: { api: Api; session: ManagedSession }) {
  const pane = useRef<HTMLPreElement>(null);
  const follow = useRef(true);
  const [text, setText] = useState('Loading logs…');
  useEffect(() => {
    let live = true,
      visible = false,
      inflight = false;
    const controller = new AbortController();
    async function tick() {
      if (inflight || !visible || document.hidden) return;
      inflight = true;
      try {
        const r = await api(`/sessions/${session.id}/logs`, { signal: controller.signal });
        const data = await readJson<{ text: string }>(r);
        if (live) setText(data.text.replace(/\x1b\[[0-9;]*m/g, '') || 'Waiting for output…');
      } catch (e) {
        if (live) setText(errorMessage(e));
      } finally {
        inflight = false;
      }
    }
    // Output is only fetched while it can be read: the view open, on screen, and the tab in front.
    const observer = new IntersectionObserver(entries => {
      visible = entries[0]?.isIntersecting ?? false;
      if (visible) tick();
    });
    if (pane.current) observer.observe(pane.current);
    const timer = setInterval(tick, 2000);
    document.addEventListener('visibilitychange', tick);
    return () => {
      live = false;
      controller.abort();
      observer.disconnect();
      clearInterval(timer);
      document.removeEventListener('visibilitychange', tick);
    };
  }, [session.id]);
  useEffect(() => {
    if (follow.current && pane.current) pane.current.scrollTop = pane.current.scrollHeight;
  }, [text]);
  const timings = Object.entries(session.timings || {})
    .map(([stage, ms]) => `${stage}: ${(ms / 1000).toFixed(2)}s`)
    .join(' · ');
  return (
    <Disclosure label="Logs">
      <pre
        ref={pane}
        onScroll={e => {
          const node = e.currentTarget;
          follow.current = node.scrollHeight - node.scrollTop - node.clientHeight < 48;
        }}
        tabIndex={0}
        className="h-96 overflow-auto bg-canvas p-4 font-mono text-xs leading-relaxed whitespace-pre-wrap text-ink-2 select-text [overflow-wrap:anywhere] focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent"
      >
        {text}
      </pre>
      {timings && <p className="border-t border-line px-4 py-2.5 text-[11px] text-ink-4">{timings}</p>}
    </Disclosure>
  );
}
