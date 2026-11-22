#!/usr/bin/env python3
"""Rebuild the bundled DeepFilterNet3 weights; requires onnx==1.21.0 and numpy."""

import argparse
import hashlib
import io
import tarfile
import urllib.request
from pathlib import Path

import numpy as np
import onnx
from onnx import numpy_helper

COMMIT = "d375b2d8309e0935d165700c91da9de862a99c31"  # Rikorose/DeepFilterNet
ARCHIVE_SHA256 = "c94d91f70911001c946e0fabb4aa9adc37045f45a03b56008cb0c8244cb63616"
OUTPUT = Path(__file__).resolve().parent.parent / "crates/deepfilternet/src/dfn3.weights"

# Tensors in the order `crates/deepfilternet/src/net.rs` reads them: the network file, the
# output name of the node that owns the weights, and the layout the runtime expects.
LAYERS = [
    ("enc", "/erb_conv0/1/Conv_output_0", "input"),
    ("enc", "/erb_conv1/0/Conv_output_0", "depthwise"),
    ("enc", "/erb_conv1/1/Conv_output_0", "pointwise"),
    ("enc", "/erb_conv2/Conv_output_0", "depthwise"),
    ("enc", "/erb_conv2/Conv_1_output_0", "pointwise"),
    ("enc", "/erb_conv3/0/Conv_output_0", "depthwise"),
    ("enc", "/erb_conv3/1/Conv_output_0", "pointwise"),
    ("enc", "/df_conv0/1/Conv_output_0", "input"),
    ("enc", "/df_conv0/2/Conv_output_0", "pointwise"),
    ("enc", "/df_conv1/Conv_output_0", "depthwise"),
    ("enc", "/df_conv1/Conv_1_output_0", "pointwise"),
    ("enc", "/df_fc_emb/0/Einsum_output_0", "grouped"),
    ("enc", "/emb_gru/linear_in/0/Einsum_output_0", "grouped"),
    ("enc", "/emb_gru/GRU_output_0", "gru"),
    ("enc", "/emb_gru/linear_out/0/Einsum_output_0", "grouped"),
    ("erb_dec", "/emb_gru/linear_in/Einsum_output_0", "grouped"),
    ("erb_dec", "/emb_gru/GRU_output_0", "gru"),
    ("erb_dec", "/emb_gru/GRU_1_output_0", "gru"),
    ("erb_dec", "/emb_gru/linear_out/Einsum_output_0", "grouped"),
    ("erb_dec", "/conv3p/0/Conv_output_0", "scale"),
    ("erb_dec", "/convt3/Conv_output_0", "depthwise"),
    ("erb_dec", "/convt3/Conv_1_output_0", "pointwise"),
    ("erb_dec", "/conv2p/Conv_output_0", "scale"),
    ("erb_dec", "/convt2/0/ConvTranspose_output_0", "depthwise"),
    ("erb_dec", "/convt2/1/Conv_output_0", "pointwise"),
    ("erb_dec", "/conv1p/Conv_output_0", "scale"),
    ("erb_dec", "/convt1/ConvTranspose_output_0", "depthwise"),
    ("erb_dec", "/convt1/Conv_output_0", "pointwise"),
    ("erb_dec", "/conv0p/Conv_output_0", "scale"),
    ("erb_dec", "/conv0_out/0/Conv_output_0", "mask"),
    ("df_dec", "/df_gru/linear_in/linear_in.0/Einsum_output_0", "grouped"),
    ("df_dec", "/df_gru/gru/GRU_output_0", "gru"),
    ("df_dec", "/df_gru/gru/GRU_1_output_0", "gru"),
    ("df_dec", "/df_skip/Einsum_output_0", "grouped"),
    ("df_dec", "/df_convp/df_convp.1/Conv_output_0", "pathway"),
    ("df_dec", "/df_convp/df_convp.2/Conv_output_0", "pointwise"),
    ("df_dec", "/df_out/df_out.0/Einsum_output_0", "grouped"),
]


def layout(kind, inputs):
    """Reorders one node's ONNX tensors so every runtime inner loop reads contiguous memory."""
    weight, rest = inputs[0], inputs[1:]
    if kind == "input":  # [out, 1, time, freq] -> [time, freq, out]
        return [weight[:, 0].transpose(1, 2, 0), *rest]
    if kind == "depthwise":  # [channel, 1, 1, tap] -> [tap, channel]
        return [weight[:, 0, 0].T]
    if kind == "pointwise":  # [out, in, 1, 1] -> [in, out]
        return [weight[:, :, 0, 0].T, *rest]
    if kind == "scale":  # [channel, 1, 1, 1] -> [channel]
        return [weight[:, 0, 0, 0], *rest]
    if kind == "mask":  # [1, in, 1, tap] -> [tap, in]
        return [weight[0, :, 0].T, *rest]
    if kind == "pathway":  # [out, in, time, 1] -> [out, time, in]
        return [weight[:, :, :, 0].transpose(0, 2, 1)]
    if kind == "grouped":  # [group, in, out]
        return [weight]
    if kind == "gru":  # input and recurrent [1, 3 * hidden, in] -> [in, 3 * hidden]; both biases
        return [weight[0].T, rest[0][0].T, rest[1][0]]
    raise ValueError(kind)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, help="use a previously downloaded pinned tar.gz")
    args = parser.parse_args()
    if args.archive:
        data = args.archive.read_bytes()
    else:
        url = f"https://raw.githubusercontent.com/Rikorose/DeepFilterNet/{COMMIT}/models/DeepFilterNet3_onnx.tar.gz"
        with urllib.request.urlopen(url, timeout=60) as response:
            data = response.read()
    if hashlib.sha256(data).hexdigest() != ARCHIVE_SHA256:
        raise SystemExit("DeepFilterNet3 archive does not match the pinned SHA-256")

    graphs = {}
    with tarfile.open(fileobj=io.BytesIO(data)) as archive:
        for member in archive:
            if member.name.endswith(".onnx"):
                graph = onnx.load_from_string(archive.extractfile(member).read()).graph
                values = {tensor.name: numpy_helper.to_array(tensor) for tensor in graph.initializer}
                for node in graph.node:
                    if node.op_type == "Constant":
                        values[node.output[0]] = numpy_helper.to_array(node.attribute[0].t)
                nodes = {node.output[0]: node for node in graph.node}
                graphs[Path(member.name).stem] = (nodes, values)

    blob = bytearray()
    for network, output, kind in LAYERS:
        nodes, values = graphs[network]
        inputs = [values[name] for name in nodes[output].input[1:4] if name]
        for tensor in layout(kind, inputs):
            blob += np.ascontiguousarray(tensor, dtype="<f4").tobytes()
    OUTPUT.write_bytes(blob)
    print(f"{OUTPUT.name}: {len(blob)} bytes, SHA-256 {hashlib.sha256(blob).hexdigest()}")


if __name__ == "__main__":
    main()
