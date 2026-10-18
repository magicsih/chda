# Bundled fonts

The text faces are JetBrains Mono 2.304, licensed under the adjacent
`JetBrainsMono-OFL.txt`.

The icon fallback is the unmodified `JetBrainsMonoNerdFontMono-Regular.ttf`
from [Nerd Fonts 3.5.1](https://github.com/ryanoasis/nerd-fonts/releases/tag/v3.5.1).
It is also covered by `JetBrainsMono-OFL.txt`; the Nerd Fonts icon license is
retained in `SymbolsNerdFont-LICENSE.txt`.

The complete face supplies both text and icons. GPUI rejects families without
an `m` glyph, so an icon-only Symbols Nerd Font cannot be loaded by its Linux
text system. Using a complete family also removes the need for a separate
CoreText font-file registration path on macOS.

To restore the exact upstream file:

```sh
curl -fL -o JetBrainsMono.tar.xz https://github.com/ryanoasis/nerd-fonts/releases/download/v3.5.1/JetBrainsMono.tar.xz
echo '04d5e8f903693f9dd13e16f867e994834e681eb3c72c0d337a770dcda09010cf  JetBrainsMono.tar.xz' | shasum -a 256 -c -
tar -xOf JetBrainsMono.tar.xz JetBrainsMonoNerdFontMono-Regular.ttf > JetBrainsMonoNerdFontMono-Regular.ttf
```
