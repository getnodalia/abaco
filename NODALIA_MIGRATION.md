# Nodalia migration: Ábaco

Category: new app, built for Nodalia from the start (no upstream, no inherited code).
Status: first version on a pull request, 2026-11-26.

## Current state

See `README.md` for what runs. CI calls Foundation's `rust-gate`, `git-sources` and `agents-check`
workflows, pinned by commit, plus the release-notes check.

Release workflows exist: `release-auto` (tags a merged version bump through Foundation's
`release-tag`, publishes through `release-publish`), `release-linux` (AppImage, .deb, .rpm, .tar.gz),
`release-macos` (DMG (ad hoc signed, not notarized) with its own install window in Ábaco's colour) and `release-windows`
(MSI and portable zip), with the scripts in `packaging/`. Only the Linux packaging has been run.
Nothing has been released yet.

## Desired state

Engineering suite look through `nodalia_theme::engineering`, shared with Forja, Torno and Vortia;
website page getnodalia.com/abaco; a Nodalia Launcher catalog entry.

## Inherited dependencies still pending

None.

## Reusable Nodalia components

- `nodalia-theme` (egui, `engineering` feature) from nodalia-design: palette, islands, ribbon,
  properties panel, window controls, icons, About page. Ábaco's accent is `Accent::Abaco`.
- `nodalia-update`: the in-app update check.

## NDL work pending

- The app icon is a portrait of Carl Friedrich Gauss (`assets/icons/`, made with `tools/app-portrait` in
  nodalia-design); `assets/app-icon/` (.ico, .icns) is built from it by `packaging/icons.sh`.
- `nodalia-update` (the in-app update check) is not wired in.
- The macOS DMG and the Windows MSI and zip have not been built: the workflows and scripts exist
  but their first run is the first release. The Windows setup dialogs are Pliego's
  `installer-ui.wxs` with the name changed, not regenerated with `craft-installer`.
- The window's Windows executable embeds no icon resource (the installer ships `abaco.ico` beside
  it), and the app has no portable-data mode or macOS file-open handler.
- The cargo binaries are `abaco-app` (window) and `abaco` (command line), the other way round
  from every Nodalia app; `packaging/env.sh` renames them when staging.
- The DMG motif for Ábaco lives in `packaging/macos/dmg/draw_background_abaco.py` and should move
  into nodalia-design's `draw_background.py`.
- No real signing: macOS is signed ad hoc and not notarized, Windows unsigned, until secrets are set.

## Missing tests

- The window is checked by hand with screenshots in light and dark; a headless render test of the
  main screens would catch layout regressions. The view-model and the engine thread are covered by
  unit and integration tests.
