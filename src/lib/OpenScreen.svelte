<script lang="ts">
  let { onopen, onbrowse }: { onopen: (path: string) => void; onbrowse: () => void } = $props()

  let typed = $state('')

  function submit(event: SubmitEvent) {
    event.preventDefault()
    // Explorer's "Copy as path" wraps the path in quotes.
    const path = typed.trim().replace(/^"(.*)"$/, '$1')
    if (path) onopen(path)
  }
</script>

<section>
  <h1><span>$</span> trimmy</h1>
  <p># drop a video here, paste its path, or browse for it</p>
  <form onsubmit={submit}>
    <input bind:value={typed} placeholder="C:\path\to\clip.mp4" spellcheck="false" aria-label="Video file path" />
    <button class="primary" disabled={!typed.trim()}>open</button>
  </form>
  <button type="button" onclick={onbrowse}>browse <kbd>ctrl+o</kbd></button>
</section>

<style>
  section {
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
    text-shadow: 0 0 18px rgb(83 252 24 / 0.35);
  }

  h1 span {
    color: var(--muted);
    text-shadow: none;
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

  kbd {
    margin-left: 6px;
    font: inherit;
    color: var(--muted);
  }
</style>
