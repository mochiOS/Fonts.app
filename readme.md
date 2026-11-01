# Fonts

Native AppCore / ViewKit font manager for mochiOS. Uses the existing omochi
controls, VK Symbols and the unchanged `docs/resources/appicons/Fonts.app.png`.
The standard language is English; Japanese is available as a preview specimen.

The application toolbar uses ViewKit's `Toolbar` and `ActionToolbar`. At normal
widths, `NavigationSplitView` owns sidebar sizing, pane backgrounds, continuous
corner clipping and input routing. Compact widths retain separate library and
specimen views. The family list keeps its native `ListRow` selection states,
logical-neighbor corners and row gap; its virtualized measurement and hit regions
use the same shared row-height and spacing tokens. Search, filters and the list
receive their actual sidebar viewport after each resize.

## Features

- Search font families and full face names; filter All, User or System.
- Select individual faces, including collection members and duplicate files.
- Type a specimen, switch English/Japanese samples, and adjust 12–96px size.
- Inspect source, full name, variable/monospaced status, glyph count, format and size.
- Install `.ttf`, `.otf`, `.ttc` or `.otc` through the standard Files.app OpenPanel.
- Open font files from Files.app using AppCore's manifest associations and
  document delivery. Preview first, choose Install explicitly, and see whether
  the exact file is already installed, including disabled installations.
- Enable/disable a managed user font file; confirm before removing its entire file.
- Retain selection across refresh/enable changes and distinguish external copies.
- Use master/detail panes at normal widths and a separate detail page below 720px.

Discovery and management run on a worker. The family list paints visible rows
only. System/shared font files are never mutation targets. External user files
can be browsed; import a managed copy before enabling, disabling or removing it.

## Font storage and rendering

AppCore exposes the shared `mochios-fonts` library from `libraries/fonts`.
Native system fonts live in `/system/libraries/fonts`, shared additions in
`/libraries/fonts`, and this manager installs into `$HOME/.local/share/mochios/fonts`.
New names are content-addressed; existing files are never overwritten. Installation
validates size and face metadata before writing, syncs a temporary file, then
publishes it with a no-replace hard link. Disabled files retain the `.disabled`
suffix. Mutations revalidate the selected managed path and digest. A collection
is always managed as one file, with its face count shown in confirmation.

ViewKit loads enabled additions for normal family lookup. Its existing UI,
monospace and Japanese fallback families stay protected. Exact-file preview
references validate allowed roots, digest and face index, so another file with
the same family name is not substituted. Disabled faces remain previewable.
Missing glyphs can still use normal fallback fonts; the UI states this.

A small revision marker notifies running apps. On their next redraw ViewKit
refreshes font databases, measurements, text layouts and CPU/GPU glyph caches.
The marker is polled at most once per second; font files are not reparsed per
frame. Exact-preview cache history is bounded. Manual changes outside this API
may require reopening apps. If notification fails after a successful mutation,
the error explicitly reports that the file change already happened.

## Build and preview

- `mmake fonts-app`: native app build. The existing `mmake fonts` asset target stays unchanged.
- `mmake preview-fonts`: standalone Linux window.
- `mmake ui`: full desktop, catalogue entry and Files.app installation panel.

Linux reads host system font directories but writes only the isolated mochiOS
preview font folder (`MOCHIOS_PREVIEW_CONFIG_ROOT/fonts`). It never changes host
font installations. The full desktop discards its session font folder on exit.
The standalone preview has no file-panel broker; use `mmake ui` for installation.

Opening a font creates a validated, disabled snapshot in the reserved user-font
`.previews` folder. Discovery excludes this folder, so opening does not install
or activate the font. File-backed aliases also let the Linux desktop renderer
read the exact preview. Installation uses the snapshot, never a reopened source.
Snapshots are removed when their last owner closes; storage refuses more than
eight files or 128 MiB. A crash may leave snapshots behind in that folder.

Google Fonts search/download, variable-axis editing and remote font libraries are
not implemented in this local-management stage. No network capability is requested.

## Verification

Tests cover invalid/oversized collections, duplicate publication, protected and
stale files, symlink rejection, enable/disable/delete, notification errors,
collection indices, duplicate-copy selection, search and compact layout,
confirmation cancellation, exact preview data, preservation of OS UI fonts,
bounded preview history, pane-corner backdrops in both themes, and virtualized
row geometry after theme changes. Linux and mochiOS builds are checked; visual and
on-device verification are performed by the user.
