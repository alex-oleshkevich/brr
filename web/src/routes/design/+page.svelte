<script lang="ts">
  import { ApiError } from '$lib/api/client';
  import { knownStates, type StateKind, type Tone } from '$lib/state';
  import Badge from '$lib/ui/Badge.svelte';
  import Button from '$lib/ui/Button.svelte';
  import Card from '$lib/ui/Card.svelte';
  import Checkbox from '$lib/ui/Checkbox.svelte';
  import CodeBlock from '$lib/ui/CodeBlock.svelte';
  import { confirm } from '$lib/ui/confirm.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Input from '$lib/ui/Input.svelte';
  import KeyValue from '$lib/ui/KeyValue.svelte';
  import Menu from '$lib/ui/Menu.svelte';
  import PageHeader from '$lib/ui/PageHeader.svelte';
  import Progress from '$lib/ui/Progress.svelte';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';
  import Select from '$lib/ui/Select.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import StateBadge from '$lib/ui/StateBadge.svelte';
  import StatusDot from '$lib/ui/StatusDot.svelte';
  import Switch from '$lib/ui/Switch.svelte';
  import Table from '$lib/ui/Table.svelte';
  import Tabs from '$lib/ui/Tabs.svelte';
  import Textarea from '$lib/ui/Textarea.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  const tones: Tone[] = ['neutral', 'accent', 'success', 'warning', 'danger'];
  const kinds: StateKind[] = ['instance', 'tab', 'build', 'login', 'proxy'];
  const variants = ['primary', 'secondary', 'ghost', 'danger'] as const;

  let dialogOpen = $state(false);
  let name = $state('work');
  let notes = $state('');
  let engine = $state('chromium');
  let autostart = $state(true);
  let headless = $state(false);
  let density = $state('comfortable');
  let tableTab = $state('rows');
  let confirmResult = $state('');
  let menuPick = $state('chromium');

  type Row = { id: string; name: string; state: string; memory: number };
  const data: Row[] = [
    { id: '01J9Z3', name: 'work-profile', state: 'running', memory: 612 },
    { id: '01J9Z4', name: 'scraper-eu-west-with-a-very-long-profile-name', state: 'idle', memory: 188 },
    { id: '01J9Z5', name: 'checkout-tests', state: 'crashed', memory: 0 },
  ];
  const columns = [
    { key: 'name', label: 'Name' },
    { key: 'state', label: 'State', width: '140px' },
    { key: 'memory', label: 'Memory', width: '120px', align: 'right' as const },
    { key: 'actions', label: 'Actions', width: '48px', srOnly: true },
  ];
  const failure = new ApiError(500, 'Internal', 'The daemon returned an unexpected response.', {});

  const tableRows = $derived(tableTab === 'rows' ? data : tableTab === 'empty' ? [] : undefined);
  const tabItems = [
    { id: 'rows', label: 'Rows', count: data.length },
    { id: 'loading', label: 'Loading' },
    { id: 'empty', label: 'Empty' },
    { id: 'error', label: 'Error' },
  ];

  const menuItems = $derived([
    { label: 'Chromium', checked: menuPick === 'chromium', onselect: () => (menuPick = 'chromium') },
    { label: 'Camoufox', checked: menuPick === 'camoufox', onselect: () => (menuPick = 'camoufox') },
    'separator' as const,
    { label: 'Duplicate', onselect: () => toast.success('Duplicated') },
    { label: 'Delete', disabled: true, hint: 'The profile is in use', onselect: () => {} },
    { label: 'Remove', tone: 'danger' as const, onselect: () => toast.success('Removed') },
  ]);

  const rowMenu = [
    { label: 'Open', onselect: () => {} },
    { label: 'Delete', tone: 'danger' as const, onselect: () => {} },
  ];

  const json = JSON.stringify(
    {
      id: '01J9Z3K2M4N5P6Q7R8S9T0V1W2',
      engine: 'chromium',
      args: ['--headless=new', '--disable-gpu', '--window-size=1280,800'],
      limits: { memory_mb: 1024, cpu_shares: 512, tabs: 8 },
      labels: { owner: 'ci', purpose: 'checkout regression run against the staging environment' },
      history: Array.from({ length: 12 }, (_, i) => ({ seq: 1000 + i, kind: 'instance.state_changed', to: 'running' })),
    },
    null,
    2,
  );

  const options = [
    { value: 'chromium', label: 'Chromium' },
    { group: 'Other engines', options: [{ value: 'camoufox', label: 'Camoufox' }, { value: 'edge', label: 'Edge', disabled: true }] },
  ];

  async function askConfirm(tone: 'primary' | 'danger') {
    const ok = await confirm(
      tone === 'danger'
        ? { title: 'Delete profile work?', description: 'Its cookies and storage are removed. This cannot be undone.', confirmLabel: 'Delete profile', tone }
        : { title: 'Start 3 instances?', description: 'They use about 1.8 GB of memory.', confirmLabel: 'Start', tone },
    );
    confirmResult = ok ? 'Confirmed' : 'Cancelled';
  }
