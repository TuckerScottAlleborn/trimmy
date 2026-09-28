<script lang="ts">
  import { formatTime, keyframeAfter, keyframeAtOrBefore } from './video'

  let {
    duration,
    time,
    peaks,
    keyframes,
    step,
    start = $bindable(),
    end = $bindable(),
    onseek,
  }: {
    duration: number
    time: number
    /** Audio loudness (0 to 1) in evenly spaced slices; drawn as the waveform. */
    peaks: number[]
    /** Where the start handle can go (export starts on a keyframe); empty means anywhere. */
    keyframes: number[]
    /** How far the arrow keys move a focused handle (one frame). */
    step: number
    start: number
    end: number
    onseek: (time: number) => void
  } = $props()

  /** The shortest clip the handles allow, in seconds. */
  const MIN_LENGTH = 0.1

  let track: HTMLDivElement
  let dragging: 'start' | 'end' | 'playhead' | null = null

  const percent = (t: number) => `${(t / duration) * 100}%`

  /** Past this many keyframes the ticks would just be a solid smear, so they're left out. */
  const MAX_TICKS = 400

  function setStart(t: number) {
    start = keyframeAtOrBefore(keyframes, Math.max(0, Math.min(t, end - MIN_LENGTH)))
    onseek(start)
  }

  function setEnd(t: number) {
    end = Math.min(duration, Math.max(t, start + MIN_LENGTH))
    onseek(end)
  }

  function grab(event: PointerEvent, what: 'start' | 'end' | 'playhead') {
    event.stopPropagation()
    dragging = what
    track.setPointerCapture(event.pointerId)
    drag(event)
  }

  function drag(event: PointerEvent) {
    if (!dragging) return
    const box = track.getBoundingClientRect()
    const t = Math.max(0, Math.min(duration, ((event.clientX - box.left) / box.width) * duration))
    if (dragging === 'start') setStart(t)
    else if (dragging === 'end') setEnd(t)
    else onseek(t)
  }

  function nudge(event: KeyboardEvent, which: 'start' | 'end') {
    const by = event.key === 'ArrowLeft' ? -1 : event.key === 'ArrowRight' ? 1 : 0
    if (!by) return
    event.preventDefault()
    event.stopPropagation() // don't also step the playhead
    const amount = by * (event.shiftKey ? 1 : step)
    if (which === 'end') return setEnd(end + amount)
    if (keyframes.length === 0) return setStart(start + amount)
    // The start handle steps from keyframe to keyframe.
    if (by < 0) return setStart(start - 1e-3)
    const next = keyframeAfter(keyframes, start)
    if (next !== null && next <= end - MIN_LENGTH) setStart(next)
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="track"
  bind:this={track}
  onpointerdown={(e) => grab(e, 'playhead')}
  onpointermove={drag}
  onpointerup={() => (dragging = null)}
>
  <div class="wave">
    {#each peaks as peak, i (i)}
      <i style:height="{Math.max(4, peak * 100)}%"></i>
    {/each}
  </div>
  {#if keyframes.length > 0 && keyframes.length <= MAX_TICKS}
    <svg class="ticks" viewBox="0 0 {duration} 1" preserveAspectRatio="none" aria-hidden="true">
      <path d={keyframes.map((k) => `M${k} 0V1`).join('')} />
    </svg>
  {/if}
  <div class="outside" style:left="0" style:width={percent(start)}></div>
  <div class="outside" style:left={percent(end)} style:right="0"></div>
  <div class="clip" style:left={percent(start)} style:width={percent(end - start)}></div>
  <div class="playhead" style:left={percent(time)}></div>
  {#each ['start', 'end'] as const as which (which)}
    <div
      class="handle {which}"
      style:left={percent(which === 'start' ? start : end)}
      role="slider"
      tabindex="0"
      aria-label={which === 'start' ? 'Clip start' : 'Clip end'}
      aria-valuemin={0}
      aria-valuemax={duration}
      aria-valuenow={which === 'start' ? start : end}
      aria-valuetext={formatTime(which === 'start' ? start : end)}
      title={which === 'start' && keyframes.length > 0 ? 'Snaps to keyframes, where a lossless clip can start' : undefined}
      onpointerdown={(e) => grab(e, which)}
      onkeydown={(e) => nudge(e, which)}
    ></div>
  {/each}
</div>

<style>
  .track {
    position: relative;
    height: 56px;
    margin: 0 8px;
    border: 1px solid var(--line);
    cursor: pointer;
    touch-action: none;
  }

  /* Bars mirrored around the middle, like an audio editor. */
  .wave {
    position: absolute;
    inset: 4px 0;
    display: flex;
    align-items: center;
    gap: 1px;
  }

  .wave i {
    flex: 1;
    background: var(--accent);
    opacity: 0.75;
  }

  /* Keyframes: where the start handle can land. Faint, along the bottom edge. */
  .ticks {
    position: absolute;
    left: 0;
    bottom: 0;
    width: 100%;
    height: 6px;
    pointer-events: none;
  }

  .ticks path {
    stroke: var(--muted);
    stroke-width: 1px;
    vector-effect: non-scaling-stroke;
    opacity: 0.6;
  }

  .outside {
    position: absolute;
    top: 0;
    bottom: 0;
    background: rgb(10 13 14 / 0.75);
  }

  .clip {
    position: absolute;
    top: 0;
    bottom: 0;
    box-sizing: border-box;
    border-top: 1px solid var(--accent);
    border-bottom: 1px solid var(--accent);
    background: var(--accent-soft);
  }

  .playhead {
    position: absolute;
    top: -4px;
    bottom: -4px;
    width: 2px;
    margin-left: -1px;
    background: var(--fg);
    pointer-events: none;
  }

  .handle {
    position: absolute;
    top: -6px;
    bottom: -6px;
    width: 3px;
    margin-left: -1px;
    background: var(--accent);
    box-shadow: 0 0 8px rgb(83 252 24 / 0.6);
    cursor: ew-resize;
  }

  /* A thin line to look at, but a wide target to grab. */
  .handle::before {
    content: '';
    position: absolute;
    inset: 0 -7px;
  }
</style>
