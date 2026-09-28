<script lang="ts">
  import { getVersion } from '@tauri-apps/api/app'
  import { invoke } from '@tauri-apps/api/core'
  import { listen } from '@tauri-apps/api/event'
  import { getCurrentWebview } from '@tauri-apps/api/webview'
  import { getCurrentWindow } from '@tauri-apps/api/window'
  import { open as pickFile } from '@tauri-apps/plugin-dialog'
  import { onMount } from 'svelte'
  import Editor from './lib/Editor.svelte'
  import OpenScreen from './lib/OpenScreen.svelte'
  import { checkForUpdates } from './lib/updates.svelte'
  import { launchPath, openVideo, type VideoInfo } from './lib/video'

  const VIDEO_EXTENSIONS = ['mp4', 'mkv', 'mov', 'webm', 'avi', 'm4v', 'ts', 'mts', 'm2ts', 'wmv', 'flv', '3gp', 'mpg', 'mpeg']

  let video = $state<VideoInfo | null>(null)
  let error = $state('')
  let dragging = $state(false)
  let version = $state('')

  // The window starts hidden; show it once the first frame is on screen, so there's no empty flash.
  onMount(() => requestAnimationFrame(() => requestAnimationFrame(() => invoke('ui_ready'))))

  getVersion().then((v) => (version = v))
  // Only looks for an update; installing waits for the user to click [update].
  checkForUpdates()

  async function open(path: string) {
    error = ''
    try {
      video = await openVideo(path)
    } catch (e) {
      error = String(e)
    }
  }

  async function browse() {
    const path = await pickFile({
      filters: [
        { name: 'Videos', extensions: VIDEO_EXTENSIONS },
        { name: 'All files', extensions: ['*'] },
      ],
    })
    if (path) await open(path)
  }

  $effect(() => {
    getCurrentWindow().setTitle(video ? `${video.name} - Trimmy` : 'Trimmy')
  })

  function close() {
    video = null
    error = ''
  }

  launchPath().then((path) => {
    if (path) open(path)
  })

  $effect(() => {
    const unlisten = getCurrentWebview().onDragDropEvent(({ payload }) => {
      dragging = payload.type === 'enter' || payload.type === 'over'
      if (payload.type === 'drop' && payload.paths.length > 0) open(payload.paths[0])
    })
    return () => void unlisten.then((stop) => stop())
  })

  // macOS sends Finder's "Open With" files to the running app as an event.
  $effect(() => {
    const unlisten = listen<string>('open-file', (event) => open(event.payload))
    return () => void unlisten.then((stop) => stop())
  })

  function onkeydown(event: KeyboardEvent) {
    // Ctrl+O on Windows, ⌘O on macOS.
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'o') {
      event.preventDefault()
      browse()
    }
  }
</script>

<svelte:window {onkeydown} />

<main>
  {#if video}
    <!-- A new file gets a fresh editor: handles, playhead and export state all reset. -->
    {#key video.path}
      <Editor {video} onclose={close} />
    {/key}
  {:else}
    <OpenScreen onopen={open} onbrowse={browse} {version} />
  {/if}
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
</main>

{#if dragging}
  <div class="overlay">&gt; drop to open</div>
{/if}

<style>
  main {
    box-sizing: border-box;
    height: 100%;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 24px;
  }

  .error {
    margin: 0;
    color: var(--error);
  }

  .overlay {
    position: fixed;
    inset: 12px;
    display: grid;
    place-content: center;
    border: 1px dashed var(--accent);
    background: rgb(10 13 14 / 0.92);
    font-size: 20px;
    font-weight: 800;
    color: var(--accent);
    pointer-events: none;
  }
</style>
