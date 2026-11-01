# Parakeet benchmarks

Measured 2026-10-31 on an Apple M4 Pro (`sysctl -n machdep.cpu.brand_string`),
macOS 27.0.1, release build, FluidInference's CoreML conversions at the
revisions pinned in `src/manifest.rs`. Audio is macOS TTS converted to
16 kHz mono.

```sh
cargo run --release -p onda-cli -- --models-dir target/dev-models \
    transcribe --model parakeet-ultra --language de --bench --stream long.wav
```

## Load

| Model | First load | Later loads |
| --- | --- | --- |
| Parakeet TDT v2 | 12 to 14 s | 0.1 to 0.2 s |
| Parakeet Ultra | 8.5 to 9.8 s | 0.08 to 0.1 s |
| Parakeet Japanese | 8.5 to 9.9 s | 0.09 s |
| Parakeet Unified | 7.0 s | 0.08 s |

The first load of a model folder (and the first after a new binary) waits
while CoreML compiles the encoder for the Neural Engine; later loads hit
CoreML's cache.

## Final decode

| Model | Audio | Decode | Real-time factor |
| --- | --- | --- | --- |
| Parakeet TDT v2 | 1.8 s | 53 ms | 0.029 |
| Parakeet TDT v2 | 14 s | 71 ms | 0.005 |
| Parakeet TDT v2 | 60 s | 308 ms | 0.005 |
| Parakeet TDT v2 | 188 s | 950 ms | 0.005 |
| Parakeet Ultra | 5.2 s | 52 to 64 ms | 0.010 to 0.012 |
| Parakeet Ultra | 72 to 78 s | 430 to 549 ms | 0.006 to 0.007 |
| Parakeet Japanese | 5.6 s | 56 ms | 0.010 |
| Parakeet Japanese | 91 s | 515 ms | 0.006 |
| Parakeet Unified, offline | 5.2 s | 145 to 171 ms | 0.028 to 0.033 |
| Parakeet Unified, offline | 72 s | 2.09 to 2.13 s | 0.029 |
| Parakeet Unified, after previews | 72 s | 17 ms | |

TDT windows decode one after another, so a 15-minute dictation takes about
4.6 s on v2. FluidAudio decodes four windows at once and finishes the 188 s
clip in about 0.4 s. Unified encodes a 6.72 s window for every 0.56 s of
audio, about 16 ms each, so a recording decoded without previews costs
about 3% of its length; with previews, the final decode only flushes the
tail.

## Preview latency (ASR-17)

### Parakeet TDT

Decode of a recording prefix, as each preview tick does. 30 runs after 3
warm-up runs.

| Prefix | v2 median (p95) | Ultra median (p95) | Japanese median (p95) |
| --- | --- | --- | --- |
| 1 s | 38.4 ms (40.7) | 33.3 ms (34.2) | 32.1 ms (33.2) |
| 5 s | 46.3 ms (46.9) | 43.6 ms (44.6) | 36.7 ms (37.5) |
| 10 s | 56.0 ms (57.7) | 55.9 ms (57.5) | 43.7 ms (46.2) |
| 15 s | 64.1 ms (65.5) | 64.9 ms (66.6) | 51.1 ms (53.7) |

A 15 s prefix decodes in well under 100 ms, as ASR-17 expects. Every window
pays about 30 to 40 ms for the preprocessor and the fixed 15 s encoder; the
rest is the TDT decode loop.

### Parakeet Unified

`--stream` replays a dictation: a preview of the growing recording every
300 ms, then the final decode. Each preview encodes only the 0.56 s chunks
that arrived since the last one, so a tick costs one window or none.

| Run | Previews | Median | p95 | Max |
| --- | --- | --- | --- | --- |
| 72 s clip, `--stream` | 239 | 14.8 ms | 16.3 ms | 16.9 ms |
| Live `dictate`, ticks that encoded a window | 21 | 38 ms | | 47 ms |

The live run shares the machine with audio capture and the session engine.
FluidAudio's `StreamingUnifiedAsrManager` takes a median of 15.1 ms per tick
on the same clip. The first preview appears after 1.12 s of audio (one chunk
plus the look-ahead). A stream that restarts from scratch on a long
recording pays for every window at once (`--bench` measures this: 103 ms for
5 s, 389 ms for 15 s); in a dictation that happens only when the first
preview arrives late.

## Parity with FluidAudio

Output is byte-identical to FluidAudio's except for two deliberate
deviations:

- When a long recording ends in silence, FluidAudio does not mark the
  end-aligned final window as last, so its end-of-recording flush never
  runs and the final tokens (usually the closing period) are dropped. Onda
  finalizes that window (Ultra and Japanese; v2 already zero-pads to the
  end).
- At a seam that lands mid-word, Onda lets exactly one window segment the
  seam word (`merge_using_matches` in `src/chunk.rs`), where FluidAudio can
  type it twice.

Before the first deviation, transcripts matched FluidAudio's byte for byte
on every clip tried: Ultra on German, French, Polish, Russian, and English
(5 s and 72 to 78 s clips, with the language passed), Japanese (5.6 s and
91 s), and Unified offline and streamed (5 s and 72 s, against FluidAudio's
streaming manager with the 70/7/7 context); the seam-word fix did not
trigger on them. With the fix, the German, French, and Russian long clips
end with their period again. Unified's mel features are computed in Rust
and match FluidAudio's to 2e-4 on the same input.
