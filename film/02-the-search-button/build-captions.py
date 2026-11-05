#!/usr/bin/env python3
"""Splits each narration line into captions timed to the voice, and writes timing.js for the scene."""
import json, re

MAX = 46
timing = json.load(open("timing.json"))


def real(s):
    return re.sub(r"[Vv]erifeek", "verifique", s)


def pieces(text):
    parts = re.findall(r'.+?(?:[.?!]"?(?=\s|$)|,(?=\s)|$)', text)
    parts = [p.strip() for p in parts if p.strip()]
    out = []
    for part in parts:
        if out and len(out[-1]) + 1 + len(part) <= MAX and not out[-1].endswith((".", "?", "!")):
            out[-1] += " " + part
        else:
            out.append(part)
    return out


for shot in timing["shots"]:
    line = shot["lines"][0]
    text = " ".join(c["text"] for c in line["chunks"]) if "voiced" not in line else line["voiced"]
    line["voiced"] = text
    align = json.load(open(f"out/assets/voice/{line['id']}.align.json"))
    al = align.get("alignment") or align["normalized_alignment"]
    chars, begin, end = al["characters"], al["character_start_times_seconds"], al["character_end_times_seconds"]
    joined = "".join(chars)
    at, chunks = 0, []
    for piece in pieces(text):
        i = joined.index(piece, at)
        j = i + len(piece) - 1
        chunks.append({"text": real(piece), "from": round(line["voiceAt"] + begin[i], 2), "to": round(line["voiceAt"] + end[j], 2)})
        at = j + 1
    line["captions"] = chunks

json.dump(timing, open("timing.json", "w"), indent=1, ensure_ascii=False)
open("timing.js", "w").write("window.TIMING = " + json.dumps(timing, ensure_ascii=False) + ";\n")
for shot in timing["shots"]:
    for c in shot["lines"][0]["captions"]:
        print(c["from"], c["to"], c["text"])
print("duration", timing["duration"])
