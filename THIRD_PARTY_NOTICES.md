# Third-Party Notices & Clean-Room Engineering Attribution

This workspace (`psx-*`) is licensed under the **Mozilla Public License 2.0 (`MPL-2.0`)**.

---

## 1. PCSX-Redux `OpenBIOS` (MIT License)

When using or distributing pre-built images or source derivations of **PCSX-Redux `OpenBIOS`** (`src/mips/openbios` from [`grumpycoders/pcsx-redux`](https://github.com/grumpycoders/pcsx-redux)), the following license applies:

```text
MIT License

Copyright (c) 2019-2026 PCSX-Redux authors (Nicolas "Pixel" Noble and contributors)

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

---

## 2. Hardware Documentation & Clean-Room Reference Policy

1. **Primary Hardware Specification:** Hardware behavior, register maps, cycle timings, and lookup tables (such as the 257-byte GTE `UNR` reciprocal table) are implemented from publicly documented hardware specifications in [**PSX-SPX**](https://psx-spx.consoledev.net/) (originally authored by Martin "nocash" Korth) and verified against bare-metal hardware test suites ([`JaCzekanski/ps1-tests`](https://github.com/JaCzekanski/ps1-tests), [`PeterLemon/PSX`](https://github.com/PeterLemon/PSX), and Amidog's `psxtest_*` ROMs).
2. **Prohibited Proprietary Assets:** No copyrighted Sony PlayStation BIOS ROMs (`scph*.bin`), commercial disc images, or trademarked visual assets may ever be committed to this repository or bundled in release binaries.
3. **DuckStation License Boundary:** In September 2024, DuckStation changed its license to `CC BY-NC-ND 4.0` (forbidding commercial use and all derivative works). **No source code, shaders, or `gamedb` data from post-September-2024 DuckStation may be copied, translated, or referenced** when developing this project. Clean-room references are strictly restricted to public specifications (`PSX-SPX`), permissive MIT/BSD sources (`OpenBIOS`, `chd-rs`), `MPL-2.0` libraries (`librashader`), and independent hardware testing.
