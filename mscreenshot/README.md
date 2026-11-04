# mscreenshot

The **screenshot CLI** for margo. Capture a region, a window, or a whole
output to a file and the clipboard, optionally handing off to an annotation
editor.

## Usage

```bash
mscreenshot region        # interactive region select (in-compositor selector)
mscreenshot window        # the focused / picked window
mscreenshot output        # the whole monitor
mscreenshot ocr           # region → OCR (tesseract) → text on the clipboard
mscreenshot qr            # region → QR code / barcode (zbarimg) → clipboard
mscreenshot palette       # region → dominant colours as hex → clipboard
mscreenshot --help
```

Captures land as a file **and** on the clipboard. With `satty` or `swappy`
installed, mscreenshot can pipe the capture straight into the annotation
editor.

`ocr` reads the languages from `--lang`, then `MSCREENSHOT_OCR_LANG`, then
defaults to `eng+tur` narrowed to the installed `tesseract-data-*` packs.
See *Region tools* in `docs/companion-tools.md`.

Bind in `config.conf`:

```
bind = NONE, Print,        spawn, mscreenshot output
bind = SHIFT, Print,       spawn, mscreenshot region
```

## Build

```bash
cargo build --release -p mscreenshot
sudo install -m755 target/release/mscreenshot /usr/bin/mscreenshot
```

## See also

- [`mpicker`](../mpicker/) — colour picker built on the same frozen-screencap
  overlay.
- The shell ships a screenshot widget with RGB readout, magnifier, and inline
  annotation.

## License

GPL-3.0-or-later.
