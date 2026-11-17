<script lang="ts">
  import { tick, untrack } from 'svelte';
  import PagedFooter from '$lib/ui/PagedFooter.svelte';
  import { api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import { pagedQuery } from '$lib/api/paged.svelte';
  import { canonical } from '$lib/config/form';
  import { clock } from '$lib/clock.svelte';
  import { count, relative, setHint, usd } from '$lib/format';
  import Badge from '$lib/ui/Badge.svelte';
  import Button from '$lib/ui/Button.svelte';
  import BarChart from '$lib/ui/BarChart.svelte';
  import Card from '$lib/ui/Card.svelte';
  import Disclosure from '$lib/ui/Disclosure.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Input from '$lib/ui/Input.svelte';
  import KeyValue from '$lib/ui/KeyValue.svelte';
  import Progress from '$lib/ui/Progress.svelte';
  import SaveBar from '$lib/ui/SaveBar.svelte';
  import Select from '$lib/ui/Select.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import Table from '$lib/ui/Table.svelte';
  import { confirm } from '$lib/ui/confirm.svelte';
  import { toast } from '$lib/ui/toast.svelte';
  import type { components, operations } from '$lib/api/schema';

  type SettingsView = components['schemas']['SettingsView'];
  type Solve = components['schemas']['CaptchaSolve'];

  const POLICY_OPTIONS = [
    { value: 'off', label: 'Off' },
    { value: 'on_demand', label: 'On demand', hint: 'Detect only; solve when an agent asks' },
    { value: 'auto', label: 'Auto', hint: 'Detect and solve' },
    { value: 'human', label: 'Human', hint: 'Detect, then wait for a person' },
  ];
  const LIMIT = 50;
  const KEY_URL = 'https://2captcha.com/enterpage';
  const SOLVING = { off: 'not solving', on_demand: 'solving when an agent asks', auto: 'auto-solving', human: 'handing captchas to a person' };
  const TYPE_LABELS: Record<string, string> = {
    recaptcha_v2: 'reCAPTCHA v2',
    recaptcha_v3: 'reCAPTCHA v3',
    hcaptcha: 'hCaptcha',
    turnstile: 'Turnstile',
    funcaptcha: 'FunCaptcha',
    geetest: 'GeeTest',
    image: 'Image',
  };

  let { dirty = $bindable(false) }: { dirty?: boolean } = $props();

  const settings = liveQuery(() => unwrap(api.GET('/v1/captcha/settings')), { kinds: ['captcha.*'] });
  const stats = liveQuery(async () => (await unwrap(api.GET('/v1/captcha/stats', { params: { query: { days: 7 } } }))).days, { kinds: ['captcha.*'] });
  const general = liveQuery(() => unwrap(api.GET('/v1/settings')), { kinds: ['settings.changed'] });

  let policy = $state('auto');
  let dailyUsd = $state<string | number>('');
  let dailySolves = $state<number | null>(null);
  let humanWait = $state<number | null>(null);
  let providerUrl = $state('');
  let providerBaseline = $state('');
  let providerLoaded = $state(false);
  let apiKeyInput = $state('');
  let replacing = $state(false);
  let snapshot = $state('');
  let saving = $state(false);
  let removing = $state(false);
  let balance = $state<{ value: number } | { error: string } | null>(null);

  let view = $state<SettingsView>();
  const keySet = $derived(view?.api_key?.set === true);
  const keyHint = $derived(view?.api_key?.hint ?? null);

  const form = $derived(
    canonical({
      policy,
      dailyUsdMicros: Math.round(Number(dailyUsd) * 1_000_000),
      dailySolves,
      humanWait,
    }),
  );
  const isDirty = $derived((snapshot !== '' && form !== snapshot) || (providerLoaded && providerUrl !== providerBaseline) || apiKeyInput !== '');
  $effect(() => {
    dirty = isDirty;
  });

  const sources = $derived(general.data?.sources ?? {});
  const isEnv = (key: string) => sources[key] === 'env';
  const envName = (key: string) => 'BRR_' + key.replace('.', '__').toUpperCase();
  const badge = (key: string) => (isEnv(key) ? `Set by ${envName(key)}` : undefined);
  const envHint = (key: string, hint?: string) => (isEnv(key) ? 'Remove the environment variable to edit it here.' : hint);
  const dailyUsdError = $derived.by(() => {
    const text = String(dailyUsd).trim();
    const amount = Number(text);
    return text === '' || !Number.isFinite(amount) || amount < 0 ? 'Enter an amount, 0 or more.' : null;
  });
  const dailySolvesError = $derived(
    dailySolves === null || !Number.isInteger(dailySolves) || dailySolves < 0 ? 'Enter a whole number, 0 or more.' : null,
  );
  const budgetInvalid = $derived(dailyUsdError !== null || dailySolvesError !== null);

  const spendDays = $derived(stats.data ?? []);
  const spendPoints = $derived(
    spendDays.map((d) => ({
      x: new Date(d.start_ms).toLocaleDateString(undefined, { weekday: 'short' }),
      values: { spend: d.spend_usd_micros / 1_000_000 },
    })),
  );
  const spendTotal = $derived(spendDays.reduce((sum, d) => sum + d.spend_usd_micros, 0));
  const spendAttempts = $derived(spendDays.reduce((sum, d) => sum + d.solves + d.failures, 0));

  function apply(next: SettingsView) {
    view = next;
    policy = next.default_policy;
    dailyUsd = (next.daily_usd_micros / 1_000_000).toFixed(2);
    dailySolves = next.daily_solves;
    humanWait = next.human_wait_s;
    snapshot = canonical({
      policy,
      dailyUsdMicros: next.daily_usd_micros,
      dailySolves: next.daily_solves,
      humanWait: next.human_wait_s,
    });
  }

  $effect(() => {
    const next = settings.data;
    if (!next) return;
    if (untrack(() => isDirty)) view = next;
    else apply(next);
  });

  $effect(() => {
    const next = general.data;
    if (!next) return;
    if (untrack(() => providerLoaded && providerUrl !== providerBaseline)) return;
    providerUrl = next.settings.captcha.provider_url;
    providerBaseline = providerUrl;
    providerLoaded = true;
  });

  $effect(() => {
    void keyHint;
    if (!keySet) {
      balance = null;
      return;
    }
    let stale = false;
    void unwrap(api.GET('/v1/captcha/balance')).then(
      (out) => {
        if (!stale) balance = { value: out.balance_usd_micros };
      },
      (e) => {
        if (!stale) balance = { error: e instanceof Error ? e.message : 'Balance unavailable' };
      },
    );
    return () => {
      stale = true;
    };
  });

  async function save() {
    if (saving || !isDirty || budgetInvalid) return;
    saving = true;
    try {
      const current = view;
      const body: operations['captcha.settings_set']['requestBody']['content']['application/json'] = {};
      if (current) {
        if (policy !== current.default_policy) body.default_policy = policy as SettingsView['default_policy'];
        if (Math.round(Number(dailyUsd) * 1_000_000) !== current.daily_usd_micros) body.daily_usd_micros = Math.round(Number(dailyUsd) * 1_000_000);
        if (dailySolves !== null && dailySolves !== current.daily_solves) body.daily_solves = dailySolves;
        if (humanWait !== null && humanWait !== current.human_wait_s) body.human_wait_s = humanWait;
      }
      if (apiKeyInput) body.api_key = apiKeyInput;
      if (Object.keys(body).length > 0) {
        const next = await unwrap(api.PUT('/v1/captcha/settings', { body }));
        apply(next);
      }
      if (providerLoaded && providerUrl !== providerBaseline && !isEnv('captcha.provider_url')) {
        await unwrap(api.PATCH('/v1/settings', { body: { 'captcha.provider_url': providerUrl } }));
        providerBaseline = providerUrl;
        general.reload();
      }
      apiKeyInput = '';
      replacing = false;
      settings.reload();
      toast.success('Settings saved');
    } catch (e) {
      toast.apiError(e, "Couldn't save the captcha settings");
    } finally {
      saving = false;
    }
  }

  function discard() {
    apiKeyInput = '';
    replacing = false;
    providerUrl = providerBaseline;
    if (view) apply(view);
  }

  async function startReplace() {
    replacing = true;
    await tick();
    document.getElementById('captcha-api-key')?.focus();
  }

  async function cancelReplace() {
    replacing = false;
    apiKeyInput = '';
    await tick();
    document.getElementById('replace-key')?.focus();
  }

  async function removeKey() {
    const ok = await confirm({
      title: 'Remove the API key?',
      description: 'Automatic solving stops. Captcha solving turns off.',
      confirmLabel: 'Remove API key',
      tone: 'danger',
    });
    if (!ok || removing) return;
    removing = true;
    try {
      const next = await unwrap(api.PUT('/v1/captcha/settings', { body: { api_key: null } }));
      apiKeyInput = '';
      replacing = false;
      apply(next);
      settings.reload();
      toast.success('API key removed');
    } catch (e) {
      toast.apiError(e, "Couldn't remove the API key");
    } finally {
      removing = false;
    }
  }

  const timeZone = new Intl.DateTimeFormat(undefined, { timeZoneName: 'short' }).formatToParts().find((part) => part.type === 'timeZoneName')?.value;

  const meterTone = (pctValue: number) => (pctValue >= 100 ? 'danger' : pctValue >= 80 ? 'warning' : 'success');
  const pct = (part: number, limit: number) => (limit > 0 ? Math.min(100, (part / limit) * 100) : part > 0 ? 100 : 0);
  const solveDuration = (ms: number) =>
    ms < 1000 ? `${(ms / 1000).toFixed(1)} s` : ms < 60_000 ? `${Math.floor(ms / 1000)}s` : `${Math.floor(ms / 60_000)}m ${Math.floor((ms % 60_000) / 1000)}s`;
  const spentPct = $derived(view ? pct(view.spent_today_usd_micros, view.daily_usd_micros) : 0);
  const solvesPct = $derived(view ? pct(view.solves_today, view.daily_solves) : 0);

  const solves = pagedQuery(
    async (cursor) => {
      const page = await unwrap(api.GET('/v1/captcha/solves', { params: { query: { limit: LIMIT, cursor } } }));
      return { items: page.solves, next: page.next_cursor };
    },
    { kinds: ['captcha.*'], key: (solve) => solve.id, moreError: "Couldn't load more solves" },
  );
</script>

{#if settings.loading && !view}
  <div aria-busy="true" class="stack">
    <Skeleton width="240px" height="28px" />
    <Skeleton height="160px" radius="var(--radius-lg)" />
    <Skeleton height="160px" radius="var(--radius-lg)" />
  </div>
{:else if settings.error && !view}
  <EmptyState tone="danger" title="Couldn't load captcha settings" description={settings.error.message}>
    {#snippet action()}<Button onclick={() => settings.reload()}>Retry</Button>{/snippet}
  </EmptyState>
{:else if view}
  <div class="stack">
    <p class="status" data-testid="captcha-status">
      {#if keySet && view.default_policy === 'off'}
        <strong>Off</strong> — the default policy is Off. Change it below to turn solving on.
      {:else if keySet}
        <strong>On</strong> · {SOLVING[view.default_policy]} · {usd(view.spent_today_usd_micros)} of {usd(view.daily_usd_micros)} today · resets 00:00 {timeZone}
      {:else}
        <strong>Off</strong> — add a 2captcha key to turn it on (<a href={KEY_URL} target="_blank" rel="noreferrer">Get a key ↗</a>)
      {/if}
    </p>

    <Card title="Provider">
      <div class="fields">
        <KeyValue items={[{ label: 'Provider', value: view.provider }]} />

        <div class="field">
          <span class="field-label">API key</span>
          {#if keySet && !replacing}
            <div class="key-row">
              <span>{setHint(keyHint)}</span>
              <Button id="replace-key" variant="ghost" size="sm" onclick={startReplace}>Replace</Button>
              <Button variant="ghost" size="sm" loading={removing} onclick={removeKey}>Remove…</Button>
            </div>
          {:else}
            <div class="key-input">
              <Input
                id="captcha-api-key"
                label="API key"
                hideLabel
                type="password"
                autocomplete="off"
                bind:value={apiKeyInput}
                hint={keySet ? undefined : 'Paste your 2captcha API key.'}
              />
            </div>
            {#if keySet}
              <div><Button variant="ghost" size="sm" onclick={cancelReplace}>Cancel</Button></div>
            {/if}
          {/if}
        </div>

        <KeyValue
          items={[
            {
              label: 'Balance',
              value: !keySet ? '—' : balance === null ? '…' : 'error' in balance ? balance.error : usd(balance.value),
              tone: balance && 'error' in balance ? ('danger' as const) : undefined,
            },
          ]}
        />
      </div>
    </Card>

    <Card title="Defaults">
      <div class="fields">
        <Select
          name="captcha.default_policy"
          label="Default policy"
          bind:value={policy}
          options={POLICY_OPTIONS}
          disabled={!keySet}
          hint={keySet ? POLICY_OPTIONS.find((o) => o.value === policy)?.hint : 'Add a key first.'}
        />
      </div>
    </Card>

    <Card title="Budget">
      <div class="fields">
        <p class="note">Every solve is charged to your 2captcha account. These limits cap what brr spends per day.</p>
        <div class="pair">
          <Input name="captcha.daily_usd" label="Daily spend limit (USD)" type="number" min={0} step={0.01} bind:value={dailyUsd} error={dailyUsdError} />
          <Input name="captcha.daily_solves" label="Daily solve limit" type="number" min={0} bind:value={dailySolves} error={dailySolvesError} />
        </div>

        <div class="meter">
          <div class="meter-head">
            <span class="field-label">Spent today</span>
            <span class="num">{usd(view.spent_today_usd_micros)} of {usd(view.daily_usd_micros)}</span>
          </div>
          <Progress value={spentPct} size="md" tone={meterTone(spentPct)} label="Spent today" />
        </div>

        <div class="meter">
          <div class="meter-head">
            <span class="field-label">Solves today</span>
            <span class="num">{count(view.solves_today)} of {count(view.daily_solves)}</span>
          </div>
          <Progress value={solvesPct} size="md" tone={meterTone(solvesPct)} label="Solves today" />
        </div>

        {#if spentPct >= 100 || solvesPct >= 100}
          <p class="danger-text">Budget used up. New captchas go to a person until midnight.</p>
        {/if}
      </div>
    </Card>

    <Card title="Spend" description="Last 7 days">
      {#if stats.error && !stats.data}
        <EmptyState tone="danger" compact title="Couldn't load captcha spend" description={stats.error.message}>
          {#snippet action()}<Button size="sm" onclick={() => stats.reload()}>Retry</Button>{/snippet}
        </EmptyState>
      {:else if stats.loading}
        <div aria-busy="true"><Skeleton height="200px" radius="var(--radius-md)" /></div>
      {:else if spendAttempts === 0 && spendTotal === 0}
        <EmptyState compact title="No captchas were solved in the last 7 days." />
      {:else}
        <BarChart label="Captcha spend per day" series={[{ key: 'spend', label: 'Spend' }]} data={spendPoints} format={(n) => usd(n * 1_000_000)} />
      {/if}
    </Card>

    <Disclosure summary="Advanced">
      {#if general.loading && !providerLoaded}
        <div aria-busy="true"><Skeleton height="64px" radius="var(--radius-md)" /></div>
      {:else if general.error && !providerLoaded}
        <EmptyState tone="danger" title="Couldn't load advanced settings" description={general.error.message}>
          {#snippet action()}<Button onclick={() => general.reload()}>Retry</Button>{/snippet}
        </EmptyState>
      {:else}
        <div class="pair">
          <Input name="captcha.provider_url" label="Provider URL" type="url" bind:value={providerUrl} readonly={isEnv('captcha.provider_url')} badge={badge('captcha.provider_url')} hint={envHint('captcha.provider_url', 'Where solve requests are sent.')} />
          <Input name="captcha.human_wait_s" label="How long to wait for a person (s)" type="number" min={0} bind:value={humanWait} />
        </div>
      {/if}
    </Disclosure>

    {#snippet more()}
      <PagedFooter list={solves} />
    {/snippet}

    <Card title="Solve log" flush>
      <Table
        caption="Captcha solves"
        columns={[
          { key: 'time', label: 'Time', width: '14%' },
          { key: 'site', label: 'Site', width: '24%' },
          { key: 'type', label: 'Type', width: '17%' },
          { key: 'result', label: 'Result', width: '14%' },
          { key: 'cost', label: 'Cost', align: 'right', width: '13%' },
          { key: 'duration', label: 'Duration', align: 'right', width: '18%' },
        ]}
        rows={solves.items}
        key={(row: Solve) => row.id}
        loading={solves.loading}
        error={solves.error}
        onretry={() => solves.reload()}
        empty={{ title: 'No captchas solved yet' }}
        footer={solves.hasMore || solves.capped ? more : undefined}
        flush
      >
        {#snippet row(solve: Solve)}
          <td>{relative(solve.ts, clock.now)}</td>
          <td class="truncate" title={solve.site}>{solve.site}</td>
          <td class="truncate">{TYPE_LABELS[solve.type] ?? solve.type}</td>
          <td>
            {#if solve.result === 'solved'}
              <Badge tone="success">Solved</Badge>
            {:else}
              <span title={solve.error ?? undefined}><Badge tone="danger">Failed</Badge></span>
            {/if}
          </td>
          <td class="num right">{usd(solve.cost_usd_micros)}</td>
          <td class="num right">{solveDuration(solve.duration_ms)}</td>
        {/snippet}
      </Table>
    </Card>
  </div>
  <SaveBar dirty={isDirty} {saving} disabled={budgetInvalid} onsave={save} ondiscard={discard} />
{/if}

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }

  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    align-items: flex-start;
  }

  .key-input {
    width: min(480px, 100%);
  }

  .key-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .pair {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--space-3);
  }

  .meter {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .meter-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .status {
    margin: 0;
    color: var(--text-secondary);
  }

  .status strong {
    color: var(--text);
  }

  .status a {
    text-decoration: underline;
  }

  .note {
    margin: 0;
    color: var(--text-secondary);
  }

  .danger-text {
    color: var(--danger-fg);
  }

  .right {
    text-align: right;
  }

  @media (max-width: 1023px) {
    .pair {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
