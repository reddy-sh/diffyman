"""Tests for the generated typed views (python/diffyman/types.py).

`types.py` must be what tools/gen_types.py makes of the engine's JSON Schema,
and every corpus document must fit the generated classes exactly: the keys
of each TypedDict, the Literal values and the nullability.
"""

from __future__ import annotations

import importlib.util
import json
import os
import pathlib
import subprocess
import types as pytypes
import typing
from collections.abc import Callable
from typing import Any, Literal, Union, get_args, get_origin, get_type_hints, is_typeddict

import pytest

import diffyman
import diffyman.types as dt

HERE = pathlib.Path(__file__).resolve().parent
BINDING = HERE.parent
REPO = BINDING.parents[1]
CORPUS = REPO / "tests" / "corpus"
GENERATOR = BINDING / "tools" / "gen_types.py"


def load_generator() -> pytypes.ModuleType:
    spec = importlib.util.spec_from_file_location("gen_types", GENERATOR)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


gen_types = load_generator()
render: Callable[[dict[str, Any]], str] = gen_types.render


def test_types_module_is_current() -> None:
    generated = render(diffyman.document_schema())
    current = (BINDING / "python" / "diffyman" / "types.py").read_text(encoding="utf-8")
    assert current == generated, "types.py is out of date: run tools/gen_types.py"


def test_check_flag_reports_a_stale_file(tmp_path: pathlib.Path) -> None:
    out = tmp_path / "types.py"
    assert gen_types.main(["--output", str(out), "--check"]) == 1
    assert gen_types.main(["--output", str(out)]) == 0
    assert gen_types.main(["--output", str(out), "--check"]) == 0


@pytest.mark.skipif("DIFFYMAN_CLI" not in os.environ, reason="set DIFFYMAN_CLI to a built CLI")
def test_schema_matches_the_cli() -> None:
    cli = os.environ["DIFFYMAN_CLI"]
    printed = subprocess.run([cli, "schema"], check=True, capture_output=True).stdout
    assert printed.decode("utf-8") == diffyman._native.document_schema() + "\n"


@pytest.mark.parametrize(
    "schema",
    [
        {"title": "Other"},
        {"title": "DiffDocument", "properties": {"x": {"type": "object", "properties": {}}}},
        {"title": "DiffDocument", "properties": {"x": {"$ref": "other.json"}}},
        {"title": "DiffDocument", "properties": {"x": {"type": "string", "pattern": "a"}}},
        {"title": "DiffDocument", "properties": {"x": {"allOf": [{"type": "string"}]}}},
    ],
)
def test_unknown_constructs_fail_loudly(schema: dict[str, Any]) -> None:
    with pytest.raises(gen_types.SchemaError):
        render(schema)


def test_keyword_keys_and_optional_fields() -> None:
    text = render(
        {
            "title": "DiffDocument",
            "type": "object",
            "properties": {"from": {"type": "integer"}, "note": {"type": ["string", "null"]}},
            "required": ["from"],
        }
    )
    assert '"from": "int",' in text
    assert '"note": NotRequired["str | None"],' in text
    namespace: dict[str, Any] = {}
    exec(compile(text, "types.py", "exec"), namespace)
    assert namespace["DiffDocument"].__required_keys__ == frozenset({"from"})


# --- structural check of real documents against the generated classes ---------


def check(value: Any, tp: Any, where: str) -> None:
    """Fail unless `value` (parsed JSON) fits the annotation `tp` exactly."""
    if tp is Any:
        return
    if is_typeddict(tp):
        assert isinstance(value, dict), f"{where}: expected an object"
        hints = get_type_hints(tp)
        required: frozenset[str] = tp.__required_keys__
        missing = required - value.keys()
        assert not missing, f"{where}: missing {sorted(missing)}"
        extra = value.keys() - hints.keys()
        if tp is not dt.Provenance:  # the only class with additionalProperties
            assert not extra, f"{where}: unexpected {sorted(extra)}"
        for key, hint in hints.items():
            if key in value:
                check(value[key], hint, f"{where}.{key}")
        return
    origin = get_origin(tp)
    if origin is typing.NotRequired:
        check(value, get_args(tp)[0], where)
    elif origin is Literal:
        assert value in get_args(tp), f"{where}: {value!r} not in {get_args(tp)}"
    elif origin in (Union, pytypes.UnionType):
        errors = []
        for option in get_args(tp):
            try:
                check(value, option, where)
                return
            except AssertionError as e:
                errors.append(str(e))
        raise AssertionError(f"{where}: no variant fits: {errors}")
    elif origin is list:
        assert isinstance(value, list), f"{where}: expected an array"
        (item,) = get_args(tp)
        for i, v in enumerate(value):
            check(v, item, f"{where}[{i}]")
    elif origin is dict:
        assert isinstance(value, dict), f"{where}: expected an object"
    elif tp is type(None):
        assert value is None, f"{where}: expected null"
    elif tp is float:
        assert isinstance(value, (int, float)) and not isinstance(value, bool), where
    elif tp in (int, str, bool):
        assert type(value) is tp, f"{where}: expected {tp.__name__}, got {value!r}"
    else:
        raise AssertionError(f"{where}: unhandled annotation {tp!r}")


def corpus_documents() -> list[pathlib.Path]:
    return sorted(CORPUS.glob("*/*/expected.json"))


@pytest.mark.parametrize(
    "expected", corpus_documents(), ids=lambda p: f"{p.parent.parent.name}/{p.parent.name}"
)
def test_corpus_documents_fit_the_types(expected: pathlib.Path) -> None:
    doc = json.loads(expected.read_text(encoding="utf-8"))
    doc.setdefault("provenance", diffyman.diff_document(b"", b"")["provenance"])
    check(doc, dt.DiffDocument, "$")


def test_engine_documents_fit_the_types(tmp_path: pathlib.Path) -> None:
    (tmp_path / "l" / "s").mkdir(parents=True)
    (tmp_path / "r" / "s").mkdir(parents=True)
    (tmp_path / "l" / "s" / "a.txt").write_text("one\ntwo\n", encoding="utf-8")
    (tmp_path / "r" / "s" / "a.txt").write_text("one\n2\n", encoding="utf-8")
    docs: list[dt.DiffDocument] = [
        diffyman.diff_document(b"a\nb\nc\n", b"c\na\nb\n", {"detectMoves": True}),
        diffyman.diff_document(b"\x00\x01", b"\x00\x02"),
        diffyman.diff_document(tmp_path / "l", tmp_path / "r"),
        diffyman.diff_document(b"x", b"y", {"ignore": {"regex": ["("]}}),
    ]
    for doc in docs:
        check(doc, dt.DiffDocument, "$")
    assert [d["kind"] for d in docs] == ["text", "binary", "tree", "text"]


def test_typed_access() -> None:
    doc = diffyman.diff_document(b"a\nb\n", b"a\nc\n")
    stats = doc["stats"]
    assert stats is not None
    status: dt.Status = doc["status"]
    assert (status, stats["linesAdded"]) == ("different", 1)
    op: dt.Op = doc["hunks"][0]["ops"][1]
    assert op["op"] == "replace"
