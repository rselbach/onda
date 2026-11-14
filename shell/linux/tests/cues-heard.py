#!/usr/bin/env python3
"""Sound cue check for scripts/test-linux-dictation.sh.

Takes a recording of what a sink played and the cue files expected in it,
in order. All are 16-bit mono WAV at one sample rate. Fails unless every
cue is in the recording from its first sample to its last.
"""

import array
import math
import sys
import wave

# a sample this loud is a cue, since nothing else plays through the sink
LOUD = 1000


def read(path):
    with wave.open(path, "rb") as wav:
        if (wav.getnchannels(), wav.getsampwidth()) != (1, 2):
            sys.exit(f"{path} is not 16-bit mono")
        samples = array.array("h")
        samples.frombytes(wav.readframes(wav.getnframes()))
        return wav.getframerate(), samples


def dot(left, right):
    return sum(a * b for a, b in zip(left, right))


def likeness(heard, cue):
    """1.0 when `heard` is `cue` at any volume, near 0 when unrelated."""
    scale = math.sqrt(dot(heard, heard) * dot(cue, cue))
    return dot(heard, cue) / scale if scale else 0.0


def main():
    rate, recording = read(sys.argv[1])
    position = 0
    for path in sys.argv[2:]:
        cue_rate, cue = read(path)
        if cue_rate != rate:
            sys.exit(f"{path} is not at the recording's {rate} Hz")
        lead = next(i for i, sample in enumerate(cue) if abs(sample) >= LOUD)
        onset = next(
            (i for i in range(position, len(recording)) if abs(recording[i]) >= LOUD),
            None,
        )
        if onset is None:
            sys.exit(f"{path} was not played")
        # resampling in the audio server moves the first loud sample a little
        best, start = max(
            (likeness(recording[at : at + len(cue)], cue), at)
            for at in range(max(onset - lead - 20, 0), onset - lead + 20)
        )
        if best < 0.95:
            sys.exit(f"{path} was cut short or is another sound: likeness {best:.2f}")
        position = start + len(cue)
        print(f"{path} heard at {start / rate:.2f} s, likeness {best:.3f}")


if __name__ == "__main__":
    main()
