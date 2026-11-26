# Attribution

Every asset in this repository, with its author, source and licence. Code is `MIT OR Apache-2.0`
(`LICENSE-MIT`, `LICENSE-APACHE`); third-party Rust crates keep their own licences.

| Path | What | Author | Source | Licence |
|---|---|---|---|---|
| `apps/abaco-app/src/icon.rs` | The app icon: an abacus in white on `#A23B72`, drawn in code at run time (window icon, title bar, About) | Ábaco contributors | Original work | `MIT OR Apache-2.0` |
| `assets/app-icon/**` | The same icon rendered for installers by `cargo run -p abaco-app --example icons -- assets/app-icon` (or `packaging/icons.sh`): Linux hicolor PNGs, `abaco-1024.png`, `abaco.ico`, `abaco.icns` | Ábaco contributors | Original work, generated from `icon.rs` | `MIT OR Apache-2.0` |
| `packaging/macos/dmg/background.svg`, `packaging/macos/dmg/background.tiff` | The window shown when the macOS DMG opens: Nodalia islands, a counting frame and a sum, in Ábaco's colour. The text is outlined from Sora (see below) | Ábaco contributors | Original work, written by `packaging/macos/dmg/draw_background_abaco.py` on the layout of nodalia-design's `tools/dmg-background` | `MIT OR Apache-2.0`; the outlined letter shapes come from Sora, `OFL-1.1` |
| `packaging/macos/dmg/dmg-layout.DS_Store` | Finder's view settings for that window (sizes and positions only), written by `packaging/macos/dmg/generate.py` | Ábaco contributors | Original work | `MIT OR Apache-2.0` |

Sora is by the Sora Project Authors, [OFL-1.1](https://openfontlicense.org), taken from
[nodalia-design](https://github.com/getnodalia/nodalia-design) (`packages/fonts/sora`). Only its
letter outlines appear, inside the DMG background; no font file is committed or shipped.

No other asset files are committed. Interface icons, colours and the window controls are drawn by
`nodalia-theme`, from [nodalia-design](https://github.com/getnodalia/nodalia-design) (MIT).
Interface text uses egui's default fonts (Ubuntu Light, Hack, Noto Emoji and emoji-icon-font,
under their own open licences, inside the `epaint` crate).
