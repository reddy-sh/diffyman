"""Tests for the diffyman Python binding.

Run after `maturin develop` (see bindings/python/README.md). The corpus tests
compare against tests/corpus/*/*/expected.json; the byte-level comparison with
the CLI and WASM lives in tests/conformance/run_bindings.py.
"""

from __future__ import annotations

import json
import pathlib
import threading
from typing import Any

import pytest

import diffyman

REPO = pathlib.Path(__file__).resolve().parents[3]
CORPUS = REPO / "tests" / "corpus"
GROUPS = ("text", "moved")

LEFT = b"apple\nbanana\ncherry\ndate\nelderberry\n"
RIGHT = b"apple\nblueberry\ncherry\ndate\nelderberry\nfig"
# Diff Document spec section 14: the worked example (unnamed inputs give a different id).
WORKED_EXAMPLE_ID = "sha256:488c0d1086c1e8389d24959f51ccf6282d9075b0c715daa4bbe7649fe165ea82"


def without_provenance(doc: dict[str, Any]) -> dict[str, Any]:
    return {k: v for k, v in doc.items() if k != "provenance"}


def corpus_cases() -> list[pathlib.Path]:
    return sorted(
        case
        for group in GROUPS
        for case in (CORPUS / group).iterdir()
        if (case / "options.json").is_file()
    )


def only(case: pathlib.Path, stem: str) -> pathlib.Path:
    (found,) = sorted(p for p in case.iterdir() if p.is_file() and p.name.startswith(f"{stem}."))
    return found


def test_versions() -> None:
    assert diffyman.SCHEMA_VERSION == "0.1.0"
    assert diffyman.__version__ == diffyman.ENGINE_VERSION
    assert diffyman.ENGINE_VERSION


def test_worked_example() -> None:
    text = diffyman.diff(LEFT, RIGHT, {"context": 1}, left_name="left.txt", right_name="right.txt")
    doc = json.loads(text)
    assert doc["id"] == WORKED_EXAMPLE_ID
    assert doc["status"] == "different"
    assert doc["stats"]["similarity"] == 0.7273
    assert [h["id"] for h in doc["hunks"]] == ["h1", "h2"]
    assert doc["provenance"]["binding"] == "python"
    # Canonical JSON: no insignificant whitespace, and the text ends at the closing brace.
    assert text.startswith('{"bytes":[]') and text.endswith("}")
    assert "\n" not in text


def test_diff_document_matches_diff() -> None:
    text = diffyman.diff(LEFT, RIGHT)
    doc = diffyman.diff_document(LEFT, RIGHT)
    assert without_provenance(json.loads(text)) == without_provenance(doc)


@pytest.mark.parametrize("case", corpus_cases(), ids=lambda p: f"{p.parent.name}/{p.name}")
def test_corpus_case(case: pathlib.Path) -> None:
    options = (case / "options.json").read_text(encoding="utf-8")
    doc = diffyman.diff_document(only(case, "left"), only(case, "right"), options)
    expected = json.loads((case / "expected.json").read_text(encoding="utf-8"))
    assert without_provenance(doc) == without_provenance(expected)


def test_input_kinds_agree() -> None:
    reference = without_provenance(diffyman.diff_document(LEFT, RIGHT))
    for left, right in [
        (LEFT.decode(), RIGHT.decode()),
        (bytearray(LEFT), bytearray(RIGHT)),
        (memoryview(LEFT), memoryview(RIGHT)),
    ]:
        assert without_provenance(diffyman.diff_document(left, right)) == reference


def test_path_inputs_use_file_names(tmp_path: pathlib.Path) -> None:
    (tmp_path / "old.txt").write_bytes(LEFT)
    (tmp_path / "new.txt").write_bytes(RIGHT)
    doc = diffyman.diff_document(tmp_path / "old.txt", tmp_path / "new.txt")
    assert doc["left"]["name"] == "old.txt"
    assert doc["right"]["name"] == "new.txt"
    renamed = diffyman.diff_document(tmp_path / "old.txt", tmp_path / "new.txt", left_name="a.txt")
    assert renamed["left"]["name"] == "a.txt"
    assert renamed["id"] != doc["id"]


def test_names_keep_last_component_and_default_to_null() -> None:
    doc = diffyman.diff_document(LEFT, RIGHT, left_name="dir/sub/left.txt", right_name="c:\\x\\r.txt")
    assert doc["left"]["name"] == "left.txt"
    assert doc["right"]["name"] == "r.txt"
    unnamed = diffyman.diff_document(LEFT, RIGHT)
    assert unnamed["left"]["name"] is None
    assert unnamed["right"]["name"] is None


def test_option_forms_agree() -> None:
    as_dict = diffyman.diff_document(LEFT, RIGHT, {"context": 3})
    as_text = diffyman.diff_document(LEFT, RIGHT, '{"context": 3}')
    default = diffyman.diff_document(LEFT, RIGHT, None)
    empty = diffyman.diff_document(LEFT, RIGHT, {})
    assert as_dict["id"] == as_text["id"] == default["id"] == empty["id"]
    assert default["options"]["algorithm"] == "histogram"


@pytest.mark.parametrize(
    "options",
    [{"bogus": True}, {"context": -1}, {"algorithm": "fastest"}, "[1, 2]", "{not json"],
)
def test_invalid_options_raise(options: Any) -> None:
    with pytest.raises(diffyman.DiffymanError, match="invalid options"):
        diffyman.diff(LEFT, RIGHT, options)
    assert issubclass(diffyman.DiffymanError, ValueError)


def test_unsupported_option_gives_error_document() -> None:
    doc = diffyman.diff_document(LEFT, RIGHT, {"detectMoves": True})
    assert doc["status"] == "error"
    assert ("unsupported-option", None) in [(w["code"], w["side"]) for w in doc["warnings"]]
    assert doc["hunks"] == []


def test_invalid_utf8_gives_error_document() -> None:
    doc = diffyman.diff_document(b"\xff\xfe\x00", b"ok\n")
    assert doc["status"] == "error"
    assert doc["stats"] is None
    assert ("invalid-utf8", "left") in [(w["code"], w["side"]) for w in doc["warnings"]]


@pytest.mark.parametrize("bad", [123, None, ["a"], 1.5])
def test_wrong_input_types_raise(bad: Any) -> None:
    with pytest.raises(TypeError):
        diffyman.diff(bad, b"")


def test_wrong_options_type_raises() -> None:
    with pytest.raises(TypeError):
        diffyman.diff(b"", b"", 3)  # type: ignore[arg-type]


def test_threads_get_identical_results() -> None:
    big_left = b"".join(b"line %d\n" % i for i in range(20000))
    big_right = big_left.replace(b"line 777\n", b"line 777 changed\n")
    results: list[str] = []

    def work() -> None:
        results.append(diffyman.diff(big_left, big_right))

    threads = [threading.Thread(target=work) for _ in range(4)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert len(results) == 4
    ids = {json.loads(r)["id"] for r in results}
    assert len(ids) == 1
