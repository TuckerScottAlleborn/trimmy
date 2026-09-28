<script lang="ts">
  import { convertFileSrc } from '@tauri-apps/api/core'
  import { listen } from '@tauri-apps/api/event'
  import { revealItemInDir } from '@tauri-apps/plugin-opener'
  import { onDestroy } from 'svelte'
  import Timeline from './Timeline.svelte'
  import UpdateStatus from './UpdateStatus.svelte'
  import {
    closeVideo,
    describe,
    exportClip,
    formatTime,
    keyframeAtOrBefore,
    loadKeyframes,
    loadWaveform,
    WAVEFORM_BARS,
    type VideoInfo,
  } from './video'

  let { video, onclose }: { video: VideoInfo; onclose: () => void } = $props()

  const frame = $derived(1 / (video.fps || 30))

  let player: HTMLVideoElement
  let time = $state(0)
  let paused = $state(true)
  let start = $state(0)
  // App re-creates the editor for each file, so the whole video is the initial clip.
  // svelte-ignore state_referenced_locally
  let end = $state(video.duration)
  let previewFailed = $state(false)
  /** Export progress from 0 to 1 while exporting, otherwise null. */
  let progress = $state<number | null>(null)
  let saved = $state('')
  let exportError = $state('')
  /**
   * Audio peaks for the timeline (0 to 1). While FFmpeg is still reading, slots it hasn't reached
   * yet are -1 (drawn empty). Empty if there's no audio.
   */
  let peaks = $state<number[]>([])
  /** Raw peaks received so far, before scaling to the loudest. */
  const partial: number[] = []
  /** Where a lossless clip can start; empty until ffprobe has listed them (or if any start works). */
  let keyframes = $state<number[]>([])

  // Draw the waveform in as it's computed, scaled to the loudest part heard so far, so a long
  // recording shows its start right away instead of an empty timeline.
  function addPeaks(batch: number[]) {
    partial.push(...batch)
    const loudest = Math.max(...partial) || 1
    peaks = Array.from({ length: Math.max(WAVEFORM_BARS, partial.length) }, (_, i) =>
      i < partial.length ? partial[i] / loudest : -1,
    )
  }

  // svelte-ignore state_referenced_locally
  const waveformDone = loadWaveform(video, addPeaks).then(
    (p) => (peaks = p),
    () => (peaks = []),
  )
  // Closing the file, or opening another, stops FFmpeg reading this one. On a big file on a slow
  // drive that can otherwise go on for a long time after it's gone from the screen.
  onDestroy(() => void closeVideo(video.path))

  // After the waveform, not alongside it: when the file has no index to read, the keyframe scan
  // reads the whole file too, and two readers at once make a hard drive thrash.
  // svelte-ignore state_referenced_locally
  waveformDone.then(() => loadKeyframes(video)).then(
    (k) => {
      keyframes = k
      start = keyframeAtOrBefore(k, start)
    },
    () => {},
  )

  function seek(t: number) {
    time = Math.max(0, Math.min(video.duration, t))
    scrubTo = null
    player.currentTime = time
  }

  /** Where a timeline drag wants the video to be; applied at most once per frame. */
  let scrubTo: number | null = null

  // Dragging fires far more often than a video can seek, and piling up seeks makes long files
  // stutter. The playhead follows the mouse right away; the video catches up with the latest
  // position whenever its previous seek has finished.
  function scrub(t: number) {
    time = Math.max(0, Math.min(video.duration, t))
    if (scrubTo === null) requestAnimationFrame(applyScrub)
    scrubTo = time
  }

  function applyScrub() {
    if (scrubTo === null) return
    if (player.seeking) return void requestAnimationFrame(applyScrub)
    player.currentTime = scrubTo
    scrubTo = null
  }

  function togglePlay() {
    if (!player.paused) return player.pause()
    // Play the clip: from its start, unless the playhead is already inside it.
    if (time < start || time >= end - 0.05) seek(start)
    player.play()
  }

  // While playing, move the playhead every frame and stop at the end handle.
  $effect(() => {
    if (paused) return
    let request = requestAnimationFrame(function tick() {
      time = player.currentTime
      if (time >= end) {
        player.pause()
        seek(end)
      } else {
        request = requestAnimationFrame(tick)
      }
    })
    return () => cancelAnimationFrame(request)
  })

  async function save() {
    player.pause()
    saved = ''
    exportError = ''
    progress = 0
    const stop = await listen<number>('export-progress', (event) => (progress = event.payload))
    try {
      saved = await exportClip(video.path, start, end)
    } catch (e) {
      exportError = String(e)
    } finally {
      stop()
      progress = null
    }
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.target instanceof HTMLInputElement || event.ctrlKey || event.metaKey || event.altKey) return
    const key = event.key.toLowerCase()
    if (key === ' ') togglePlay()
    else if (key === 'i') start = keyframeAtOrBefore(keyframes, Math.min(time, end - 0.1))
    else if (key === 'o') end = Math.max(time, start + 0.1)
    else if (key === 'x') {
      // Clear: back to the whole video.
      start = 0
      end = video.duration
    }
    else if (key === 'arrowleft' || key === 'arrowright') {
      player.pause()
      seek(time + (key === 'arrowleft' ? -1 : 1) * (event.shiftKey ? 1 : frame))
    } else return
    event.preventDefault()
  }
