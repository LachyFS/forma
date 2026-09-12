# Forma application icon

The icon extends the editor's mint outline cube and charcoal palette. It uses an
untextured rounded tile, three restrained face colors, and transparent margins.
The cube's stroke is optically strengthened at 16 and 32 pixels.

The canonical vector definition is `scripts/generate-icon.swift`. Regenerate all
outputs with:

```sh
swift scripts/generate-icon.swift
```

The generator uses only macOS CoreGraphics, ImageIO, and `iconutil`. It creates:

- `Forma.svg`: editable 1024-unit vector master.
- `Forma.png`: 1024-pixel preview with transparency.
- `Forma-64.png`: native-size preview.
- `Forma.icns`: complete macOS icon family, from 16 to 1024 pixels.

The bundle script refreshes an absent or stale icon and installs it in
`Forma.app/Contents/Resources`, with `CFBundleIconFile` in the application plist.
