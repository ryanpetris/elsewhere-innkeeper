// Vocabulary shared by every view of a session: what its state is called and how its settings read.
import type { Session } from '../types.ts';
import type { Tone } from './ui.tsx';
import { Badge } from './ui.tsx';

// status → [badge tone, dot, pulse]
const STATUS: Record<Session['status'], [Tone, boolean, boolean]> = {
  running: ['ok', true, false],
  preparing: ['warn', true, true],
  upgrading: ['warn', true, true],
  failed: ['bad', true, false],
  stopped: ['neutral', false, false],
  cancelled: ['neutral', false, false],
};

const DISTRIBUTION = { arch: 'Arch Linux', debian: 'Debian 13', ubuntu: 'Ubuntu 26.04 LTS' };

export const distribution = (s: Session) => DISTRIBUTION[s.distribution] ?? s.distribution;

/// The state pill. Preparing and upgrading sessions pulse until they settle.
export function StatusBadge({ session }: { session: Session }) {
  const [tone, dot, pulse] = STATUS[session.status] ?? STATUS.stopped;
  return (
    <Badge tone={tone} dot={dot} pulse={pulse}>
      {session.status}
    </Badge>
  );
}

/// True while the session is neither working nor gone, so its settings and packages can be changed.
export const settled = (s: Session) => ['running', 'stopped'].includes(s.status);

export const busyState = (s: Session) => ['preparing', 'upgrading'].includes(s.status);

// The step a session reports while it works, in the words a reader would use for it.
const STAGES: Record<string, string> = {
  queued: 'Queued',
  download: 'Downloading Elsewhere',
  image: 'Pulling the base image',
  container: 'Creating the container',
  snapshot: 'Saving the session filesystem',
  setup: 'Preparing the system',
  upgrade: 'Updating system packages',
  elsewhere: 'Installing Elsewhere',
  packages: 'Installing packages',
  launch: 'Starting the desktop',
  ready: 'Ready',
};

export const stageLabel = (s: Session) => STAGES[s.stage] ?? s.stage;

/// What installing the preferred Elsewhere version would do to the version already there.
export const installLabel = (s: Session) => (s.version_status === 'older' ? 'Upgrade' : s.version_status === 'newer' ? 'Downgrade' : 'Reinstall');

/// When the settings saved for the next launch take effect.
export const pendingNote = (s: Session) =>
  ['stopped', 'upgrading'].includes(s.status)
    ? 'Applies on next start'
    : s.status === 'preparing'
      ? 'Applying on launch'
      : s.status === 'failed'
        ? 'Stop, then start to apply'
        : 'Relaunch to apply';

export const screenLabel = (s: Session) => (s.screen_size ? `${s.screen_size.width} × ${s.screen_size.height}` : 'Dynamic');

/// How long ago a server instant was, to the coarsest useful unit. `now` reports the server's clock.
export function elapsed(since: number, now: () => number) {
  const seconds = Math.max(0, Math.round((now() - since) / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ${minutes % 60}m`;
  return `${Math.floor(hours / 24)}d ${hours % 24}h`;
}
