#!/usr/bin/env python3
"""Read current PNG pixels before choosing native pointer coordinates."""
import csv
import io
import json
import re
import subprocess
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageOps


BACKGROUND = (32, 36, 41)


def largest_region(mask):
    """Run-length connected components avoid isolated same-color text pixels."""
    width,height=mask.size
    data=mask.tobytes();parents=[];runs=[];previous=[]
    def find(index):
        while parents[index]!=index:
            parents[index]=parents[parents[index]];index=parents[index]
        return index
    for y in range(height):
        current=[];cursor=0
        for match in re.finditer(b"\xff+",data[y*width:(y+1)*width]):
            left,right=match.span();index=len(parents);parents.append(index)
            runs.append((index,left,right,y));current.append((left,right,index))
            while cursor<len(previous) and previous[cursor][1]<=left:cursor+=1
            candidate=cursor
            while candidate<len(previous) and previous[candidate][0]<right:
                parent=find(previous[candidate][2]);own=find(index)
                if parent!=own:parents[parent]=own
                candidate+=1
        previous=current
    regions={}
    for index,left,right,y in runs:
        root=find(index)
        if root not in regions:regions[root]=[0,left,y,right,y+1]
        region=regions[root];region[0]+=right-left
        region[1]=min(region[1],left);region[2]=min(region[2],y)
        region[3]=max(region[3],right);region[4]=max(region[4],y+1)
    if not regions:return None,0
    region=max(regions.values(),key=lambda item:item[0])
    return tuple(region[1:]),region[0]


def background_bounds(path, color=BACKGROUND):
    image = Image.open(path).convert("RGB")
    # Mutter's color management transformed configured #202429 to #1c2226
    # in the real ARM fixture. Derive the actual dominant nearby solid color
    # from this PNG before masking; do not assume byte-identical composition.
    nearby=[(count,actual) for count,actual in image.getcolors(image.width*image.height)
            if max(abs(actual[index]-color[index]) for index in range(3))<=8]
    if not nearby or max(nearby)[0]<20000:
        raise AssertionError("No large candidate background near configured color in actual PNG")
    _,observed=max(nearby)
    channels = ImageChops.difference(image, Image.new("RGB", image.size, observed)).split()
    # GPUI's RGB/HSL conversion may round a channel by one integer.
    mask = channels[0].point(lambda value: 255 if value <= 1 else 0)
    for channel in channels[1:]:
        mask = ImageChops.multiply(mask, channel.point(lambda value: 255 if value <= 1 else 0))
    bounds,count = largest_region(mask)
    if bounds is None or count < 20000:
        raise AssertionError("No large candidate background found in actual PNG")
    left, top, right, bottom = bounds
    if right - left < 200 or bottom - top < 150:
        raise AssertionError("Candidate background is too small for window geometry")
    return {"x": left, "y": top, "width": right-left, "height": bottom-top,
            "source": "current PNG largest connected solid background near configured RGB", "matchedPixels": count,
            "configuredRgb":color,"observedRgb":observed}


def ocr_words(path):
    response = subprocess.run(["tesseract", str(path), "stdout", "--psm", "11", "tsv"],
                              check=True, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    return list(csv.DictReader(io.StringIO(response.stdout), delimiter="\t", quoting=csv.QUOTE_NONE))


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
    # Begin inside the first glyph's cell, end after the final glyph.
    # Exact clipboard readback is the acceptance check, not OCR confidence.
    coordinates = [left+1, top+height//2, left+width+2, top+height//2]
    image = Image.open(path).convert("RGB")
    ImageDraw.Draw(image).rectangle((left, top, left+width, top+height), outline="red", width=1)
    image.save(Path(path).with_suffix(".annotated.png"))
    result = {"sentinel": sentinel, "bounds": [left, top, width, height],
              "drag": coordinates, "source": str(path)}
    Path(path).with_suffix(".coordinates.json").write_text(json.dumps(result, indent=2)+"\n")
    return coordinates


def phrase_matches(words, phrase, minimum_confidence=None):
    wanted = phrase.lower().split()
    matches = []
    for index in range(len(words)-len(wanted)+1):
        group = words[index:index+len(wanted)]
        if [re.sub(r"^\W+|\W+$", "", word["text"].strip().lower()) for word in group] != wanted:
            continue
        if len({(word["block_num"],word["par_num"],word["line_num"]) for word in group}) != 1:
            continue
        if minimum_confidence is not None and any(float(word["conf"]) < minimum_confidence for word in group):
            continue
        left = min(int(word["left"]) for word in group)
        top = min(int(word["top"]) for word in group)
        right = max(int(word["left"])+int(word["width"]) for word in group)
        bottom = max(int(word["top"])+int(word["height"]) for word in group)
        matches.append({"at": [(left+right)//2,(top+bottom)//2],
                        "bounds": [left,top,right,bottom], "words": group})
    return matches


def phrase_center(path, phrase):
    words = ocr_words(path)
    Path(path).with_suffix(".control-ocr-raw.json").write_text(json.dumps(words, indent=2)+"\n")
    matches = phrase_matches(words, phrase)
    processing = "original PNG"
    # A mixed dark terminal and lighter modal can hide the modal from
    # Tesseract's automatic threshold. Preserve geometry and actual glyphs,
    # raise their contrast, then require exact words and useful confidence.
    if not matches:
        contrast_path = Path(path).with_suffix(".bright-text160.png")
        image = ImageOps.grayscale(Image.open(path))
        image.point(lambda value: 0 if value >= 160 else 255).save(contrast_path)
        words = ocr_words(contrast_path)
        contrast_path.with_suffix(".ocr.json").write_text(json.dumps(words, indent=2)+"\n")
        matches = phrase_matches(words, phrase, minimum_confidence=70)
        processing = "same-size grayscale brightness >=160 black, lower brightness white"
    if len(matches) != 1:
        raise AssertionError("Expected one rendered control phrase "+repr(phrase)+", found "+str(len(matches)))
    match = matches[0]
    image = Image.open(path)
    if not (0 <= match["at"][0] < image.width and 0 <= match["at"][1] < image.height):
        raise AssertionError("Rendered control coordinate lies outside original PNG")
    Path(path).with_suffix(".control-coordinate.json").write_text(json.dumps({
        "phrase":phrase, "at":match["at"], "bounds":match["bounds"],
        "processing":processing, "words":match["words"], "source":str(path),
    },indent=2)+"\n")
    return match["at"]
