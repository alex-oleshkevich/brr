import { type ChildProcess, spawn } from 'node:child_process';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, rmSync, symlinkSync } from 'node:fs';
import { createServer } from 'node:net';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';

const DAEMON_START_TIMEOUT_MS = 20_000;
const DAEMON_STOP_TIMEOUT_MS = 10_000;
const POLL_MS = 200;
const SCRATCH_PARENT = join(process.env.XDG_CACHE_HOME ?? join(homedir(), '.cache'), 'brr-e2e');
const DEFAULT_BIN = resolve(import.meta.dirname, '../../../target/debug/brr');
const SEED_PROFILE = 'e2e-seed';
const SEED_TEMPLATE = 'chromium-headless';
const FINISHED_PROFILE = 'e2e-finished';
const FINAL_STATES = 'stopped,failed,detached,lost';

export type Seed = { profileId: string; instanceId: string; tabId: string; runId: string };

async function freePort(): Promise<number> {
  return new Promise((done, fail) => {
    const server = createServer();
    server.once('error', fail);
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address() as { port: number };
      server.close(() => done(port));
    });
  });
}

async function until<T>(what: string, timeoutMs: number, probe: () => Promise<T | null | undefined>): Promise<T> {
  const end = Date.now() + timeoutMs;
  for (;;) {
    const value = await probe().catch(() => null);
    if (value) return value;
    if (Date.now() > end) throw new Error(`${what} did not happen within ${timeoutMs} ms`);
    await new Promise((r) => setTimeout(r, POLL_MS));
  }
}

async function call<T>(base: string, method: string, path: string, body?: unknown): Promise<T> {
  const response = await fetch(base + path, {
    method,
    headers: { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await response.text();
  if (!response.ok) throw new Error(`${method} ${path}: ${response.status} ${text}`);
  return JSON.parse(text) as T;
}

type Instance = { id: string; state: string };

// Profile, running Instance and its Tab for the routes that need one; reuses them when they exist.
async function seed(base: string): Promise<Seed> {
  await call(base, 'POST', '/v1/builds/detect', {});
  const { profiles } = await call<{ profiles: { id: string; name: string }[] }>(base, 'GET', '/v1/profiles');
  const profile =
    profiles.find((p) => p.name === SEED_PROFILE) ??
    (await call<{ id: string }>(base, 'POST', '/v1/profiles', { name: SEED_PROFILE, template: SEED_TEMPLATE }));

  const { instances } = await call<{ instances: Instance[] }>(base, 'GET', `/v1/instances?profile=${profile.id}&state=running`);
  const instance =
    instances[0] ??
    (await call<{ instance: Instance }>(base, 'POST', `/v1/profiles/${profile.id}/launch`, { display: 'headless' })).instance;

  const tab = await until('the seed Tab', DAEMON_START_TIMEOUT_MS, async () => {
    const { tabs } = await call<{ tabs: { id: string; state: string }[] }>(base, 'GET', `/v1/instances/${instance.id}/tabs`);
    return tabs.find((t) => t.state === 'ready');
  });

  return { profileId: profile.id, instanceId: instance.id, tabId: tab.id, runId: await finishedRun(base) };
}

// A finished run for /history/:run; reuses one of the Profile's when it exists.
async function finishedRun(base: string): Promise<string> {
  const { profiles } = await call<{ profiles: { id: string; name: string }[] }>(base, 'GET', '/v1/profiles');
  const profile =
    profiles.find((p) => p.name === FINISHED_PROFILE) ??
    (await call<{ id: string }>(base, 'POST', '/v1/profiles', { name: FINISHED_PROFILE, template: SEED_TEMPLATE }));

  const done = await call<{ instances: Instance[] }>(base, 'GET', `/v1/instances?profile=${profile.id}&state=${FINAL_STATES}&limit=1`);
  if (done.instances[0]) return done.instances[0].id;

  const { instance } = await call<{ instance: Instance }>(base, 'POST', `/v1/profiles/${profile.id}/launch`, { display: 'headless' });
  await until('the finished-run browser running', DAEMON_START_TIMEOUT_MS, async () => {
    const { tabs } = await call<{ tabs: { state: string }[] }>(base, 'GET', `/v1/instances/${instance.id}/tabs`);
    return tabs.some((t) => t.state === 'ready');
  });
  await call(base, 'POST', `/v1/instances/${instance.id}/stop`, {});

  return until('the finished-run browser stopped', DAEMON_START_TIMEOUT_MS, async () => {
    const out = await call<Instance>(base, 'GET', `/v1/instances/${instance.id}`);
    return FINAL_STATES.split(',').includes(out.state) ? out.id : null;
  });
}

function startDaemon(): Promise<{ url: string; stop: () => Promise<void> }> {
  return freePort().then(async (port) => {
    const bin = process.env.BRR_BIN ?? DEFAULT_BIN;
    if (!existsSync(bin)) throw new Error(`${bin} is missing; run just build (or set BRR_BIN)`);

    // Under ~/.cache, not /tmp: /tmp has a per-user quota.
    mkdirSync(SCRATCH_PARENT, { recursive: true });
    const root = mkdtempSync(join(SCRATCH_PARENT, 'run-'));
    chmodSync(root, 0o700);
    for (const dir of ['bin', 'config', 'data', 'state', 'runtime']) mkdirSync(join(root, dir));

    // The daemon finds a system Chrome by name on PATH.
    const chrome = process.env.BRR_TEST_CHROME;
    if (chrome) symlinkSync(chrome, join(root, 'bin', 'google-chrome-stable'));

    const url = `http://127.0.0.1:${port}`;
    const child: ChildProcess = spawn(bin, ['daemon', 'run'], {
      env: {
        ...process.env,
        PATH: `${join(root, 'bin')}:${process.env.PATH}`,
        XDG_CONFIG_HOME: join(root, 'config'),
        XDG_DATA_HOME: join(root, 'data'),
        XDG_STATE_HOME: join(root, 'state'),
        XDG_RUNTIME_DIR: join(root, 'runtime'),
        BRR_SERVER__BIND: `127.0.0.1:${port}`,
        BRR_VAULT__KEY_SOURCE: 'file',
      },
      stdio: 'ignore',
    });

    const stop = async () => {
      if (child.exitCode === null) {
        child.kill('SIGTERM');
        const exited = new Promise((done) => child.once('exit', done));
        const timeout = new Promise((done) => setTimeout(done, DAEMON_STOP_TIMEOUT_MS));
        await Promise.race([exited, timeout]);
        if (child.exitCode === null) child.kill('SIGKILL');
      }
      rmSync(root, { recursive: true, force: true });
    };

    try {
      await until('the daemon answering', DAEMON_START_TIMEOUT_MS, async () => (await fetch(`${url}/v1/system/info`)).ok);
    } catch (e) {
      await stop();
      throw e;
    }

    return { url, stop };
  });
}

// BRR_E2E_URL points the suite at a daemon that is already running; otherwise a scratch one lives for the run.
export default async function globalSetup(): Promise<() => Promise<void>> {
  const external = process.env.BRR_E2E_URL;
  const daemon = external ? { url: external, stop: async () => {} } : await startDaemon();

  process.env.BRR_E2E_URL = daemon.url;
  try {
    process.env.BRR_E2E_SEED = JSON.stringify(await seed(daemon.url));
  } catch (e) {
    await daemon.stop();
    throw e;
  }

  return daemon.stop;
}
