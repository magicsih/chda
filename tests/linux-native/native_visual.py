#!/usr/bin/env python3
"""Read current PNG pixels before choosing native pointer coordinates."""
import csv
import io
import json
import re
import subprocess
import time
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


def bright_text_ocr(path, threshold):
    """Preserve original pixels and coordinates while isolating light UI text."""
    contrast_path = Path(path).with_suffix(f".bright-text{threshold}.png")
    image = ImageOps.grayscale(Image.open(path))
    image.point(lambda value: 0 if value >= threshold else 255).save(contrast_path)
    words = ocr_words(contrast_path)
    contrast_path.with_suffix(".ocr.json").write_text(json.dumps(words, indent=2) + "\n")
    return words, contrast_path


def wait_for_rendered(driver, target, prefix, timeout=40):
    """Keep each real capture and wait for normalized text in rendered pixels."""
    deadline = time.monotonic() + timeout
    attempt = 0
    while time.monotonic() < deadline:
        attempt += 1
        image = driver.screenshot(f"{prefix}-{attempt:02d}")
        letters = "".join(char for word in ocr_words(image) for char in word["text"] if char.isalpha()).upper()
        if target in letters:
            return image
    raise AssertionError("Timed out waiting for rendered text: " + target)


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
        words, _ = bright_text_ocr(path, 160)
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


def prompt_region(path):
    """Read one actual light prompt from this PNG; preserve global coordinates."""
    import hashlib

    source = Path(path)
    source_hash = hashlib.sha256(source.read_bytes()).hexdigest()
    image = Image.open(source).convert("RGB")
    # The Linux GPUI fallback prompt is a connected white panel. Do not reuse
    # a previous capture's location, window geometry, or expected text to find it.
    channels = image.split()
    mask = channels[0].point(lambda value: 255 if value >= 245 else 0)
    for channel in channels[1:]:
        mask = ImageChops.multiply(mask, channel.point(lambda value: 255 if value >= 245 else 0))
    bounds, count = largest_region(mask)

    def plausible(box, pixels):
        if box is None:
            return False
        left, top, right, bottom = box
        width, height = right-left, bottom-top
        area = width*height
        return (width >= 160 and height >= 80 and pixels >= 6000
                and pixels >= area*.45 and area < image.width*image.height*.65)

    if not plausible(bounds, count):
        raise AssertionError("No single plausible light prompt found in actual PNG")
    # A second large light panel makes location ambiguous; do not pick one just
    # because it happens to contain an expected word.
    remainder = mask.copy()
    ImageDraw.Draw(remainder).rectangle(
        (bounds[0], bounds[1], bounds[2]-1, bounds[3]-1), fill=0)
    second_bounds, second_count = largest_region(remainder)
    if plausible(second_bounds, second_count):
        raise AssertionError("Multiple plausible light prompts found in actual PNG")
    region_path = source.with_suffix(".prompt-region.png")
    image.crop(bounds).save(region_path)
    response = subprocess.run(
        ["tesseract", str(region_path), "stdout", "--psm", "12", "tsv"],
        check=True, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    local_words = list(csv.DictReader(io.StringIO(response.stdout), delimiter="\t", quoting=csv.QUOTE_NONE))
    words = []
    for word in local_words:
        current = dict(word)
        current["left"] = str(int(word["left"]) + bounds[0])
        current["top"] = str(int(word["top"]) + bounds[1])
        words.append(current)
    text = " ".join(word["text"] for word in words if word["text"].strip())
    if hashlib.sha256(source.read_bytes()).hexdigest() != source_hash:
        raise AssertionError("Original prompt PNG changed during analysis")
    result = {"source": str(source), "sourceSha256": source_hash,
              "analysisPng": str(region_path), "bounds": list(bounds),
              "matchedPixels": count, "text": text, "words": words,
              "coordinateSpace": "original PNG; bounds are left,top,right,bottom",
              "detection": "unique plausible connected RGB channels >=245 panel",
              "processing": "exact unscaled crop, Tesseract PSM12 TSV; no contrast or text replacement"}
    source.with_suffix(".prompt-region.json").write_text(json.dumps(result, indent=2)+"\n")
    return result


def tooltip_region(path, pointer):
    """OCR one single-line tooltip connected to this capture's hover point."""
    import hashlib

    source = Path(path)
    source_hash = hashlib.sha256(source.read_bytes()).hexdigest()
    image = Image.open(source).convert("RGB")
    px, py = pointer
    if not (0 <= px < image.width and 0 <= py < image.height):
        raise AssertionError("Tooltip hover point is outside the actual PNG")
    # Shared product tooltip.rs draws opaque #1e1e2e with a one-pixel border.
    # Select geometry from current pixels before reading any expected words.
    configured = (30, 30, 46)
    channels = ImageChops.difference(image, Image.new("RGB", image.size, configured)).split()
    mask = channels[0].point(lambda value: 255 if value <= 1 else 0)
    for channel in channels[1:]:
        mask = ImageChops.multiply(mask, channel.point(lambda value: 255 if value <= 1 else 0))
    remaining = mask.copy()
    candidates = []
    for _ in range(16):
        box, count = largest_region(remaining)
        if box is None or count < 150:
            break
        left, top, right, bottom = box
        ImageDraw.Draw(remaining).rectangle((left, top, right-1, bottom-1), fill=0)
        width, height = right-left, bottom-top
        dx = max(left-px, px-right, 0)
        dy = max(top-py, py-bottom, 0)
        if (60 <= width <= 640 and 14 <= height <= 80 and count >= 500
                and count >= width*height*.25 and max(dx, dy) <= 64):
            candidates.append((box, count))
    if len(candidates) != 1:
        raise AssertionError("Expected one nearby single-line tooltip panel, found " + str(len(candidates)))
    box, count = candidates[0]
    # Include the border declared by tooltip.rs; do not add text-specific pads.
    bounds = (max(0, box[0]-1), max(0, box[1]-1),
              min(image.width, box[2]+1), min(image.height, box[3]+1))
    region_path = source.with_suffix(".tooltip-region.png")
    image.crop(bounds).save(region_path)
    response = subprocess.run(
        ["tesseract", str(region_path), "stdout", "--psm", "7", "tsv"],
        check=True, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    local_words = list(csv.DictReader(io.StringIO(response.stdout), delimiter="\t", quoting=csv.QUOTE_NONE))
    words = []
    for word in local_words:
        current = dict(word)
        current["left"] = str(int(word["left"]) + bounds[0])
        current["top"] = str(int(word["top"]) + bounds[1])
        words.append(current)
    text = " ".join(word["text"] for word in words if word["text"].strip())
    if hashlib.sha256(source.read_bytes()).hexdigest() != source_hash:
        raise AssertionError("Original tooltip PNG changed during analysis")
    result = {"source": str(source), "sourceSha256": source_hash,
              "analysisPng": str(region_path), "bounds": list(bounds),
              "backgroundBounds": list(box), "matchedPixels": count,
              "hoverPoint": list(pointer), "configuredBackgroundRgb": configured,
              "text": text, "words": words,
              "coordinateSpace": "original PNG; bounds are left,top,right,bottom",
              "detection": "unique source-color connected panel near actual hover point",
              "processing": "exact unscaled crop with declared one-pixel border, Tesseract PSM7 TSV"}
    source.with_suffix(".tooltip-region.json").write_text(json.dumps(result, indent=2)+"\n")
    return result
