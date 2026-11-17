<script lang="ts">
  import { attention, hostOf, REASON_VERBS } from '$lib/attention/attention.svelte';
  import { clock, everySecond } from '$lib/clock.svelte';
  import { countdown, endReason, plainError } from '$lib/events/summary';
  import { bytes, relative, usd } from '$lib/format';
  import { outcome, profileLabel } from '$lib/instances/actions';
  import { runHref } from '$lib/runs/runs';
  import { settingsHref } from '$lib/settings/sections';
  import Badge from '$lib/ui/Badge.svelte';
  import Button from '$lib/ui/Button.svelte';
  import Card from '$lib/ui/Card.svelte';
  import type { OverviewData } from './data.svelte';

  type Row = { key: string; tone: 'warning' | 'danger'; title: string; detail: string; action: string; href: string; dismissible?: boolean };

  const BUDGET_WARN = 0.8;
  const BALANCE_DAYS = 3;
  const STATS_DAYS = 7;

  let { data }: { data: OverviewData } = $props();

  let now = $state(Date.now());
  $effect(() => {
    if (!attention.needsYou.length) return;

    return everySecond((tick) => (now = tick));
  });

  const profileName = (id: string) => data.profiles.data?.find((p) => p.id === id)?.name ?? 'A profile';

  const rows = $derived.by<Row[]>(() => {
    const out: Row[] = attention.needsYou.map((n) => ({
      key: `tab:${n.tabId}`,
      tone: 'warning',
      title: `${n.agent} needs you to ${REASON_VERBS[n.reason]} on ${hostOf(n.url)}`,
      detail: countdown(n.deadline, now),
      action: 'Open',
      href: `/browsers/${n.instanceId}?tab=${n.tabId}`,
    }));

    // A crashed or failed browser stays until a person dismisses it; the dismissal survives reloads.
    for (const i of attention.failures) {
      const key = `instance:${i.id}`;
      // A failed attach already showed its error in the attach dialog.
      if (attention.dismissed.has(key) || (i.kind === 'live' && !i.started_at)) continue;
      out.push({
        key,
        tone: 'danger',
        title: `${profileLabel(i)} ${outcome(i)?.label === 'Crashed' ? 'crashed' : "couldn't start"}`,
        detail: `${relative(i.ended_at ?? i.last_activity_at, clock.now)} · ${i.error ? plainError(i.error) : i.end_reason ? endReason(i.end_reason) : 'No reason recorded'}`,
        action: 'Open',
        href: runHref(i),
        dismissible: true,
      });
    }

    for (const p of data.profiles.data ?? []) {
      if (p.login_status !== 'failed') continue;
      out.push({
        key: `login:${p.id}`,
        tone: 'danger',
        title: `A login check failed for ${p.name}`,
        detail: 'The profile may be signed out',
        action: 'Open',
        href: `/profiles/${p.id}#logins`,
      });
    }

    const captcha = data.captcha.data;
    if (captcha) {
      const usdRatio = captcha.daily_usd_micros > 0 ? captcha.spent_today_usd_micros / captcha.daily_usd_micros : 0;
      const solveRatio = captcha.daily_solves > 0 ? captcha.solves_today / captcha.daily_solves : 0;
      if (Math.max(usdRatio, solveRatio) >= BUDGET_WARN) {
        out.push({
          key: 'captcha-budget',
          tone: 'warning',
          title: Math.max(usdRatio, solveRatio) >= 1 ? 'The captcha budget for today is used up' : 'The captcha budget for today is almost used up',
          detail: `${usd(captcha.spent_today_usd_micros)} of ${usd(captcha.daily_usd_micros)} · ${captcha.solves_today} of ${captcha.daily_solves} solves`,
          action: 'Fix',
          href: settingsHref('captcha'),
        });
      }
    }

    const balance = data.balance.data;
    const days = data.stats.data;
    if (balance && days) {
      const average = days.slice(-STATS_DAYS).reduce((sum, d) => sum + d.spend_usd_micros, 0) / STATS_DAYS;
      if (average > 0 && balance.balance_usd_micros < average * BALANCE_DAYS) {
        out.push({
          key: 'captcha-balance',
          tone: 'warning',
          title: 'The captcha provider balance is running low',
          detail: `${usd(balance.balance_usd_micros)} left, less than ${BALANCE_DAYS} days at ${usd(average)} a day`,
          action: 'Fix',
          href: settingsHref('captcha'),
        });
      }
    }

    const storage = data.storage.data;
    if (storage) {
      for (const profile of data.profiles.data ?? []) {
        const quota = profile.disk_quota_bytes;
        const used = storage.profiles.find((u) => u.profile_id === profile.id)?.bytes ?? 0;
        if (quota && used > quota) {
          out.push({
            key: `quota:${profile.id}`,
            tone: 'warning',
            title: `${profile.name} is over its disk quota`,
            detail: `${bytes(used)} used of ${bytes(quota)}`,
            action: 'Open',
            href: `/profiles/${profile.id}`,
          });
        }
      }
      if (storage.floor_bytes > 0 && storage.free_bytes < storage.floor_bytes) {
        out.push({
          key: 'disk-floor',
          tone: 'danger',
          title: 'Free disk space is below the floor',
          detail: `${bytes(storage.free_bytes)} free, brr keeps ${bytes(storage.floor_bytes)} free`,
          action: 'Fix',
          href: settingsHref('storage'),
        });
      }
    }

    for (const failure of data.failedBackups.data ?? []) {
      out.push({
        key: `backup:${failure.profileId}`,
        tone: 'danger',
        title: `The last backup of ${profileName(failure.profileId)} failed`,
        detail: failure.error,
        action: 'Open',
        href: `/profiles/${failure.profileId}#backups`,
      });
    }

    const referenced = new Set<string>();
    const referencedPools = new Set<string>();
    for (const settings of [...(data.templates.data ?? []), ...(data.profiles.data ?? [])].map((x) => x.settings)) {
      if (!settings.proxy) continue;
      if ('proxy_id' in settings.proxy) referenced.add(settings.proxy.proxy_id);
      else referencedPools.add(settings.proxy.pool_id);
    }
    for (const p of attention.unhealthyProxies) {
      if (!referenced.has(p.id)) continue;
      out.push({
        key: `proxy:${p.id}:${p.checked_at}`,
        tone: 'warning',
        title: `Proxy ${p.scheme}://${p.host}:${p.port} is unhealthy`,
        detail: p.last_error ?? 'Its last health check failed',
        action: 'Fix',
        href: settingsHref('proxies'),
      });
    }
    for (const pool of data.pools.data ?? []) {
      if (!referencedPools.has(pool.id)) continue;
      const bad = attention.unhealthyProxies.filter((p) => pool.proxy_ids.includes(p.id));
      if (!bad.length) continue;
      out.push({
        key: `pool:${pool.id}:${bad.map((p) => `${p.id}:${p.checked_at}`).join(',')}`,
        tone: 'warning',
        title: `Proxy pool ${pool.name} has unhealthy proxies`,
        detail: `${bad.map((p) => `${p.host}:${p.port}`).join(', ')} · ${pool.proxy_ids.length - bad.length} of ${pool.proxy_ids.length} not flagged`,
        action: 'Fix',
        href: settingsHref('proxies'),
      });
    }

    for (const b of data.builds.data ?? []) {
      if (b.state !== 'failed') continue;
      out.push({
        key: `build:${b.id}`,
        tone: 'danger',
        title: `Installing ${b.name} ${b.version} failed`,
        detail: 'Try the install again',
        action: 'Retry',
        href: '/versions',
      });
    }

    return out;
  });

  const loading = $derived(attention.loading || !data.profiles.data);
</script>

{#if !loading && rows.length}
  <Card title="Needs you">
    <ul class="rows">
      {#each rows as row, index (row.key)}
        <li>
          <div class="who">
            <p class="title">{row.title}</p>
            <p class="meta"><Badge tone={row.tone}>{row.tone === 'danger' ? 'Failed' : 'Needs you'}</Badge><span class="num">{row.detail}</span></p>
          </div>
          <div class="row-actions">
            {#if row.dismissible}<Button variant="ghost" size="sm" aria-label="Dismiss: {row.title}" onclick={() => attention.dismiss(row.key)}>Dismiss</Button>{/if}
            <Button variant={index === 0 && row.tone === 'warning' ? 'primary' : 'secondary'} size="sm" href={row.href}>{row.action}</Button>
          </div>
        </li>
      {/each}
    </ul>
  </Card>
{/if}

<style>
  .rows {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) 0;
  }

  .row-actions {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }

  .title {
    font-weight: 500;
  }

  .meta {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-1);
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }
</style>
