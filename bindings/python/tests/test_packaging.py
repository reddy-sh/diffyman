"""What the sdist and the wheel carry: both license texts, equal to the repository's."""

from __future__ import annotations

import pathlib
import tomllib

import pytest

PACKAGE = pathlib.Path(__file__).resolve().parents[1]
REPO = PACKAGE.parents[1]


@pytest.mark.parametrize("name", ["LICENSE-MIT", "LICENSE-APACHE"])
def test_license_files_are_listed_and_match_the_repository(name: str) -> None:
    project = tomllib.loads((PACKAGE / "pyproject.toml").read_text(encoding="utf-8"))["project"]
    assert project["license"] == "MIT OR Apache-2.0"
    assert name in project["license-files"]
    assert (PACKAGE / name).read_bytes() == (REPO / name).read_bytes()
