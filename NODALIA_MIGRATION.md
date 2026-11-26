# Nodalia migration: Ábaco

Category: new app, built for Nodalia from the start (no upstream, no inherited code).
Status: first version on a pull request, 2026-11-26.

## Current state

See `README.md` for what runs. CI calls Foundation's `rust-gate`, `git-sources` and `agents-check`
workflows, pinned by commit, plus the release-notes check.

## Desired state

Engineering suite look through `nodalia_theme::engineering`, shared with Forja, Torno and Vortia;
release workflows (`release-auto`, `release-linux`, `release-macos` with a DMG window in Ábaco's
colour, `release-windows`) through Foundation's `release-tag` and `release-publish`; the app icon; website page getnodalia.com/abaco; a Nodalia Launcher catalog entry.

## Inherited dependencies still pending

None.

## Reusable Nodalia components

- `nodalia-theme` (egui, `engineering` feature) from nodalia-design: palette, islands, ribbon,
  properties panel, window controls, icons, About page. Ábaco's accent is `Accent::Abaco`.
- `nodalia-update`: the in-app update check.

## NDL work pending

- The app icon is an abacus drawn in code (`apps/abaco-app/src/icon.rs`, original work, in
  `ATTRIBUTION.md`); the Engineering suite's own mark for it is still to come from nodalia-design.
- `nodalia-update` (the in-app update check) is not wired in.
- No release workflows, DMG window or Windows/macOS runs yet.

## Missing tests

- The window is checked by hand with screenshots in light and dark; a headless render test of the
  main screens would catch layout regressions. The view-model and the engine thread are covered by
  unit and integration tests.
