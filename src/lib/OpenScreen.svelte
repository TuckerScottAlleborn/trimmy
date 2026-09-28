<script lang="ts">
  import UpdateStatus from './UpdateStatus.svelte'
  import { isMac } from './video'

  let {
    onopen,
    onbrowse,
    version,
  }: {
    onopen: (path: string) => void
    onbrowse: () => void
    /** This build's version, shown faintly in the corner. */
    version: string
  } = $props()

  let typed = $state('')

  function submit(event: SubmitEvent) {
    event.preventDefault()
    // Explorer's "Copy as path" wraps the path in double quotes; macOS Terminal uses single quotes.
    const path = typed.trim().replace(/^(["'])(.*)\1$/, '$2')
    if (path) onopen(path)
  }
</script>

<section>
  <h1>trimmy</h1>
  <p># drop a video here, paste its path, or browse for it</p>
  <form onsubmit={submit}>
    <input
      bind:value={typed}
      placeholder={isMac ? '/Users/you/Movies/clip.mp4' : 'C:\\path\\to\\clip.mp4'}
      spellcheck="false"
      aria-label="Video file path"
    />
    <button class="primary" disabled={!typed.trim()}>open</button>
  </form>
  <div class="browse">
    <button type="button" onclick={onbrowse}>browse</button>
    <kbd>{isMac ? '⌘o' : 'ctrl+o'}</kbd>
  </div>

  <footer>
    <UpdateStatus />
    {#if version}<span class="version">v{version}</span>{/if}
  </footer>
</section>

<style>
  section {
    position: relative;
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 16px;
    border: 1px dashed var(--line);
    text-align: center;
  }

  h1 {
    margin: 0;
    font-size: 40px;
    font-weight: 800;
    color: var(--accent);
    text-shadow: 0 0 18px rgb(83 252 24 / 0.175);
  }

  p {
    margin: 0 0 8px;
    color: var(--muted);
  }

  form {
    display: flex;
    gap: 8px;
    width: min(480px, 90%);
  }

  input {
    flex: 1;
    min-width: 0;
  }

  .browse {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  kbd {
    font: inherit;
    color: var(--muted);
  }

  footer {
    position: absolute;
    left: 16px;
    right: 16px;
    bottom: 10px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 12px;
  }

  /* Barely there: for bug reports, not for reading. */
  .version {
    color: var(--muted);
    opacity: 0.4;
  }
</style>
