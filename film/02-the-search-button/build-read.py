#!/usr/bin/env python3
"""Joins the narration lines with the story's pauses and times every shot from the read."""
import json, subprocess

VOICE = "out/assets/voice"
OPENING = 2.0
PAUSES = [.5, .6, 1.2, 1.8, 1.2, .5, .6, 1.6, 1.0, .6, .6, 1.4, 1.8, .5, .8, 1.0, 1.2, 2.5]


def seconds(path):
    out = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", path], capture_output=True, text=True)
    return float(out.stdout)


timing = json.load(open("timing.json"))
at, inputs, delays = OPENING, [], []
for n, (shot, pause) in enumerate(zip(timing["shots"], PAUSES)):
    line = shot["lines"][0]
    length = seconds(f"{VOICE}/{line['id']}.wav")
    shot["start"], line["voiceAt"], line["seconds"] = round(at if n else 0, 2), round(at, 2), round(length, 2)
    inputs += ["-i", f"{VOICE}/{line['id']}.wav"]
    delays.append(f"[{n}:a]adelay={int(at * 1000)}:all=1[a{n}]")
    at += length + pause
for n, shot in enumerate(timing["shots"]):
    following = timing["shots"][n + 1]["start"] if n + 1 < len(timing["shots"]) else at
    shot["dur"] = round(following - shot["start"], 2)
timing["duration"] = round(at, 2)
json.dump(timing, open("timing.json", "w"), indent=1, ensure_ascii=False)

labels = "".join(f"[a{n}]" for n in range(len(PAUSES)))
mix = f"{labels}amix=inputs={len(PAUSES)}:normalize=0:duration=longest,apad=whole_dur={at}[out]"
subprocess.run(["ffmpeg", "-y", "-loglevel", "error", *inputs, "-filter_complex", ";".join(delays) + ";" + mix, "-map", "[out]", "-ar", "48000", "-ac", "1", "out/narration.wav"], check=True)
subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-i", "out/narration.wav", "-c:a", "aac", "-b:a", "128k", "out/george-read.m4a"], check=True)
print(f"{int(at // 60)}:{int(at % 60):02d}", round(at, 1))
