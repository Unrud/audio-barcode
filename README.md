# Audio Barcode

Send and receive small data packets over sound. The packets have a size of
50 bits and transmitting one packet takes ~1.7 seconds.

Based on the protocol described in
["Chirp technology: an introduction"](https://web.archive.org/web/20120727215947/http://chirp.io/tech/).
It's **not** compatible with the Chirp protocol. The parameters for
error correction are not published in the article.

## Examples

A browser based demo is available at https://unrud.github.io/audio-barcode.
The source code can be found in ``examples/web``.

## License

MIT (see ``LICENSE``)

## Thirdparty libraries

  * **goertzel**
      * Repository: https://github.com/mcpherrinm/goertzel
      * License: MIT
      * Patches:
          * [Use f32 samples](https://github.com/Unrud/audio-barcode/commit/fc992136222b27124089fb086c71ecc474f268cc)
          * [Optimizations](https://github.com/Unrud/audio-barcode/commit/9b32e1e5c68382dd1fc9b0dd3a3c1d8b8ff9f834#diff-c38bb9ffe1f8a2d30f10dbc1c909940d)
  * **reed-solomon-rs**
      * Repository: https://github.com/mersinvald/reed-solomon-rs
      * License: MIT
      * Patches:
          * [Make Galois Field generic: Add GF(2^3) to GF(2^7)](https://github.com/Unrud/audio-barcode/commit/0aec49fff34b3ac0e2875f204820957c0fc65372)
