// JSON contracts served by src/main.rs, src/accounts.rs and src/gpu.rs.
export type AccountRole = 'administrator' | 'user';
export type AccessRole = 'viewer' | 'interactive' | 'manager';
export type Distribution = 'arch' | 'debian' | 'ubuntu';
export type SessionStatus = 'running' | 'preparing' | 'upgrading' | 'failed' | 'stopped' | 'cancelled';
export type SessionAction = 'start' | 'stop' | 'relaunch' | 'upgrade' | 'destroy';
export type Layout = 'grid' | 'list';

export interface User {
  id: string;
  username: string;
  display_name: string;
  role: AccountRole;
  enabled: boolean;
  created_at_ms: number;
}
export interface SessionClock {
  session_expires_at_ms: number;
  server_time_ms: number;
}
export interface LoginResponse extends SessionClock {
  user: User;
  csrf_token: string;
}
export interface LoginInput {
  username: string;
  password: string;
  display_name?: string;
}
export interface UserInput {
  username: string;
  display_name: string;
  role: AccountRole;
  password: string;
}
export type UserUpdate = Omit<UserInput, 'password'> & { enabled: boolean };
export type SessionSettings = Omit<SessionProfile, 'distribution'>;
export interface Gpu {
  id: string;
  driver: string;
  node: string;
  major: number;
  minor: number;
}
export interface ScreenSize { width: number; height: number }
export interface SessionProfile {
  name: string;
  distribution: Distribution;
  packages: string[];
  docker_args: string[];
  gpu_access: boolean;
  gpu_id: string | null;
  software_encoding: boolean;
  startup_command: string;
  screen_size: ScreenSize | null;
  kiosk: boolean;
}
interface SessionSummary {
  id: string;
  name: string;
  distribution: Distribution;
  status: SessionStatus;
  stage: string;
  installed_version: string | null;
  expected_version: string;
  version_status: 'older' | 'newer' | 'current' | 'unknown';
}
interface ManagerFields extends Omit<SessionProfile, 'name' | 'distribution'> {
  gpu: Gpu | null;
  nvidia: boolean;
  settings_pending: boolean;
  repair_available: boolean;
  version_error: string | null;
  port: number;
  started_ms: number;
  error: string | null;
  timings: Record<string, number>;
}
export type ManagedSession = SessionSummary & ManagerFields & { access_role: 'manager' };
export type Session = ManagedSession | (SessionSummary & {
  access_role: 'viewer' | 'interactive';
} & Partial<Record<keyof ManagerFields, never>>);
export interface SessionsResponse {
  sessions: Session[];
  version: string;
  local_elsewhere: boolean;
  gpu_available: boolean;
  gpus: Gpu[];
  gpu_errors: string[];
}
export interface Assignment { user_id: string; display_name: string; role: AccessRole }
export type Api = (path: string, options?: RequestInit) => Promise<Response>;
export interface SessionActions {
  busy: Record<string, boolean>;
  onOpen: (session: Session) => void;
  onAction: (session: Session, action: SessionAction) => Promise<void>;
}

// The server owns these response shapes. Keep JSON's untyped value at this boundary.
export async function readJson<T>(response: Response): Promise<T> {
  return response.json() as Promise<T>;
}
export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
export function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}
export async function responseError(response: Response): Promise<{ error?: string; message?: string }> {
  const body: unknown = await response.json().catch(() => null);
  return isRecord(body) ? {
    error: typeof body.error === 'string' ? body.error : undefined,
    message: typeof body.message === 'string' ? body.message : undefined,
  } : {};
}
export function formText(form: HTMLFormElement, name: string): string {
  const value = new FormData(form).get(name);
  if (typeof value !== 'string') throw new Error(`Missing form field: ${name}`);
  return value;
}

export function formAccountRole(form: HTMLFormElement): AccountRole {
  const role = formText(form, 'role');
  if (role !== 'user' && role !== 'administrator') throw new Error('Invalid account role.');
  return role;
}
