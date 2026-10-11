"""Generate a deterministic PDF with downscaled RGB/alpha and grayscale images.

Original synthetic content; MIT OR Apache-2.0 like Serein. Python stdlib only.
"""
import argparse
from pathlib import Path
import zlib


def document():
    width, height = 1800, 1200
    rgb = bytearray()
    alpha = bytearray()
    for y in range(height):
        for x in range(width):
            # Large recognizable color quadrants plus fine resampling detail.
            color = (32, 100, 200) if x < width // 2 else (210, 64, 40)
            if y >= height // 2:
                color = (40, 180, 90) if x < width // 2 else (170, 80, 190)
            detail = 12 if (x // 9 + y // 11) % 2 else 0
            rgb.extend(c + detail for c in color)
            alpha.append(128 if y < height // 2 else 255)
    gray_width, gray_height = 1200, 400
    gray = bytes(48 if x < gray_width // 2 else 208
                 for y in range(gray_height) for x in range(gray_width))
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 780] "
        b"/Resources << /XObject << /RGB 5 0 R /Gray 7 0 R >> >> "
        b"/Contents 4 0 R >>",
    ]

    def stream(fields, data):
        objects.append(b"<< " + fields + b" /Length " + str(len(data)).encode()
                       + b" >>\nstream\n" + data + b"\nendstream")

    stream(b"", b"q 520 0 0 430 40 290 cm /RGB Do Q\n"
           b"q 520 0 0 170 40 50 cm /Gray Do Q")
    image = (b"/Type /XObject /Subtype /Image /BitsPerComponent 8 "
             b"/Interpolate true /Filter /FlateDecode ")
    size = f"/Width {width} /Height {height} ".encode()
    stream(image + size + b"/ColorSpace /DeviceRGB /SMask 6 0 R", zlib.compress(rgb, 9))
    stream(image + size + b"/ColorSpace /DeviceGray", zlib.compress(alpha, 9))
    stream(image + f"/Width {gray_width} /Height {gray_height} ".encode()
           + b"/ColorSpace /DeviceGray", zlib.compress(gray, 9))
    pdf = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = []
    for index, value in enumerate(objects, 1):
        offsets.append(len(pdf))
        pdf.extend(f"{index} 0 obj\n".encode() + value + b"\nendobj\n")
    xref = len(pdf)
    pdf.extend(f"xref\n0 {len(objects) + 1}\n0000000000 65535 f \n".encode())
    for offset in offsets:
        pdf.extend(f"{offset:010} 00000 n \n".encode())
    pdf.extend(f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\n"
               f"startxref\n{xref}\n%%EOF\n".encode())
    return pdf


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-directory", type=Path, default=Path(__file__).parent)
    directory = parser.parse_args().output_directory
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "pdf-images.pdf").write_bytes(document())
