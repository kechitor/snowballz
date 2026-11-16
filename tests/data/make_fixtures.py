"""Reference data for `cargo test` (run from the project root)."""
import zlib, random, os
from PIL import Image

out = os.path.dirname(os.path.abspath(__file__))
rnd = random.Random(5)
words = [b"snow", b"ball", b"penguin", b"igloo", b"fish", b"crystal", b"\n"]
raw = b"".join(rnd.choice(words) + bytes([rnd.randrange(256)]) * rnd.randrange(3)
               for _ in range(40000))
open(os.path.join(out, "zlib_raw.bin"), "wb").write(raw)
for level in (0, 1, 6, 9):
    open(os.path.join(out, "zlib_%d.bin" % level), "wb").write(zlib.compress(raw, level))
# a short input gets a fixed-Huffman block
c = zlib.compressobj(9, zlib.DEFLATED, 15, 9, zlib.Z_FIXED)
open(os.path.join(out, "zlib_fixed.bin"), "wb").write(
    c.compress(b"hello hello hello hello world\n" * 20) + c.flush())

# decoded pixels (RGBA) of a few game images, for the PNG / JPEG tests
def dump(path, name, mode="RGBA"):
    im = Image.open(path).convert(mode)
    open(os.path.join(out, name), "wb").write(im.tobytes())
dump("data/walk.png", "walk.rgba")
dump("data/themes/snowballz/bg.png", "theme_bg.rgba")
dump("icon.png", "icon.rgba")
im = Image.open("data/bg_menu.jpg").convert("RGB").crop((1000, 400, 1256, 656))
open(os.path.join(out, "bg_menu_crop.rgb"), "wb").write(im.tobytes())
