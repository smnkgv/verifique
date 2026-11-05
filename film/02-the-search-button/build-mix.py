#!/usr/bin/env python3
"""Mixes a score under the narration: voice in front, music ducked under it, finished at -14 LUFS."""
import json, re, subprocess, sys

take = sys.argv[1]
score = f"/Users/smnkgv/src/verifique-film-assets/02-the-search-button/music/score-{take}.wav"
raw, out = f"out/mix-{take}-raw.wav", f"out/mix-{take}.wav"


def loudness(path):
    text = subprocess.run(["ffmpeg", "-hide_banner", "-i", path, "-af", "ebur128=peak=true", "-f", "null", "-"], capture_output=True, text=True).stderr
    tail = text[text.rindex("Summary:"):]
    return float(re.search(r"I:\s+(-?[\d.]+) LUFS", tail).group(1)), float(re.search(r"Peak:\s+(-?[\d.]+) dBFS", tail).group(1))


shots = {s["lines"][0]["id"]: s for s in json.load(open("timing.json"))["shots"]}


def dip(first):
    """Eases the music down for a beat of silence after each 'Nothing happens', so the moment lands."""
    a = shots[first]["start"] - 0.4
    b = shots[first]["start"] + shots[first]["dur"] - 0.2
    return f"min(min((t-{a})/0.7\\,1)\\,min(({b}-t)/0.9\\,1))"


depth = f"0.72*max(0\\,max({dip('1.4')}\\,{dip('4.4')}))"
graph = ("[0:a]aformat=channel_layouts=stereo,aresample=48000,asplit=2[voice][key];"
         f"[1:a]aresample=48000,volume=-4dB,volume=1-{depth}:eval=frame[music];"
         "[music][key]sidechaincompress=threshold=0.015:ratio=10:attack=25:release=650:knee=4[ducked];"
         "[voice][ducked]amix=inputs=2:normalize=0:duration=first[out]")
subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-i", "out/narration.wav", "-i", score, "-filter_complex", graph, "-map", "[out]", "-ar", "48000", "-c:a", "pcm_s16le", raw], check=True)
level, _ = loudness(raw)
gain = -14 - level
subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-i", raw, "-af", f"volume={gain:.2f}dB,alimiter=limit=0.87:attack=2:release=60:level=false", "-ar", "48000", "-c:a", "pcm_s16le", out], check=True)
print(take, "before", level, "after", loudness(out))
