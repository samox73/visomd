# visomd

`viso` renders a Markdown file in the terminal. Prose is styled, wrapped terminal text. Every formula, `$...$` inline and `$$...$$` display, is typeset in-process by [RaTeX](https://github.com/erweixin/RaTeX) and placed as a kitty graphics image through Unicode placeholders, so inline math sits on the text baseline and display math scrolls with the surrounding text. One static binary with the fonts embedded; no TeX, Typst or font install.

Requires a terminal that implements the kitty graphics protocol with Unicode placeholders: kitty ≥ 0.28, Ghostty, WezTerm. Anywhere else, or with `--no-images`, math is printed as LaTeX source.

## Install

```sh
cargo install --git https://github.com/samox73/visomd --locked   # the binary is `viso`
```

Every [release](https://github.com/samox73/visomd/releases) ships static Linux binaries (x86_64, aarch64) and an Apple silicon binary as tarballs: unpack and put `viso` on your `PATH`.

## Use

```sh
viso notes.md
cat notes.md | viso
viso --no-images notes.md   # LaTeX source instead of images
viso --debug-term           # what viso detects: size, cell pixels, foreground, graphics, config
```

| Flag | Meaning | Default |
|:--|:--|:--|
| `--width N` | wrap width in columns | terminal width, 80 when piped |
| `--max-width N` | cap on the automatic wrap width | 100 |
| `--display-scale F` | display math size relative to inline math | 1 |
| `--fg RRGGBB` | math ink color | terminal foreground, via OSC 10 |
| `--theme NAME` | syntect theme for code blocks | `base16-ocean.dark` |
| `--no-images` | print math as LaTeX | off |
| `--print-config` | print every config key with its effective value, as a file to copy from | |

## Config

`~/.config/visomd/config.toml` (or `$XDG_CONFIG_HOME/visomd/config.toml`) holds persistent settings, one `key = value` per line named like the flags. Flags override the file. `viso --print-config` prints every key with its current value, ready to paste.

```toml
max-width = 100
display-scale = 1.0
theme = "base16-ocean.dark"
# fg = "d3c6aa"
# no-images = true
```

## What you get

- Headings, emphasis, strong, inline code, links, nested and ordered lists, block quotes, rules, fenced code with syntax highlighting.
- Tables with aligned columns (`:--`, `:-:`, `--:` honored), `│` between columns, a bold header. A table wider than the width narrows its widest columns, never below their longest word, and wraps the cells, with a rule between rows so they stay apart.
- Inline math in TeX text style, sized so the math x-height matches the terminal font's, sharing one baseline with the prose. A formula that would leave its row is shifted by the missing pixels; one taller than a row is shrunk to fit.
- Display math in TeX display style, at the inline size unless `--display-scale` says otherwise, at most four rows tall, scaled down to the width, left-aligned.
- Images, footnotes and raw HTML are printed as text. A formula RaTeX cannot parse is printed as LaTeX with a warning on stderr.

Everything is written once to stdout, image transfers interleaved with the text. Piping into a pager shows the transfers as garbage; a pager mode is planned in `PLAN-0.2.md`. `PLAN.md` describes the design.

## Release

Bump `version` in `Cargo.toml`, commit, tag `vX.Y.Z` and push the tag. The release workflow checks that the tag matches the version, builds the binaries and publishes a GitHub release with generated notes.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
