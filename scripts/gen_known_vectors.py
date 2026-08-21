"""Regenerate the known-vector fixture that gates ido's semantic index.

docs/mcp-server.md §10 open question 1: *everything downstream is worthless if
the vectors are subtly wrong.* candle reimplements BERT rather than binding
PyTorch, so "it loads and produces 384 normalized floats" proves nothing on its
own. This script produces the other side of the comparison — the vectors the
model's own reference implementation (`sentence-transformers`, i.e. PyTorch +
transformers) yields for a handful of fixed strings — and writes them to
`crates/ido-store/tests/reference/bge-known-vectors.json`, which
`crates/ido-store/tests/known_vector.rs` diffs candle's output against.

`SentenceTransformer("BAAI/bge-small-en-v1.5")` applies the model's own
`modules.json` pipeline: Transformer -> Pooling (**CLS**) -> Normalize. That is
exactly what `ModelSpec { pooling: Cls, normalize: true }` claims, so a match
also validates the registry entry and not merely the forward pass.

Prefixes are checked by construction rather than asserted: the query case is
embedded here as the **literal** prefixed string, while the Rust side embeds
only the bare query with `Role::Query`. They agree only if candle applied
`ModelSpec::query_prefix` — the second of §6.3's two silent quality bugs.

Usage (no uv; a plain venv is enough):

    python -m venv .venv
    .venv/Scripts/python -m pip install sentence-transformers
    .venv/Scripts/python scripts/gen_known_vectors.py

Rerun only when the pinned model revision in `index::embed::BGE_SMALL_EN_V15`
changes — the fixture is a pin, and a regenerated one that "fixes" a failing
test has fixed nothing.
"""

from __future__ import annotations

import datetime as dt
import json
import pathlib

from sentence_transformers import SentenceTransformer

MODEL = "BAAI/bge-small-en-v1.5"
# Must match `ModelSpec::revision` in crates/ido-store/src/index/embed.rs.
REVISION = "5c38ec7c405ec4b44b94cc5a9bb96e735b38267a"
# Must match `ModelSpec::query_prefix`. Applied here by hand, on purpose.
QUERY_PREFIX = "Represent this sentence for searching relevant passages: "

OUT = (
    pathlib.Path(__file__).resolve().parent.parent
    / "crates"
    / "ido-store"
    / "tests"
    / "reference"
    / "bge-known-vectors.json"
)

# `text` is what the Rust side feeds the embedder; `role` is the Role it uses.
# Documents are prefix-free for BGE (`doc_prefix` is empty), so they are also a
# check that candle prepends *nothing* to them.
CASES = [
    ("session keys are kept in the OS keychain", "document"),
    (
        "ido (井戸) is a well — plain markdown, no database; "
        "~450-token chunks, 15% overlap.",
        "document",
    ),
    ("where do we store credentials", "query"),
]


def python_input(text: str, role: str) -> str:
    """The literal string handed to sentence-transformers for a case."""
    return QUERY_PREFIX + text if role == "query" else text


def main() -> None:
    model = SentenceTransformer(MODEL, revision=REVISION)
    inputs = [python_input(text, role) for text, role in CASES]
    # `normalize_embeddings` is redundant (the model ships a Normalize module)
    # and stated anyway, so the fixture cannot depend on modules.json.
    vectors = model.encode(inputs, normalize_embeddings=True)

    import sentence_transformers
    import torch
    import transformers

    payload = {
        "meta": {
            "what": (
                "reference embeddings for the candle known-vector gate "
                "(docs/mcp-server.md §10.1) — regenerate with "
                "scripts/gen_known_vectors.py"
            ),
            "model": MODEL,
            "revision": REVISION,
            "pooling": "cls",
            "normalize": True,
            "query_prefix": QUERY_PREFIX,
            "generated": dt.date.today().isoformat(),
            "packages": {
                "sentence-transformers": sentence_transformers.__version__,
                "transformers": transformers.__version__,
                "torch": torch.__version__,
                "python": __import__("platform").python_version(),
            },
        },
        "vectors": [
            {
                "text": text,
                "role": role,
                "python_input": python_input(text, role),
                "vector": [float(x) for x in vector],
            }
            for (text, role), vector in zip(CASES, vectors)
        ],
    }

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(payload, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"wrote {OUT} ({len(payload['vectors'])} vectors x {len(vectors[0])} dims)")


if __name__ == "__main__":
    main()
