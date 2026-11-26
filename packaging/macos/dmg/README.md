# macOS DMG window

What Finder shows when the DMG opens: a 660 × 400 pt background with the app icon and the
`Applications` link side by side. `package.sh` copies these files into the image; nothing here is
generated at build time, so the DMG builds with `hdiutil create` alone (no mounted device, no
Finder scripting on CI, nothing extra installed).

| File | What |
|---|---|
| `background.svg` | Source of the background: two Nodalia islands on the window colour, the left one in Ábaco's colour (`#A23B72`) with a counting frame and the sum it adds up to, the right one white under the icons. Its text (Sora) is outlined, so rendering it needs no fonts. Written by `draw_background_abaco.py`. |
| `background.tiff` | The background at 1x (660 × 400 px, 72 dpi) and 2x (1320 × 800 px, 144 dpi) in one HiDPI TIFF (16-colour palette, Deflate, sRGB). Goes to `.background/background.tiff`. |
| `dmg-layout.DS_Store` | Finder's view settings for the volume: window size, icon size 128, `Abaco.app` at (326, 205), `Applications` at (574, 205), and the background. Goes to `.DS_Store` in the image; named so it isn't mistaken for (or ignored like) a Finder-generated `.DS_Store`. |
| `generate.py` | Writes `background.tiff` and `dmg-layout.DS_Store` from the SVG and the layout above. |
| `draw_background_abaco.py` | Writes `background.svg` using the layout and type of nodalia-design's `tools/dmg-background/draw_background.py`, plus Ábaco's own picture and arrow, which that script does not have yet. When it does, run it with `abaco` and delete this file. |

## The volume name has no version, and no accent

The mounted volume is called `Abaco`, not `Abaco 0.1.0` and not `Ábaco`. The layout points at the
background through an alias that includes the volume name, and Finder resolves it by that name:
with a different name the window keeps its size and icon positions but shows no background. The
DMG file name (`abaco-<version>-macos-<arch>.dmg`) still carries the version, and the bundle shows
its display name, Ábaco, in Finder.

## Rules

- **Finder draws the icon labels in black in light and dark mode** when a window has a background,
  so the area under both icons stays light (the white island).
- **Nothing goes inside the icon boxes:** artwork keeps 10 pt clear of each 128 pt icon box and of
  the label strip under it.

## Regenerate

On any OS, from a checkout of this repository next to one of nodalia-design:

```sh
pip install fonttools 'pillow>=12' ds_store==1.3.3 mac_alias==2.2.3
NODALIA_DESIGN=../nodalia-design python3 packaging/macos/dmg/draw_background_abaco.py   # background.svg
python3 packaging/macos/dmg/generate.py                                                 # needs resvg on PATH
```

Both outputs are byte-for-byte reproducible and carry nothing from the machine that ran them: the
SVG renders without any fonts, and `dmg-layout.DS_Store` is written from scratch, its background
alias holding only the volume name and `/.background/background.tiff`. If you add text to the SVG,
outline it. The window is 660 × 432 with Finder's 32 pt title bar; the content area is 660 × 400.

What has not been checked: the DMG itself. It is built by `release-macos.yml` on a macOS runner,
and nobody has opened it in Finder yet.
