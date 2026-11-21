# Third-party code

Fumen is under the GPL v3 or later (see [LICENSE](LICENSE)). Some of its code follows other
projects closely: their logic is rewritten in Rust or TypeScript, so a skin, a slider, a star
rating or a storyboard comes out the way it does there. Those projects are under the MIT licence,
which asks that their copyright notice and licence text go along with that code. They are in
[LICENSES](LICENSES).

## osu! (ppy Pty Ltd, MIT)

[github.com/ppy/osu](https://github.com/ppy/osu), licence in [LICENSES/osu.txt](LICENSES/osu.txt).

Ported from osu!lazer:

| Where in Fumen | What it follows in lazer |
| --- | --- |
| `src-tauri/src/stars.rs` | the osu!mania difficulty calculator (star rating at any rate) |
| `src-tauri/src/osu.rs` | beatmap decoding, slider paths and their ticks |
| `src-tauri/src/catch.rs` | the osu!catch beatmap converter and processor |
| `src-tauri/src/taiko.rs` | the osu!taiko beatmap converter |
| `src-tauri/src/hitsounds.rs` | how samples, banks and storyboard samples are looked up |
| `src-tauri/src/replay.rs`, `src/lib/osu-replay.ts`, `src/lib/catch-replay.ts` | replay reading and judgements, hit windows |
| `src/lib/skin/ini.ts` | `LegacyDecoder`, `LegacySkinDecoder`, `LegacyManiaSkinDecoder` |
| `src/ui/mania`, `src/ui/osu`, `src/ui/taiko` | the playfields and the legacy, argon and triangles skin components |
| `src/lib/vis/storyboard.ts` | the storyboard decoder and how storyboard sprites play |

lazer's own skin textures and sounds are not part of Fumen: Fumen reads them from the osu!lazer
installed on the computer.

## osu-framework (ppy Pty Ltd, MIT)

[github.com/ppy/osu-framework](https://github.com/ppy/osu-framework), licence in
[LICENSES/osu-framework.txt](LICENSES/osu-framework.txt).

| Where in Fumen | What it follows |
| --- | --- |
| `src-tauri/src/osu.rs` | `PathApproximator` and `CircularArcProperties` (linear, perfect circle, Bézier and Catmull paths) |
| `src/lib/vis/storyboard.ts` | the easing functions |

## ManiaMapAnalyser (Leo_Black, MIT)

[github.com/LeoBlackMT/osumania_map_analyser](https://github.com/LeoBlackMT/osumania_map_analyser),
licence in [src-tauri/src/mma/LICENSE](src-tauri/src/mma/LICENSE). The pattern report and the dan
estimators in `src-tauri/src/mma` are ported from it; it builds on
[Interlude](https://github.com/YAVSRG/YAVSRG)'s pattern analysis,
[Star-Rating-Rebirth](https://github.com/sunnyxxy/Star-Rating-Rebirth) (Sunny) and
[Daniel](https://thebagelofman.github.io/Daniel/).

## Libraries

Libraries Fumen builds with (Rust crates, npm packages) come with their own licences in their
packages. Among them [rosu-pp](https://github.com/MaxOhn/rosu-pp) by MaxOhn (MIT), through its fork
[rosu-pp-gemini](https://github.com/Raregendary/rosu-pp-gemini) (MIT), for star ratings and pp.
