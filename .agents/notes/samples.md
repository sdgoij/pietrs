# The published sample gallery

Acceptance sweep over <https://www.dangermouse.net/esoteric/piet/samples.html>,
the gallery of sample programs by the language's author. The sweep is the check
that `pietrs` runs the language as it is actually written, not just as the
regression corpus writes it.

## Reproducing it

```sh
mkdir -p target/piet-samples
curl -sS -o target/piet-samples/index.html \
  https://www.dangermouse.net/esoteric/piet/samples.html
# every src="…png|gif" in that page, downloaded next to it
cargo build -r -p cli -p xtask
for image in target/piet-samples/*.png target/piet-samples/*.gif; do
  target/release/xtask.exe conformance "$image" --max-steps 2000000 --timeout-ms 120000
done
```

The images are third-party work with mixed licences, so they are not committed.
They live under `target/` and are downloaded again by the commands above.

`conformance` compares `interp` with npiet on the grid `pietrs` decoded, so it
checks the semantics and the image front end, not npiet's own file parsing.

## Result, 2026-11-10

72 images: 64 produce byte-identical stdout and the same exit reason as npiet.
The eight that do not are all explained, and none is a `pietrs` mis-execution:

| Image | Why it is not a match |
|---|---|
| `fib.gif`, `fibbig.gif` | npiet does not finish these within 120 s; `fibbig1.gif`, the same program, matches. `pietrs` runs all three |
| `helloworld-cmb.png`, `helloworld-cmb-sm.png` | photographs of framed wall art, not program files: the decode is an all-white image, `pietrs` terminates, npiet loops in white |
| `japh_big.png` | a resampled (interpolated) upscale: no codel size has uniform tiles, so the code cannot be recovered by any detector. The 1:1 `japh.png` matches |
| `primetest.png` | a composite figure: two programs side by side plus palette legends, not a runnable file |
| `ROT13_dot_piet_sm.png` | the page says it has black grid lines between codels and "can't be used directly" |
| `sortgnu.png` | stdout is identical, including the sorted list and the licence; `pietrs` then terminates where npiet loops in white |

The remaining gallery files, including every "big" upscale that is a clean
nearest-neighbour resize, decode to the same grid as their 1:1 original and
match npiet.

## What the sweep fixed

The sweep is how these were found; each is a separate commit or a note here.

- **GIF was unsupported.** Twenty gallery files are GIFs, including the canonical
  Hello World (`hw1-1.gif`), the Fibonacci and Hanoi programs, and three of the
  artistic hello worlds. `canvas` now decodes PNG and GIF, taking a GIF's first
  frame, and the public entry point is `CodelGrid::from_image`.
- **npiet writes CRLF on Windows.** Its stdout goes through C stdio in text mode,
  so every program that prints a newline disagreed with `pietrs` by one byte per
  line. `xtask conformance` now normalises `\r\n` to `\n` where that translation
  happens; without it the gate would have rejected the canonical Hello World.
- **Equal step budgets are not equal work.** A blocked white slide is one
  transition for `pietrs` and several for npiet, so when both implementations hit
  their limit the output lengths differ even though the bytes agree. The gate now
  compares the common prefix when both are bounded, and only requires exact
  equality when a run terminates. See `.agents/notes/divergences.md`.

## What the gallery shows about the images themselves

Some published files are illustrations rather than programs, and some "big"
versions are annotated or resampled. A compiler cannot tell those apart from a
program with a huge codel grid: `japh_big.png` decodes as 150 x 150 codels with
codel size 1, which is a valid grid and the wrong program. `pietrs check` reports
the grid and the codel size so the mismatch is visible, but an image that is
neither a clean upscale nor a 1:1 drawing cannot be recovered by any rule; use
the 1:1 original.
