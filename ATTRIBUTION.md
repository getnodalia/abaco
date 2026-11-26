# Attribution

Every asset in this repository, with its author, source and licence. Code is `MIT OR Apache-2.0`
(`LICENSE-MIT`, `LICENSE-APACHE`); third-party Rust crates keep their own licences.

| Path | What | Author | Source | Licence |
|---|---|---|---|---|

| `assets/icons/abaco-portrait-1024.png`, `assets/icons/abaco*.svg`, `assets/icons/abaco-1024.png` | App icon artwork: John Napier of Merchiston (1550–1617), inventor of logarithms, cut out in black and white on Ábaco's plum | Nodalia, from a 1616 portrait painting (artist unknown) | [Wikimedia Commons](https://commons.wikimedia.org/wiki/File:John_Napier_of_Merchiston,_1616.jpg); made with nodalia-design `tools/app-portrait` | Portrait: public domain. Artwork: `MIT OR Apache-2.0` |
| `assets/icons/hicolor/*/apps/dev.danielmiguelt.Abaco.*` | App icon at each size, on the Colloid application grid | Nodalia | made from the artwork above with nodalia-design `tools/app-portrait/icon-set.py` | `MIT OR Apache-2.0` |

No other asset files are committed. Interface icons, colours and the window controls are drawn by
`nodalia-theme`, from [nodalia-design](https://github.com/getnodalia/nodalia-design) (MIT).
Interface text uses egui's default fonts (Ubuntu Light, Hack, Noto Emoji and emoji-icon-font,
under their own open licences, inside the `epaint` crate).
