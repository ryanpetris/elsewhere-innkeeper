// The settings a session is created with, and the subset that can be changed afterwards.
import type { Distribution, Gpu, ManagedSession, ScreenSize, SessionProfile } from '../types.ts';
import { errorMessage, isRecord } from '../types.ts';
import { useRef, useState } from 'react';
import { Alert, Disclosure, Field, FormActions, Section } from './ui.tsx';
import { Link } from '../router.tsx';

function isDistribution(value: unknown): value is Distribution {
  return value === 'arch' || value === 'debian' || value === 'ubuntu';
}
function isStringList(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item: unknown) => typeof item === 'string');
}
function isScreenSize(value: unknown): value is ScreenSize {
  return isRecord(value) && Object.keys(value).every(key => key === 'width' || key === 'height') &&
    [value.width, value.height].every(n => typeof n === 'number' && Number.isInteger(n) && n >= 2 && n <= 8192 && n % 2 === 0);
}

const defaultProfile: SessionProfile = {
  name: '',
  distribution: 'arch',
  packages: [],
  docker_args: [],
  gpu_access: false,
  gpu_id: null,
  software_encoding: false,
  startup_command: '',
  screen_size: null,
  kiosk: false,
};
const screenPresets = ['1280x720', '1920x1080', '2560x1440', '3840x2160'];

