import { relaunch } from '@tauri-apps/plugin-process'
import { check, type Update } from '@tauri-apps/plugin-updater'

/**
 * Where the one update check stands. Shared by the open screen and the editor so they always agree.
 * Trimmy only looks for an update on launch; nothing is downloaded until the user clicks [update].
 */
export const updates = $state<{
  status: 'checking' | 'current' | 'offline' | 'available' | 'downloading' | 'restarting' | 'failed'
  update: Update | null
  /** Download progress from 0 to 1 (null when the size is unknown). */
  progress: number | null
  error: string
}>({ status: 'checking', update: null, progress: null, error: '' })

/** Keeps "checking for updates" on screen long enough to read instead of flickering past. */
const MIN_CHECK_MS = 800
const CHECK_TIMEOUT_MS = 15_000

/** Asks GitHub whether a newer published release exists. Never throws. */
export async function checkForUpdates() {
  updates.status = 'checking'
  updates.error = ''
  const shown = new Promise((done) => setTimeout(done, MIN_CHECK_MS))
  try {
    // A stalled network shouldn't leave "checking for updates..." up forever.
    const found = import.meta.env.DEV ? null : await check({ timeout: CHECK_TIMEOUT_MS })
    await shown
    updates.update = found
    updates.status = found ? 'available' : 'current'
  } catch (e) {
    await shown
    // Offline, GitHub down, or a firewall: say so quietly and let the user try again.
    updates.error = String(e)
    updates.status = 'offline'
  }
}

/** Downloads and installs the update the check found, then restarts Trimmy. */
export async function installUpdate() {
  const update = updates.update
  if (!update) return
  updates.status = 'downloading'
  updates.progress = null
  let total = 0
  let received = 0
  try {
    await update.downloadAndInstall((event) => {
      if (event.event === 'Started') total = event.data.contentLength ?? 0
      else if (event.event === 'Progress') {
        received += event.data.chunkLength
        updates.progress = total ? Math.min(1, received / total) : null
      }
    })
    updates.status = 'restarting'
    // On Windows the installer closes Trimmy itself; this restarts it everywhere else.
    await relaunch()
  } catch (e) {
    updates.error = String(e)
    updates.status = 'failed'
  }
}

export type { Update }
