#!/usr/bin/env python3
"""Read current PNG pixels before choosing native pointer coordinates."""
import csv
import io
import json
import re
import subprocess
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw


BACKGROUND = (32, 36, 41)


def background_bounds(path, color=BACKGROUND):
    image = Image.open(path).convert("RGB")
    channels = ImageChops.difference(image, Image.new("RGB", image.size, color)).split()
    # GPUI's RGB/HSL conversion may round a channel by one integer.
    mask = channels[0].point(lambda value: 255 if value <= 1 else 0)
    for channel in channels[1:]:
        mask = ImageChops.multiply(mask, channel.point(lambda value: 255 if value <= 1 else 0))
    bounds = mask.getbbox()
    count = mask.histogram()[255]
    if bounds is None or count < 20000:
        raise AssertionError("No large candidate background found in actual PNG")
    left, top, right, bottom = bounds
    if right - left < 200 or bottom - top < 150:
        raise AssertionError("Candidate background is too small for window geometry")
    return {"x": left, "y": top, "width": right-left, "height": bottom-top,
            "source": "current PNG configured background pixels", "matchedPixels": count}


def ocr_words(path):
    response = subprocess.run(["tesseract", str(path), "stdout", "--psm", "11", "tsv"],
                              check=True, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    return list(csv.DictReader(io.StringIO(response.stdout), delimiter="\t"))


def sentinel_bounds(path, sentinel):
    words = ocr_words(path)
    Path(path).with_suffix(".ocr.json").write_text(json.dumps(words, indent=2) + "\n")
    matches = [word for word in words if word["text"].strip() == sentinel]
    if len(matches) != 1:
        raise AssertionError("Expected exactly one visible sentinel in PNG OCR, found " + str(len(matches)))
    word = matches[0]
    left, top, width, height = (int(word[key]) for key in ["left", "top", "width", "height"])
    if width <= 20 or height <= 5:
        raise AssertionError("Rendered sentinel OCR bounds are implausible")
    # Begin just before its first ink pixel, end just after its last ink pixel.
    # Exact clipboard readback is the acceptance check, not OCR confidence.
    coordinates = [left-1, top+height//2, left+width+2, top+height//2]
    image = Image.open(path).convert("RGB")
    ImageDraw.Draw(image).rectangle((left, top, left+width, top+height), outline="red", width=1)
    image.save(Path(path).with_suffix(".annotated.png"))
    result = {"sentinel": sentinel, "bounds": [left, top, width, height],
              "drag": coordinates, "source": str(path)}
    Path(path).with_suffix(".coordinates.json").write_text(json.dumps(result, indent=2)+"\n")
    return coordinates


def phrase_center(path, phrase):
    words = ocr_words(path)
    wanted = phrase.lower().split()
    matches = []
    for index in range(len(words)-len(wanted)+1):
        group = words[index:index+len(wanted)]
        if [re.sub(r"^\W+|\W+$", "", word["text"].strip().lower()) for word in group] == wanted:
            if len({(word["block_num"],word["par_num"],word["line_num"]) for word in group}) != 1:
                continue
            left = min(int(word["left"]) for word in group)
            top = min(int(word["top"]) for word in group)
            right = max(int(word["left"])+int(word["width"]) for word in group)
            bottom = max(int(word["top"])+int(word["height"]) for word in group)
            matches.append([(left+right)//2,(top+bottom)//2])
    if len(matches) != 1:
        raise AssertionError("Expected one rendered control phrase "+repr(phrase)+", found "+str(len(matches)))
    Path(path).with_suffix(".control-coordinate.json").write_text(json.dumps({"phrase":phrase,"at":matches[0]},indent=2)+"\n")
    return matches[0]
