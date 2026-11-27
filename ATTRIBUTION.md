# Attribution

Every asset in this repository, with its author, source and licence. Code is `MIT OR Apache-2.0`
(`LICENSE-MIT`, `LICENSE-APACHE`); third-party Rust crates keep their own licences.

| Path | What | Author | Source | Licence |
|---|---|---|---|---|
| `assets/icons/abaco-portrait-1024.png`, `assets/icons/abaco*.svg`, `assets/icons/abaco-1024.png` | App icon artwork: Carl Friedrich Gauss (1777–1855), cut out in black and white on Ábaco's plum | Nodalia, from the 1840 portrait painting by Christian Albrecht Jensen | [Wikimedia Commons](https://commons.wikimedia.org/wiki/File:Carl_Friedrich_Gauss_1840_by_Jensen.jpg); made with nodalia-design `tools/app-portrait` | Portrait: public domain. Artwork: `MIT OR Apache-2.0` |
| `assets/icons/hicolor/*/apps/dev.danielmiguelt.Abaco.*` | App icon at each size, on the Colloid application grid | Nodalia | made from the artwork above with nodalia-design `tools/app-portrait/icon-set.py` | `MIT OR Apache-2.0` |
| `assets/app-icon/abaco.ico`, `assets/app-icon/abaco.icns` | The same icon packed for Windows and macOS installers by `cargo run -p abaco-app --example icons -- assets/app-icon` (or `packaging/icons.sh`); the PNGs are embedded as they are | Nodalia | Generated from `assets/icons/` | `MIT OR Apache-2.0` |
| `packaging/macos/dmg/background.svg`, `packaging/macos/dmg/background.tiff` | The window shown when the macOS DMG opens: Nodalia islands, a counting frame and a sum, in Ábaco's colour. The text is outlined from Sora (see below) | Ábaco contributors | Original work, written by `packaging/macos/dmg/draw_background_abaco.py` on the layout of nodalia-design's `tools/dmg-background` | `MIT OR Apache-2.0`; the outlined letter shapes come from Sora, `OFL-1.1` |
| `packaging/macos/dmg/dmg-layout.DS_Store` | Finder's view settings for that window (sizes and positions only), written by `packaging/macos/dmg/generate.py` | Ábaco contributors | Original work | `MIT OR Apache-2.0` |
| `docs/images/abaco-document-light.png` | README screenshot: a maths document (a cantilever beam with units), light theme | Nodalia contributors | Screenshot of the running app on an original sample document, taken for this README | `MIT OR Apache-2.0`; no third-party content |
| `docs/images/abaco-document-dark.png` | README screenshot: the same document, dark theme | Nodalia contributors | Screenshot of the running app on an original sample document, taken for this README | `MIT OR Apache-2.0`; no third-party content |
| `docs/images/abaco-script-dark.png` | README screenshot: the Ábaco Script workspace with a damped-oscillation plot, dark theme | Nodalia contributors | Screenshot of the running app on an original sample document, taken for this README | `MIT OR Apache-2.0`; no third-party content |
| `docs/images/abaco-blocks-dark.png` | README screenshot: the block-diagram workspace with a servo model, dark theme | Nodalia contributors | Screenshot of the running app on an original sample document, taken for this README | `MIT OR Apache-2.0`; no third-party content |
| `docs/images/abaco-blocks-light.png` | README screenshot: the block-diagram workspace with a servo model, light theme | Nodalia contributors | Screenshot of the running app on an original sample document, taken for this README | `MIT OR Apache-2.0`; no third-party content |

Sora is by the Sora Project Authors, [OFL-1.1](https://openfontlicense.org), taken from
[nodalia-design](https://github.com/getnodalia/nodalia-design) (`packages/fonts/sora`). Only its
letter outlines appear, inside the DMG background; no font file is committed or shipped.

No other asset files are committed. Interface icons, colours and the window controls are drawn by
`nodalia-theme`, from [nodalia-design](https://github.com/getnodalia/nodalia-design) (MIT).
Interface text uses egui's default fonts (Ubuntu Light, Hack, Noto Emoji and emoji-icon-font,
under their own open licences, inside the `epaint` crate).
