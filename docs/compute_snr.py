#!/usr/bin/env python3
import wave
import struct
import math
import sys

def read_wav_mono_samples(path):
    with wave.open(path, 'rb') as w:
        nch = w.getnchannels()
        nframes = w.getnframes()
        sw = w.getsampwidth()
        raw = w.readframes(nframes)
        fmt = {1: 'b', 2: 'h', 4: 'i'}[sw]
        fmt = '<' + fmt * nframes * nch
        samples = struct.unpack(fmt, raw)
        samples = [float(s) for s in samples]
        if nch == 2:
            left  = samples[0::2]
            right = samples[1::2]
            mono = [(l + r) / 2.0 for l, r in zip(left, right)]
            return mono
        return samples

def snr_db(ref, test):
    n = min(len(ref), len(test))
    ref = ref[:n]
    test = test[:n]
    signal_power = sum(r * r for r in ref) / n
    noise_power = sum((r - t) ** 2 for r, t in zip(ref, test)) / n
    if noise_power == 0:
        return float('inf')
    return 10 * math.log10(signal_power / noise_power)

if __name__ == '__main__':
    ref = read_wav_mono_samples(sys.argv[1])
    test = read_wav_mono_samples(sys.argv[2])
    print(f'{snr_db(ref, test):.2f} dB')