</script>

<PageHeader
  title="Design system"
  description="Every component in every state. Visible in development only."
  breadcrumb={[{ label: 'System', href: '/design' }]}
>
  {#snippet actions()}
    <Button icon="refresh">Refresh</Button>
    <Button variant="primary" icon="plus">New profile</Button>
  {/snippet}
</PageHeader>

<div class="stack">
  <Card title="Buttons" description="Primary is used once per view. Danger only inside confirm dialogs.">
    <div class="stack-sm">
      {#each ['md', 'sm'] as const as size}
        {#each variants as variant}
          <div class="row">
            <Button {variant} {size} icon="plus">Normal</Button>
            <Button {variant} {size} disabled>Disabled</Button>
            <Button {variant} {size} loading>Loading</Button>
            <Button {variant} {size} iconOnly icon="more" aria-label="More {variant} {size}" />
            <Button {variant} {size} href="/design">Link</Button>
          </div>
        {/each}
      {/each}
    </div>
  </Card>

  <Card title="Fields">
    <div class="grid">
      <Input label="Name" bind:value={name} />
      <Input label="Proxy url" placeholder="http://127.0.0.1:8080" hint="Leave empty to connect directly." />
      <Input label="Port" type="number" value="9898" error="Port 9898 is already in use." />
      <Input label="Region" value="eu-west" disabled />
      <Input label="Password" type="password" value="hunter2" />
      <Select label="Engine" bind:value={engine} {options} hint="Chosen when the instance starts." />
      <Textarea label="Notes" bind:value={notes} placeholder="Anything worth remembering" />
      <div class="stack-sm">
        <Checkbox label="Restore tabs" bind:checked={autostart} hint="Reopen the last session." />
        <Checkbox label="Disabled option" disabled />
        <Switch label="Autostart" bind:checked={autostart} hint="Start with the daemon." />
        <Switch label="Headless" bind:checked={headless} />
        <Switch label="Locked" checked disabled />
      </div>
      <SegmentedControl
        label="Density"
        bind:value={density}
        options={[
          { value: 'comfortable', label: 'Comfortable', hint: 'Roomy rows for everyday use.' },
          { value: 'compact', label: 'Compact', hint: 'More rows per screen.' },
          { value: 'dense', label: 'Dense' },
        ]}
      />
    </div>
  </Card>

  <section class="stack-sm" aria-labelledby="table-title">
    <h2 id="table-title">Table</h2>
    <Tabs items={tabItems} bind:value={tableTab} label="Table state" idPrefix="table" />
    <div role="tabpanel" id="table-panel-{tableTab}" aria-labelledby="table-tab-{tableTab}">
      <Table
        caption="Profiles"
        {columns}
        rows={tableRows}
        key={(row: Row) => row.id}
        loading={tableTab === 'loading'}
        error={tableTab === 'error' ? failure : null}
        onretry={() => (tableTab = 'rows')}
        empty={{ title: 'No profiles yet', description: 'Profiles keep cookies and storage between runs.' }}
      >
        {#snippet row(item: Row)}
          <td class="truncate" title={item.name}><a href="/design">{item.name}</a></td>
          <td><StateBadge kind="instance" state={item.state} /></td>
          <td class="num right">{item.memory} MB</td>
          <td><Menu label="Actions for {item.name}" items={rowMenu} /></td>
        {/snippet}
      </Table>
    </div>
  </section>

  <Card title="Badges">
    <div class="stack-sm">
      <div class="row">
        {#each tones as tone}<Badge {tone}>{tone}</Badge>{/each}
        {#each tones as tone}<Badge {tone} dot>{tone}</Badge>{/each}
      </div>
      {#each kinds as kind}
        <div class="row">
          <span class="kind">{kind}</span>
          {#each knownStates(kind) as state}<StateBadge {kind} {state} />{/each}
        </div>
      {/each}
    </div>
  </Card>

  <Card title="Overlays" description="Dialog, confirmation, toasts and menu.">
    <div class="row">
      <Button onclick={() => (dialogOpen = true)}>Open dialog</Button>
      <Button onclick={() => askConfirm('primary')}>Confirm</Button>
      <Button onclick={() => askConfirm('danger')}>Confirm danger</Button>
      <Button onclick={() => toast.show({ tone: 'neutral', title: 'Profile saved', description: 'Changes apply on the next start.' })}>Neutral toast</Button>
      <Button onclick={() => toast.success('Copied')}>Success toast</Button>
      <Button onclick={() => toast.show({ tone: 'warning', title: 'Memory is low', description: 'New instances will queue.' })}>Warning toast</Button>
      <Button onclick={() => toast.error('Could not start', 'Port 9898 is already in use.')}>Error toast</Button>
      <Button onclick={() => toast.show({ tone: 'neutral', title: 'Instance stopped', action: { label: 'Undo', onclick: () => toast.success('Restored') } })}>Toast with action</Button>
      <Menu label="Engine" items={menuItems} align="start" />
      {#if confirmResult}<span class="result">{confirmResult}</span>{/if}
    </div>
  </Card>

  <div class="grid">
    <Card title="Empty states">
      <EmptyState title="No instances" description="Start one from a profile to see it here." icon="plus" />
    </Card>
    <Card title="Danger">
      <EmptyState tone="danger" title="Couldn't load instances" description="The daemon returned an unexpected response.">
        {#snippet action()}<Button>Retry</Button>{/snippet}
      </EmptyState>
    </Card>
  </div>

  <div class="grid">
    <Card title="Skeleton and status">
      <div class="stack-sm">
        <Skeleton width="60%" />
        <Skeleton width="80%" />
        <Skeleton height="64px" radius="var(--radius-lg)" />
        <div class="row">
          {#each tones as tone}<StatusDot {tone} label={tone} pulse={tone === 'success'} />{/each}
        </div>
      </div>
    </Card>
    <Card title="Progress">
      <div class="stack-sm">
        {#each ['accent', 'success', 'warning', 'danger'] as const as tone}
          <Progress value={40} label="{tone} progress" {tone} />
        {/each}
        <Progress value={null} label="Indeterminate progress" size="md" />
      </div>
    </Card>
  </div>

  <div class="grid">
    <Card title="Key value">
      <KeyValue
        items={[
          { label: 'Name', value: 'work-profile' },
          { label: 'Id', value: '01J9Z3K2M4N5P6Q7R8S9T0V1W2', mono: true, copy: true },
          { label: 'Engine', value: 'Chromium 131', href: '/design' },
          { label: 'Proxy', value: null },
        ]}
      />
    </Card>
    <Card title="Code block">
      <CodeBlock code={json} label="Profile JSON" />
    </Card>
  </div>
</div>

<Dialog bind:open={dialogOpen} title="Rename profile" description="The name is shown across the dashboard.">
  <Input label="Name" bind:value={name} />
  {#snippet footer()}
    <Button onclick={() => (dialogOpen = false)}>Cancel</Button>
    <Button variant="primary" onclick={() => (dialogOpen = false)}>Save</Button>
  {/snippet}
</Dialog>

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-6);
  }

  .stack-sm {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: var(--space-4);
  }

  h2 {
    font-size: var(--text-md-size);
    line-height: var(--text-md-line);
  }

  .kind {
    width: 72px;
    color: var(--text-secondary);
  }

  .result {
    color: var(--text-secondary);
  }
</style>
