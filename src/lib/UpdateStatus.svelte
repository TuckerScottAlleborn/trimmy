<script lang="ts">
  import { checkForUpdates, installUpdate, updates } from './updates.svelte'

  /** `compact` is the editor's version: smaller, and it says nothing unless there's something to do. */
  let { compact = false }: { compact?: boolean } = $props()

  const percent = $derived(updates.progress === null ? '' : ` ${Math.round(updates.progress * 100)}%`)

  // The editor says "up to date" for a moment only if it watched the check happen, then goes quiet.
  let showCurrent = $state(updates.status === 'checking')
  $effect(() => {
    if (!compact || updates.status !== 'current') return
    const timer = setTimeout(() => (showCurrent = false), 2500)
    return () => clearTimeout(timer)
  })
</script>

{#if compact}
  <span class="status compact" aria-live="polite">
    {#if updates.status === 'checking'}
      checking for updates...
    {:else if updates.status === 'current' && showCurrent}
      up to date
    {:else if updates.status === 'available'}
      v{updates.update?.version} available
      <button type="button" onclick={installUpdate}>update</button>
    {:else if updates.status === 'downloading'}
      updating{percent}
    {:else if updates.status === 'restarting'}
      restarting...
    {:else if updates.status === 'failed'}
      <span title={updates.error}>update failed</span>
      <button type="button" onclick={installUpdate}>retry</button>
    {/if}
  </span>
{:else}
  <span class="status" aria-live="polite">
    {#if updates.status === 'checking'}
      # checking for updates...
    {:else if updates.status === 'current'}
      # up to date
    {:else if updates.status === 'offline'}
      <span title={updates.error}># couldn't check for updates</span>
      <button type="button" onclick={checkForUpdates}>retry</button>
    {:else if updates.status === 'available'}
      # update available: v{updates.update?.version}
      <button type="button" onclick={installUpdate}>update</button>
    {:else if updates.status === 'downloading'}
      # downloading update{percent}
    {:else if updates.status === 'restarting'}
      # restarting...
    {:else if updates.status === 'failed'}
      # update failed: {updates.error}
      <button type="button" onclick={installUpdate}>retry</button>
    {/if}
  </span>
{/if}

<style>
  .status {
    color: var(--muted);
  }

  .compact {
    font-size: 11px;
    white-space: nowrap;
  }

  .compact button {
    padding: 0 2px;
  }
</style>
