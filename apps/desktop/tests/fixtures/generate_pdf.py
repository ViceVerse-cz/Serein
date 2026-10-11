"""Generate the original, deterministic offline PDF fixture using only Python."""
import argparse
from pathlib import Path


def document():
    objects = []

    def obj(value):
        objects.append(value.encode("ascii") if isinstance(value, str) else value)

    def stream(value):
        data = value.encode("ascii")
        obj(
            b"<< /Length " + str(len(data)).encode("ascii")
            + b" >>\nstream\n" + data + b"\nendstream"
        )

    obj("<< /Type /Catalog /Pages 2 0 R >>")
    obj("<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>")
    obj("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 780] "
        "/Resources << /Font << /F1 5 0 R >> >> /Contents 6 0 R >>")
    obj("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 800 600] "
        "/CropBox [20 20 780 580] /Rotate 90 "
        "/Resources << /Font << /F1 5 0 R >> >> /Contents 7 0 R >>")
    obj("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
    stream("""0.96 0.97 0.99 rg 0 0 600 780 re f
0.14 0.22 0.35 rg 0 680 600 100 re f
1 1 1 rg BT /F1 28 Tf 40 724 Td (Serein / PDF preview) Tj ET
0.12 0.18 0.26 rg BT /F1 15 Tf 40 638 Td (Synthetic offline document - page 1 of 2) Tj ET
0.40 0.46 0.54 rg BT /F1 12 Tf 40 607 Td (No account, network, personal data or microphone used.) Tj ET
0.87 0.90 0.95 rg 40 415 520 150 re f
0.12 0.18 0.26 rg BT /F1 20 Tf 60 523 Td (Native attachment preview) Tj ET
BT /F1 13 Tf 60 491 Td (Read a page without leaving the conversation.) Tj ET
BT /F1 13 Tf 60 466 Td (Use the arrows for page 2, or Download the original.) Tj ET
0.22 0.45 0.69 rg 40 312 160 65 re f
0.24 0.61 0.50 rg 220 312 160 65 re f
0.75 0.43 0.35 rg 400 312 160 65 re f
0.12 0.18 0.26 rg BT /F1 14 Tf 40 255 Td (Bounded pages. Explicit preview. Local rendering.) Tj ET
0.40 0.46 0.54 rg BT /F1 11 Tf 40 48 Td (This fixture is development evidence, not a Discord compatibility test.) Tj ET""")
    stream("""0.96 0.97 0.99 rg 0 0 800 600 re f
0.24 0.61 0.50 rg 60 60 680 90 re f
0.12 0.18 0.26 rg BT /F1 25 Tf 70 450 Td (Page 2 - rotation and crop) Tj ET
BT /F1 16 Tf 70 410 Td (This page declares a 90 degree rotation.) Tj ET
BT /F1 14 Tf 70 375 Td (Its crop box excludes a 20-point outer margin.) Tj ET
0.22 0.45 0.69 rg 70 210 250 100 re f
0.75 0.43 0.35 rg 350 210 250 100 re f""")
    pdf = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = []
    for index, value in enumerate(objects, 1):
        offsets.append(len(pdf))
        pdf.extend(f"{index} 0 obj\n".encode("ascii") + value + b"\nendobj\n")
    xref = len(pdf)
    pdf.extend(f"xref\n0 {len(objects) + 1}\n0000000000 65535 f \n".encode("ascii"))
    for offset in offsets:
        pdf.extend(f"{offset:010} 00000 n \n".encode("ascii"))
    pdf.extend(
        f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\n"
        f"startxref\n{xref}\n%%EOF\n".encode("ascii")
    )
    return pdf


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-directory", type=Path, default=Path(__file__).parent)
    directory = parser.parse_args().output_directory
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "pdf-preview.pdf").write_bytes(document())
