# Changelog

What changed in each release. The release workflow copies a version's section into its GitHub
release notes, so write it for people who use Trimmy.

## 1.0.1

Big files, and a much smaller download.

- **A 7 MB installer instead of 58 MB.** Trimmy now bundles its own build of FFmpeg with only
  the parts it uses (about 5 MB instead of 100), so opening a file is faster too.
- **Big files get a waveform.** Files over 4 GB (a two-hour Blu-ray remux, a long recording) no
  longer read every byte for it: Trimmy measures a short slice at each point of the timeline
  instead. A 38 GB movie on a USB hard drive went from about an hour to a minute or two.
- **The waveform draws in as it's read,** left to right, instead of staying empty until it's done.
- **Keyframes straight from the file's index.** For MP4, MOV and MKV files the start handle can
  snap right away, instead of after a scan of the whole file.
- **Closing a file stops FFmpeg.** Closing a video, opening another, or quitting Trimmy stops the
  work it was doing in the background. Before, a big file could keep a drive busy for an hour
  after it was gone from the screen.

## 1.0.0

The first stable release. If you have 0.2.x, click **[update]** on the open screen.

- **What you see is what you export.** A lossless clip can only start on a keyframe, so the
  start handle now snaps to keyframes (the faint ticks along the bottom of the timeline).
  The preview starts exactly where the exported clip will, instead of the export quietly
  starting up to a few seconds early.
- **Update status everywhere.** Trimmy says `checking for updates...` when it starts, then
  whether you're up to date. When an update is out, a small note with an **[update]** button
  shows on the clip screen too, not only the open screen. If the check fails (offline, for
  example), you can **[retry]**.
- **"Open with" on Windows.** The installer adds Trimmy to Explorer's right-click
  **Open with** menu for video files. It never changes which app opens them by default.
- **Recordings with no stored length** (an OBS recording cut off by a crash or power loss) now
  open: Trimmy measures the video itself instead of showing an empty timeline.
- **Phone videos filmed in portrait** show their real size (1080×1920, not 1920×1080).
- The window title shows the open file's name.
- Smaller fixes: `trimmy clip.mp4` from a terminal accepts a relative path; Ctrl+O works with
  Caps Lock on; files with cover art always export the real video track; screen readers read
  the handle positions as times.

## 0.2.1

- A release to test the in-app updater with a real update from 0.2.0. No other changes.

## 0.2.0

- **Opt-in updates.** Trimmy checks GitHub for a newer version at startup and offers an
  **[update]** button; nothing downloads until you click it. Updates are signature-checked.
- The installed version is shown in the corner of the open screen.
- Faster: FFmpeg is warmed up at launch, the window appears only once it's drawn, waveforms
  are cached, and scrubbing long videos no longer stutters.
- Security hardening: a strict Content Security Policy, and FFmpeg only ever runs on files you
  opened.

## 0.1.1

- Windows installers (per-user setup.exe and an MSI) and an untested macOS build.

## 0.1.0

- First release: open a video, set the start and end on a waveform timeline, export losslessly.
