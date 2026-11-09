# npiet (vendored conformance oracle)

`npiet` is the de-facto reference interpreter for Piet. It is used only by the
developer `xtask conformance` gate. It is not a dependency of `pietrs`, is
never linked into the shipped binary, and is never committed as a binary.

- Upstream: <https://www.bertnase.de/npiet/>
- File: `npiet-1.3h.c`, version `v1.3h`
- SHA-256: `ffbd0de67691f8500418ae19aaeff5e05990689ed69617b644a1ce89e51d4e57`
- Licence: GNU General Public License, version 2 (see `COPYING`), which matches
  the project licence `GPL-2.0-only`.

The source is vendored byte-for-byte from upstream. Do not edit it. To refresh,
download the same URL and confirm the SHA-256 above before replacing it.

## Building

A C compiler is the only requirement:

```sh
cc -O2 -o target/npiet/npiet vendor/npiet/npiet-1.3h.c
```

The committed source has no GD, GIF, or PNG support, so the oracle is fed PPM
(`P6`) images. `pietrs` decodes the PNG fixture and writes a PPM copy for it.
