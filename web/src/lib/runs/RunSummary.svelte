<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import type { components } from '$lib/api/schema';
  import { buildLabel } from '$lib/builds/labels';
  import { clock } from '$lib/clock.svelte';
  import { plainError } from '$lib/events/summary';
  import { absolute, duration, trafficSummary } from '$lib/format';
  import KeyValue from '$lib/ui/KeyValue.svelte';
  import { atLeastZero, isFinal, runResult, templateLabel } from './runs';

  type Instance = components['schemas']['Instance'];

  let { run }: { run: Instance } = $props();

  // A deleted Profile keeps its name on the run but has no page to link to.
  const profile = liveQuery(
    async () => {
      if (!run.profile_id) return null;
      try {
        return await unwrap(api.GET('/v1/profiles/{profile}', { params: { path: { profile: run.profile_id } } }));
      } catch {
        return null;
      }
    },
    { kinds: ['profile.*'] },
  );
  const template = liveQuery(
    async () => {
      if (!run.template_id) return null;
      try {
        return await unwrap(api.GET('/v1/templates/{template}', { params: { path: { template: run.template_id } } }));
      } catch {
        return null;
      }
    },
    { kinds: ['template.*'] },
  );
  const leases = liveQuery(
    async () => (await unwrap(api.GET('/v1/leases', { params: { query: { instance_id: run.id, include_released: true } } }))).leases,
    { kinds: ['lease.*'], match: (e) => (e.data as { instance_id?: string } | null)?.instance_id === run.id },
  );

  const hosts = liveQuery(async () => (await unwrap(api.GET('/v1/hosts'))).hosts, { kinds: ['instance.state_changed'] });
  const hostName = $derived(run.host_id ? (hosts.data?.find((h) => h.id === run.host_id)?.name ?? 'Unknown host') : run.kind === 'managed' ? 'This machine' : null);

  const clients = $derived([...new Set((leases.data ?? []).map((l) => l.client_label))].join(', ') || null);
  const length = $derived.by(() => {
    if (!run.started_at) return null;
    return duration(atLeastZero((run.ended_at ?? clock.now) - run.started_at));
  });

  const items = $derived(
    [
      { label: 'Started', value: run.started_at ? absolute(run.started_at) : null },
      { label: 'Ended', value: run.ended_at ? absolute(run.ended_at) : null },
      { label: 'Duration', value: length },
      { label: 'Traffic', value: run.kind === 'managed' && run.started_at ? trafficSummary(run.traffic) : null },
      { label: 'End reason', value: isFinal(run) ? runResult(run) : null },
      { label: 'Error', value: run.error ? plainError(run.error) : null, tone: 'danger' as const },
      { label: 'Browser version', value: run.build ? buildLabel(run.build) : null },
      { label: 'Host', value: hostName },
      { label: 'Display', value: run.display },
      { label: 'Sandbox', value: run.sandbox ? (run.sandbox === 'docker' ? 'Docker' : 'None') : null },
      run.kind === 'live'
        ? { label: 'Profile', value: 'Attached browser' }
        : {
            label: 'Profile',
            value: run.profile_name ?? null,
            href: profile.data && run.profile_id ? `/profiles/${run.profile_id}` : undefined,
          },
      {
        label: 'Template',
        value: run.template_name ? templateLabel(run.template_name) : null,
        href: template.data ? `/templates/${template.data.id}` : undefined,
      },
      { label: 'Clients', value: clients },
      { label: 'Restarts', value: run.restart_count > 0 ? run.restart_count : null },
      { label: 'Run ID', value: run.id, mono: true, copy: true },
      { label: 'Process ID', value: run.pid && !isFinal(run) ? run.pid : null },
      { label: 'Browser log', value: run.log_path ?? null, mono: true, copy: true },
    ].filter((item) => item.value !== null),
  );
</script>

<KeyValue {items} grid />
