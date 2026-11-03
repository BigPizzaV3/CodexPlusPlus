"""从原始四姿态透明图生成配准的十二帧进食序列。"""

from pathlib import Path
import subprocess
from tempfile import TemporaryDirectory

import numpy as np
from PIL import Image


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "assets/rice-maid-sheet.png"
OUTPUT = ROOT / "assets/rice-maid-sequence.webp"
FRAME_SIZE = (256, 284)
ORIGINS = ((236, -10), (536, -10), (236, 275), (536, 275))
LIFT = (0, 2, 5, 7, 7, 7, 6, 5, 4, 3, 2, 1)


def crop_frame(sheet, origin):
    frame = Image.new("RGBA", FRAME_SIZE)
    frame.paste(sheet, (-origin[0], -origin[1]))
    frame.putalpha(frame.getchannel("A").point(lambda value: 0 if value < 24 else value))
    return frame


def lift_bowl(frame, amount):
    if not amount:
        return frame
    pixels = np.asarray(frame, dtype=np.float32)
    height, width = pixels.shape[:2]
    y, x = np.mgrid[:height, :width]
    # 只在碗和持碗的手附近形变，脸、头饰与鞋底保持同一基线。
    falloff = np.exp(-(((x - 133) / 35) ** 4 + ((y - 190) / 36) ** 4))
    source_y = np.clip(y + amount * falloff, 0, height - 1)
    low = np.floor(source_y).astype(np.int32)
    high = np.minimum(low + 1, height - 1)
    weight = (source_y - low)[..., None]
    output = pixels[low, x] * (1 - weight) + pixels[high, x] * weight
    return Image.fromarray(np.uint8(np.clip(output + 0.5, 0, 255)))


def main():
    source = Image.open(SOURCE).convert("RGBA")
    if source.size != (1024, 572):
        raise ValueError(f"unexpected source dimensions: {source.size}")
    anchors = [crop_frame(source, origin) for origin in ORIGINS]
    with TemporaryDirectory(prefix="rice-maid-frames-") as directory:
        temp = Path(directory)
        for index, frame in enumerate((*anchors, anchors[0], anchors[0])):
            frame.save(temp / f"key-{index:02d}.png")
        subprocess.run(
            [
                "ffmpeg", "-hide_banner", "-loglevel", "error", "-framerate", "1",
                "-start_number", "0", "-i", str(temp / "key-%02d.png"),
                "-vf", "minterpolate=fps=3:mi_mode=mci:mc_mode=aobmc:"
                "me=epzs:mb_size=4:search_param=16:scd=none,format=rgba",
                "-frames:v", "13", "-start_number", "0", str(temp / "tween-%02d.png"),
            ],
            check=True,
        )
        atlas = Image.new("RGBA", (FRAME_SIZE[0] * 4, FRAME_SIZE[1] * 3))
        for index in range(12):
            if index % 3 == 0:
                frame = anchors[index // 3]
            else:
                frame = Image.open(temp / f"tween-{index:02d}.png").convert("RGBA")
                # 光流会把透明背景卷进发梢；轮廓沿用最近的原画帧。
                nearest = anchors[((index + 1) // 3) % len(anchors)]
                frame.putalpha(nearest.getchannel("A"))
            frame = lift_bowl(frame, LIFT[index])
            bounds = frame.getchannel("A").getbbox()
            if not bounds or bounds[0] <= 0 or bounds[1] > 20 or bounds[2] >= 256 or not 260 <= bounds[3] < 284:
                raise ValueError(f"frame {index} is clipped or missing shoes: {bounds}")
            atlas.paste(frame, ((index % 4) * FRAME_SIZE[0], (index // 4) * FRAME_SIZE[1]))
        atlas.save(OUTPUT, "WEBP", quality=95, method=6)
    print(f"{OUTPUT}: {OUTPUT.stat().st_size} bytes")


if __name__ == "__main__":
    main()
