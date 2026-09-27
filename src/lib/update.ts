import { relaunch } from '@tauri-apps/plugin-process'
import { check, type Update } from '@tauri-apps/plugin-updater'

export type { Update }

/**
 * Asks GitHub whether a newer published release exists. Only looks: nothing is downloaded until
 * the user clicks [update]. Never throws; offline, no release yet, or any other error means "no".
 */
export async function findUpdate(): Promise<Update | null> {
  if (import.meta.env.DEV) return null
  try {
    return await check()
  } catch {
    return null
  }
}

/** Downloads and installs `update`, reporting progress (0 to 1, or null if unknown), then restarts. */
export async function installUpdate(update: Update, onProgress: (fraction: number | null) => void) {
  let total = 0
  let received = 0
  await update.downloadAndInstall((event) => {
    if (event.event === 'Started') total = event.data.contentLength ?? 0
    else if (event.event === 'Progress') {
      received += event.data.chunkLength
      onProgress(total ? Math.min(1, received / total) : null)
    }
  })
  // On Windows the installer closes Trimmy itself; this restarts it everywhere else.
  await relaunch()
}