</script>

<svelte:window {onkeydown} />

<header>
  <div>
    <h2 title={video.path}>{video.name}</h2>
    <p># {describe(video)}</p>
  </div>
  <div class="actions">
    <UpdateStatus compact />
    <button onclick={onclose} aria-label="Close video">close</button>
  </div>
</header>

<!-- svelte-ignore a11y_media_has_caption -->
<video
  bind:this={player}
  bind:paused
  src={convertFileSrc(video.path)}
  onclick={togglePlay}
  onerror={() => (previewFailed = true)}
></video>
{#if previewFailed}
  <p class="error">! can't preview this file yet, but you can still trim and export it</p>
{/if}

<Timeline duration={video.duration} {time} {peaks} {keyframes} step={frame} bind:start bind:end onseek={scrub} />

<div class="controls">
  <button class="play" onclick={togglePlay}>{paused ? 'play' : 'stop'}</button>
  <span class="time">{formatTime(time)}</span>
  <span class="clip">
    clip {formatTime(start)} – {formatTime(end)} <strong>({formatTime(end - start)})</strong>
  </span>
  <span class="keys">space play · i/o set start/end · x clear · ←→ frame</span>
  <button
    class="primary"
    onclick={save}
    disabled={progress !== null}
    title="Lossless and fast: nothing is re-encoded. That's why the start snaps to keyframes."
  >
    {progress === null ? 'export' : `exporting ${Math.round(progress * 100)}%`}
  </button>
</div>

{#if saved}
  <p class="saved">
    <span>&gt; saved {saved.split(/[\\/]/).pop()}</span>
    <button onclick={() => revealItemInDir(saved)}>show in folder</button>
  </p>
{/if}
{#if exportError}
  <p class="error" role="alert">{exportError}</p>
{/if}

<style>
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }

  h2 {
    margin: 0 0 4px;
    font-size: 14px;
    font-weight: 700;
    color: var(--accent);
    overflow-wrap: anywhere;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 16px;
  }

  header p {
    margin: 0;
    font-size: 12px;
    color: var(--muted);
  }

  video {
    flex: 1;
    min-height: 0;
    width: 100%;
    box-sizing: border-box;
    border: 1px solid rgb(83 252 24 / 0.12); /* barely there */
    background: #000;
    cursor: pointer;
  }

  .controls {
    display: flex;
    align-items: center;
    gap: 16px;
  }

  .play {
    min-width: 6ch;
  }

  .time {
    min-width: 7ch;
    color: var(--accent);
    font-weight: 700;
  }

  .clip {
    color: var(--muted);
  }

  .clip strong {
    color: var(--fg);
    font-weight: 700;
  }

  .keys {
    flex: 1;
    text-align: right;
    font-size: 11px;
    color: var(--muted);
  }

  .saved {
    display: flex;
    align-items: center;
    gap: 12px;
    margin: 0;
  }

  .error {
    margin: 0;
    color: var(--error);
  }
</style>
