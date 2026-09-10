# Brand

<p align="center"><img src="wordmark.png" width="560" alt="tungstate"></p>

Three marks, all the same periodic tile. They are not alternatives; each has a job.

| File | Use it for | Why |
|---|---|---|
| `mark-solid.svg` | the app icon | A filled tile is found fastest among thirty others in a dock, and `WO` stays legible at 16px. |
| `mark-outline.svg` | the wordmark, docs, anywhere with room | Matches the interface, where the fluorescent blue is spent sparingly and reserved for the file being verified right now. |
| `mark-compact.svg` | inside the product, 20–70px | Drops the name and corner marks. At that size they are noise, not detail. |

`wordmark.svg` locks the outline mark up with the name. Use it at 560px or wider.

## What the tile says

- **WO** — the tungstate anion, and the only part that has to survive at any size.
- **2−** and **4** — its charge and subscript, set at a fifth of `WO` and pushed
  into matched corners. Chemistry to anyone who looks, texture to anyone who
  does not. Keeping them out of the centre is what lets `WO` sit dead centre and
  large; set inline they crowd it and overflow the tile.
- **TUNGSTATE** — the name.

Tungsten is element 74, `W`. A tungstate is the WO₄²⁻ anion, and *scheelite* is
calcium tungstate, CaWO₄: a dense grey mineral that fluoresces bright blue-white
under ultraviolet light. That fluorescence is where the palette comes from.

## Colour

| | |
|---|---|
| Fluoresce | `#7fd4ff` |
| Ink, on the solid mark | `#0a2230` |
| Plate | `#23272c` → `#13161a` |

Edit the SVGs, never the PNGs. The platform icon set is generated from
`mark-solid.svg`, so regenerate it after any change:

```
cd crates/tungstate-gui
npx --prefix ui tauri icon <a 1024px render of mark-solid.svg> --output icons
rm -rf icons/android icons/ios
```