/// `note` says what saving does; `blocked`, when set, replaces it with why it cannot and holds the
/// commit back.
export function SessionForm({ submit, error, initial, gpus = [], gpuErrors = [], administrator = false, cancelTo, note = '', blocked = '' }: { submit: (profile: SessionProfile) => Promise<void>; error: string; initial?: ManagedSession; gpus?: Gpu[]; gpuErrors?: string[]; administrator?: boolean; cancelTo: string; note?: string; blocked?: string }) {
  const gpuAvailable = gpus.length > 0;
  const defaults = { ...defaultProfile, gpu_access: gpuAvailable, gpu_id: gpus[0]?.id ?? null, software_encoding: !gpuAvailable };
  const [profile, setProfile] = useState<SessionProfile>(initial || defaults);
  const [packages, setPackages] = useState(initial?.packages.join(' ') || '');
  const [dockerArgs, setDockerArgs] = useState(initial?.docker_args.join('\n') || '');
  const initialSize = initial?.screen_size;
  const initialPreset = initialSize ? `${initialSize.width}x${initialSize.height}` : 'dynamic';
  const [screen, setScreen] = useState(initialSize && !screenPresets.includes(initialPreset) ? 'custom' : initialPreset);
  const [width, setWidth] = useState<number | string>(initialSize?.width ?? 1920);
  const [height, setHeight] = useState<number | string>(initialSize?.height ?? 1080);
  const [text, setText] = useState('');
  const [importError, setImportError] = useState('');
  const importPanel = useRef<HTMLDetailsElement>(null);
  const [pending, setPending] = useState(false);
  function change<K extends keyof SessionProfile>(key: K, value: SessionProfile[K]) {
    setProfile(p => ({ ...p, [key]: value }));
  }
  function importProfile() {
    try {
      const value: unknown = JSON.parse(text);
      if (!isRecord(value) || Object.keys(value).some(key => !Object.hasOwn(defaultProfile, key))) {
        throw new Error('Profile must be an object containing session settings only.');
      }
      const p: Record<string, unknown> = { ...defaults, ...value };
      if (
        typeof p.name !== 'string' ||
        !isDistribution(p.distribution) ||
        !isStringList(p.packages) ||
        p.packages.some(item => /\s/.test(item)) ||
        !isStringList(p.docker_args) ||
        p.docker_args.some(arg => /[\r\n\0]/.test(arg)) ||
        typeof p.startup_command !== 'string' ||
        typeof p.kiosk !== 'boolean' ||
        typeof p.gpu_access !== 'boolean' ||
        (p.gpu_id !== null && typeof p.gpu_id !== 'string') ||
        typeof p.software_encoding !== 'boolean'
      ) {
        throw new Error('Invalid profile field types.');
      }
      if (
        p.screen_size !== null && !isScreenSize(p.screen_size)
      ) {
        throw new Error('Screen dimensions must be even numbers between 2 and 8192.');
      }
      if (!p.gpu_access && value.gpu_id != null) throw new Error('GPU selection requires GPU access.');
      if (p.gpu_access && !gpuAvailable) throw new Error('No host GPU is available.');
      if (p.gpu_access && p.gpu_id !== null && !gpus.some(g => g.id === p.gpu_id)) throw new Error('Selected GPU is unavailable.');
      const gpuId = p.gpu_access ? (p.gpu_id ?? gpus[0]?.id ?? null) : null;
      const softwareEncoding = p.software_encoding || !p.gpu_access;
      setProfile({
        name: p.name, distribution: p.distribution, packages: p.packages, docker_args: p.docker_args,
        startup_command: p.startup_command, kiosk: p.kiosk, gpu_access: p.gpu_access,
        gpu_id: gpuId, software_encoding: softwareEncoding, screen_size: p.screen_size,
      });
      setPackages(p.packages.join(' '));
      setDockerArgs(p.docker_args.join('\n'));
      const size = p.screen_size;
      const preset = size ? `${size.width}x${size.height}` : 'dynamic';
      setScreen(size && !screenPresets.includes(preset) ? 'custom' : preset);
      setWidth(size?.width ?? 1920);
      setHeight(size?.height ?? 1080);
      setImportError('');
      setText('');
    } catch (e) {
      setImportError(errorMessage(e));
    }
  }
  return (
    <form
      onSubmit={async event => {
        event.preventDefault();
        if (text.trim()) {
          if (importPanel.current) importPanel.current.open = true;
          setImportError(message => message || 'Apply or clear the pasted profile before creating a session.');
          return;
        }
        setPending(true);
        const size =
          screen === 'dynamic'
            ? null
            : screen === 'custom'
              ? { width: Number(width), height: Number(height) }
              : { width: Number(screen.split('x')[0]), height: Number(screen.split('x')[1]) };
        try {
          await submit({
            ...profile,
            packages: packages.trim().split(/\s+/).filter(Boolean),
            docker_args: (administrator ? dockerArgs : '').split('\n').map(line => line.trim()).filter(Boolean),
            screen_size: size,
            gpu_id: profile.gpu_access ? profile.gpu_id : null,
            software_encoding: profile.software_encoding || !profile.gpu_access,
          });
        } finally {
          setPending(false);
        }
      }}
    >
      <fieldset disabled={pending} className="flex min-w-0 flex-col gap-5">
        {!initial && (
          <Disclosure label="Import Profile" panelRef={importPanel}>
            <div className="flex flex-col gap-4 p-4">
              <Field label="Profile JSON">
              <textarea
                rows={6}
                value={text}
                onChange={e => {
                  setText(e.target.value);
                  setImportError('');
                }}
                spellCheck={false}
                className="input min-h-28 resize-y py-2 font-mono text-xs"
              />
            </Field>
              <button type="button" className="btn btn-outline btn-sm self-start" onClick={importProfile}>
                Apply Profile
              </button>
            </div>
          </Disclosure>
        )}
        {importError && <Alert>{importError}</Alert>}

        <Section title="Basics">
          <div className="flex flex-col gap-4 p-4">
            <Field label="Session name">
              <input
                className="input"
                name="name"
                required
                maxLength={80}
                placeholder="My desktop"
                autoFocus
                value={profile.name}
                onChange={e => change('name', e.target.value)}
              />
            </Field>
            <Field label="Distribution" hint={initial ? 'The distribution is set at creation.' : undefined}>
              <select
                className="select select-md w-full"
                disabled={!!initial}
                name="distribution"
                value={profile.distribution}
                onChange={e => { if (isDistribution(e.target.value)) change('distribution', e.target.value); }}
              >
                <option value="arch">Arch Linux · rolling base</option>
                <option value="debian">Debian 13</option>
                <option value="ubuntu">Ubuntu 26.04 LTS</option>
              </select>
            </Field>
          </div>
        </Section>

        <Section title="Software">
          <div className="p-4">
            <Field
              label="Extra packages"
              hint={initial ? 'Packages are set at creation. Create a new session to change them.' : 'Optional. Separate package names with spaces.'}
            >
              <textarea
                className="input min-h-16 resize-y py-2 font-mono text-xs"
                readOnly={!!initial}
                name="packages"
                rows={3}
                placeholder="firefox foot"
                value={packages}
                onChange={e => setPackages(e.target.value)}
              />
            </Field>
          </div>
        </Section>

        <Section title="Display">
          <div className="flex flex-col gap-4 p-4">
            {!initial && gpuErrors.map(message => <Alert key={message}>{message}</Alert>)}
            <div className="flex flex-col gap-2">
              <label className="flex items-center gap-2.5 text-sm text-ink">
                <input type="checkbox" className="check" name="gpu_access" checked={profile.gpu_access}
                  disabled={!!initial || (!gpuAvailable && !profile.gpu_access)}
                  onChange={e => setProfile(p => ({ ...p, gpu_access: e.target.checked, gpu_id: p.gpu_id ?? gpus[0]?.id ?? null, software_encoding: p.software_encoding || !e.target.checked }))} />
                GPU access
              </label>
              <p className="text-xs text-ink-3">{initial ? 'GPU access is set at creation.' : gpuAvailable ? 'Let the desktop and applications use the host GPU. Set at creation.' : 'No host GPU is available.'}</p>
              {profile.gpu_access && <Field label="GPU" hint="The selected GPU renders and encodes the desktop. Set at creation.">
                <select className="select select-md w-full" name="gpu_id" disabled={!!initial}
                  value={profile.gpu_id ?? ''} onChange={e => change('gpu_id', e.target.value)}>
                  {(initial?.gpu ? [initial.gpu] : gpus).map(g => <option key={g.id} value={g.id}>{g.driver} · {g.id} · {g.node}</option>)}
                </select>
              </Field>}
              <label className="flex items-center gap-2.5 text-sm text-ink">
                <input type="checkbox" className="check" name="software_encoding" checked={profile.software_encoding || !profile.gpu_access}
                  disabled={!profile.gpu_access} onChange={e => change('software_encoding', e.target.checked)} />
                Software video encoding
              </label>
              <p className="text-xs text-ink-3">Use CPU encoders for the viewer stream. The desktop runs at 30 Hz. Applies on Start or Relaunch; required without GPU access.</p>
            </div>
            <Field label="Screen size">
              <select className="select select-md w-full" value={screen} onChange={e => setScreen(e.target.value)}>
                <option value="dynamic">Dynamic</option>
                {screenPresets.map(size => (
                  <option key={size} value={size}>
                    {size.replace('x', ' × ')}
                  </option>
                ))}
                <option value="custom">Custom</option>
              </select>
            </Field>
            {screen === 'custom' && (
              <div className="flex gap-4">
                <Field label="Width" className="min-w-0 flex-1">
                  <input className="input" type="number" required min={2} max={8192} step={2} value={width} onChange={e => setWidth(e.target.value)} />
                </Field>
                <Field label="Height" className="min-w-0 flex-1">
                  <input className="input" type="number" required min={2} max={8192} step={2} value={height} onChange={e => setHeight(e.target.value)} />
                </Field>
              </div>
            )}
            <label className="flex items-center gap-2.5 text-sm text-ink">
              <input type="checkbox" className="check" checked={profile.kiosk} onChange={e => change('kiosk', e.target.checked)} />
              Kiosk mode
            </label>
          </div>
        </Section>

        <Section title="Startup">
          <div className="p-4">
            <Field label="Startup command">
              <textarea
                className="input min-h-14 resize-y py-2 font-mono text-xs"
                rows={2}
                maxLength={4096}
                placeholder="0ad"
                value={profile.startup_command}
                onChange={e => change('startup_command', e.target.value)}
              />
            </Field>
          </div>
        </Section>

        {(administrator || initial) && (
          <Disclosure label="Advanced Docker Options">
            <div className="p-4">
              <Field
              label="Docker options"
              hint={
                initial
                  ? 'Docker options are set at creation. Create a new session to change them.'
                  : 'Optional. One --flag=value per line. Supports --security-opt, --cap-add, and --cap-drop. Repeated options are allowed.'
              }
            >
              <textarea
                className="input min-h-20 resize-y py-2 font-mono text-xs"
                name="docker_args"
                rows={4}
                readOnly={!!initial}
                value={dockerArgs}
                onChange={e => setDockerArgs(e.target.value)}
                placeholder={'--security-opt=seccomp=unconfined\n--security-opt=apparmor=unconfined\n--cap-add=SYS_ADMIN'}
              />
              </Field>
            </div>
          </Disclosure>
        )}

        {error && <Alert>{error}</Alert>}

        <FormActions note={blocked || note}>
          <Link to={cancelTo} className="btn btn-outline btn-sm">
            Cancel
          </Link>
          <button type="submit" className="btn btn-primary btn-sm" disabled={!!blocked}>
            {initial ? 'Save Changes' : 'Create Session'}
          </button>
        </FormActions>
      </fieldset>
    </form>
  );
}
