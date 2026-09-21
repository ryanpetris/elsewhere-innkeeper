// Run in proxy-browser with this script mounted at /src/scripts/check-docker-options-browser.mjs.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { chromium } from '../web/node_modules/playwright-core/index.mjs';

const intel = {id:'0000:00:02.0',driver:'i915',node:'/dev/dri/renderD128',major:226,minor:128};
const nvidia = {id:'0000:01:00.0',driver:'nvidia',node:'/dev/dri/renderD129',major:226,minor:129};
let gpus = [intel, nvidia];
let gpuErrors = [];
const dockerArgs = ['--security-opt=seccomp=unconfined', '--security-opt=apparmor=unconfined', '--cap-add=SYS_ADMIN'];
const session = {
  gpu: nvidia, gpu_id: nvidia.id, id: 'docker-options-fixture', name: 'Steam', distribution: 'ubuntu', packages: [],
  access_role: 'manager', docker_args: dockerArgs, status: 'stopped', screen_size: null, kiosk: false, software_encoding: true, gpu_access: true,
  startup_command: '', settings_pending: false, installed_version: '0.7.3-1', expected_version: '0.7.3',
  version_status: 'current', repair_available: false, port: 0, started_ms: 0, timings: {},
};
const server = createServer(async (request, response) => {
  try {
    const path = new URL(request.url, 'http://localhost').pathname;
    // Innkeeper serves the same document for every in-app path; the browser routes it.
    const file = path === '/app.js' || path === '/app.css' ? path : '/index.html';
    response.setHeader('Content-Type', file.endsWith('.js') ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : 'text/html');
    // The headers Innkeeper serves, so what the browser refuses there it refuses here too.
    response.setHeader('Referrer-Policy', 'no-referrer');
    response.setHeader('X-Content-Type-Options', 'nosniff');
    response.setHeader('Content-Security-Policy', "default-src 'self'; img-src 'self' blob: data:; style-src 'self' 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'");
    response.end(await readFile(new URL('../web/dist' + file, import.meta.url)));
  } catch {
    response.writeHead(404).end();
  }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const browser = await chromium.launch({ executablePath: '/usr/bin/chromium', headless: true, args: ['--no-sandbox'] });
try {
  const page = await browser.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => {
      if (/Content Security Policy|Refused to/i.test(message.text())) errors.push(`refused: ${message.text()}`);
    });
  await page.route('**/api/**', async route => {
    const request = route.request(), path = new URL(request.url()).pathname;
    if (path === '/api/me') {
      await route.fulfill({json: {user:{id:'fixture',username:'fixture',display_name:'Fixture',role:'administrator'},csrf_token:'fixture-csrf',session_expires_at_ms:Date.now()+7*86400000,server_time_ms:Date.now()}});
    } else if (path === '/api/sessions' && request.method() === 'GET') {
      await route.fulfill({ json: { sessions: [session], version: 'fixture', gpu_available: gpus.length > 0, gpus, gpu_errors: gpuErrors } });
    } else if (path === '/api/sessions' && request.method() === 'POST') {
      await route.fulfill({ status: 202, json: { ...session } });
    } else if (path === `/api/sessions/${session.id}/settings` && request.method() === 'PUT') {
      await route.fulfill({ json: { ...session, ...request.postDataJSON() } });
    } else if (path === '/api/users') {
      await route.fulfill({ json: { users: [] } });
    } else if (path.endsWith('/access')) {
      await route.fulfill({ json: { assignments: [] } });
    } else if (path.endsWith('/logs')) {
      await route.fulfill({ json: { text: 'fixture log' } });
    } else if (path.endsWith('/preview')) {
      await route.fulfill({ status: 404, body: '' });
    } else {
      errors.push(`Unexpected API request: ${request.method()} ${path}`);
      await route.fulfill({ status: 500, json: {} });
    }
  });
  const origin = `http://127.0.0.1:${server.address().port}`;
  await page.goto(origin);
  await page.getByRole('link', { name: 'New Session', exact: true }).click();
  const create = page.getByRole('heading', { name: 'New Session', exact: true });
  await create.waitFor();
  assert.equal(await page.getByRole('checkbox', {name:'GPU access',exact:true}).isChecked(), true);
  assert.equal(await page.getByRole('checkbox', {name:'Software video encoding',exact:true}).isChecked(), false);
  assert.equal(new URL(page.url()).pathname, '/sessions/new');
  assert.equal(await page.getByRole('combobox', {name:'GPU', exact:true}).inputValue(), intel.id);
  await page.getByRole('combobox', {name:'GPU', exact:true}).selectOption(nvidia.id);
  await page.getByRole('checkbox', {name:'GPU access',exact:true}).uncheck();
  await page.getByRole('checkbox', {name:'GPU access',exact:true}).check();
  assert.equal(await page.getByRole('combobox', {name:'GPU', exact:true}).inputValue(), nvidia.id);
  await page.getByText('Import Profile', { exact: true }).click();
  await page.getByText('Advanced Docker Options', { exact: true }).click();
  const options = page.locator('textarea[name="docker_args"]');
  async function importProfile(profile) {
    await page.getByLabel('Profile JSON').fill(JSON.stringify(profile));
    await page.getByRole('button', { name: 'Apply Profile', exact: true }).click();
  }
  for (const distro of ['arch', 'debian', 'ubuntu']) {
    await importProfile({ name: 'Distribution profile', distribution: distro });
    assert.equal(await page.locator('select[name=distribution]').inputValue(), distro);
  }
  await importProfile({ name: 'Steam', docker_args: dockerArgs, gpu_access: true, gpu_id:nvidia.id, software_encoding: true });
  assert.equal(await options.inputValue(), dockerArgs.join('\n'));
  assert.equal(await options.evaluate(element => element.readOnly), false);
  for (const invalid of ['--cap-add=SYS_ADMIN', ['--cap-add=SYS_ADMIN\n--cap-drop=NET_RAW']]) {
    await importProfile({ name: 'Invalid profile', docker_args: invalid });
    assert.equal(await page.locator('form').getByRole('alert').innerText(), 'Invalid profile field types.');
    assert.equal(await options.inputValue(), dockerArgs.join('\n'));
    assert.equal(await page.getByLabel('Session name').inputValue(), 'Steam');
  }
  for (const screen_size of [[], '1920x1080', { width: 1920 }, { width: '1920', height: 1080 }, { width: 1921, height: 1080 }, { width: 1920, height: 1080, extra: true }]) {
    await importProfile({ name: 'Invalid dimensions', screen_size });
    assert.equal(await page.locator('form').getByRole('alert').innerText(), 'Screen dimensions must be even numbers between 2 and 8192.');
    assert.equal(await page.getByLabel('Session name').inputValue(), 'Steam');
  }
  await importProfile({name:'Disabled GPU',gpu_access:false,gpu_id:nvidia.id});
  assert.equal(await page.locator('form').getByRole('alert').innerText(), 'GPU selection requires GPU access.');
  await importProfile({name:'Missing GPU', gpu_id:'not-a-device'});
  assert.equal(await page.locator('form').getByRole('alert').innerText(), 'Selected GPU is unavailable.');
  await importProfile({ name: 'Preset display', screen_size: { width: 1920, height: 1080 } });
  assert.equal(await page.locator('select').filter({ has: page.locator('option[value=dynamic]') }).inputValue(), '1920x1080');
  await importProfile({ name: 'Basic profile', distribution: 'ubuntu', gpu_access: false, software_encoding: false, screen_size: { width: 1366, height: 768 } });
  assert.equal(await page.locator('select').filter({ has: page.locator('option[value=dynamic]') }).inputValue(), 'custom');
  assert.equal(await page.getByLabel('Width', { exact: true }).inputValue(), '1366');
  assert.equal(await page.getByLabel('Height', { exact: true }).inputValue(), '768');
  assert.equal(await options.inputValue(), '');
  const basicRequest = page.waitForRequest(request => request.method() === 'POST' && new URL(request.url()).pathname === '/api/sessions');
  await page.getByRole('button', { name: 'Create Session', exact: true }).click();
  const basic = (await basicRequest).postDataJSON();
  assert.deepEqual(basic.docker_args, []);
  assert.deepEqual(basic.screen_size, { width: 1366, height: 768 });
  assert.equal(basic.gpu_access, false);
  assert.equal(basic.gpu_id, null);
  assert.equal(basic.software_encoding, true);
  assert.equal(basic.distribution, 'ubuntu');
  // Creating lands on the new session's own page.
  await page.getByRole('heading', { name: session.name, exact: true }).waitFor();
  assert.equal(new URL(page.url()).pathname, `/sessions/${session.id}`);
  await page.getByText('Ubuntu 26.04 LTS', { exact: true }).waitFor();

  await page.goto(origin + '/sessions/new');
  await page.getByText('Import Profile', { exact: true }).click();
  await page.getByText('Advanced Docker Options', { exact: true }).click();
  await importProfile({ name: 'Steam', docker_args: dockerArgs, gpu_access: true, gpu_id:nvidia.id, software_encoding: true });
  assert.equal(await options.inputValue(), dockerArgs.join('\n'));
  await options.fill(`  ${dockerArgs.join('\n\n')}  \n`);
  const createRequest = page.waitForRequest(request => request.method() === 'POST' && new URL(request.url()).pathname === '/api/sessions');
  await page.getByRole('button', { name: 'Create Session', exact: true }).click();
  const created = (await createRequest).postDataJSON();
  assert.deepEqual(created.docker_args, dockerArgs);
  assert.equal(created.gpu_access, true);
  assert.equal(created.gpu_id, nvidia.id);
  assert.equal(created.software_encoding, true);
  await page.getByRole('heading', { name: session.name, exact: true }).waitFor();

  // Settings are reached from the session, and keep the creation-time options read-only.
  await page.getByRole('link', { name: 'Edit Settings', exact: true }).click();
  await page.getByRole('heading', { name: 'Edit Settings', exact: true }).waitFor();
  assert.equal(new URL(page.url()).pathname, `/sessions/${session.id}/settings`);
  await page.getByText('Advanced Docker Options', { exact: true }).click();
  assert.equal(await options.inputValue(), dockerArgs.join('\n'));
  assert.equal(await options.evaluate(element => element.readOnly), true);
  assert.equal(await page.getByRole('checkbox', {name:'GPU access',exact:true}).isDisabled(), true);
  await page.getByRole('checkbox', {name:'Software video encoding',exact:true}).uncheck();
  await page.getByLabel('Session name').fill('Steam renamed');
  const saveRequest = page.waitForRequest(request => request.method() === 'PUT');
  await page.getByRole('button', { name: 'Save Changes', exact: true }).click();
  assert.deepEqual((await saveRequest).postDataJSON(), {
    name: 'Steam renamed', screen_size: null, kiosk: false, software_encoding: false, startup_command: '',
  });
  // Saving returns to the session.
  await page.getByRole('heading', { name: session.name, exact: true }).waitFor();
  assert.equal(new URL(page.url()).pathname, `/sessions/${session.id}`);
  gpus = [];
  gpuErrors = ['Cannot identify render device. Check host device and sysfs access.'];
  await page.goto(origin + '/sessions/new');
  const gpu = page.getByRole('checkbox', {name:'GPU access',exact:true});
  await gpu.waitFor();
  await page.getByText(gpuErrors[0], {exact:true}).waitFor();
  gpuErrors = [];
  await page.reload();
  await gpu.waitFor();
  assert.equal(await gpu.isDisabled(), true);
  assert.equal(await gpu.isChecked(), false);
  assert.equal(await page.getByRole('checkbox', {name:'Software video encoding',exact:true}).isChecked(), true);
  await page.getByText('Import Profile', {exact:true}).click();
  await importProfile({name:'Unavailable', gpu_access:true});
  assert.equal(await page.locator('form').getByRole('alert').innerText(), 'No host GPU is available.');
  await importProfile({name:'Unknown', unexpected:true});
  assert.equal(await page.locator('form').getByRole('alert').innerText(), 'Profile must be an object containing session settings only.');
  await importProfile({name:'Invalid', software_encoding:'true'});
  assert.equal(await page.locator('form').getByRole('alert').innerText(), 'Invalid profile field types.');
  assert.deepEqual(errors, []);
  await page.route('**/api/me', route => route.fulfill({status:503,json:{error:'unavailable'}}));
  await page.reload();
  await page.getByRole('button', {name:'Sign In',exact:true}).waitFor();
  assert.deepEqual(errors, []);
  console.log('Docker options: profile imports, default options, repeated creation arguments, read-only settings and Save payload passed');
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
