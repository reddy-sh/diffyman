#!/usr/bin/env python3
"""The Maven Central upload of the Java binding, from the files the release built.

    python3 scripts/release/maven_bundle.py bundle --version 0.1.0 --files DIR --out BUNDLE.zip [--sign]
    python3 scripts/release/maven_bundle.py upload --version 0.1.0 --bundle BUNDLE.zip

`bundle` takes DIR, the release-maven artifact of .github/workflows/release.yml
(diffyman-VERSION.jar, -sources.jar, -javadoc.jar and diffyman-VERSION.pom,
nothing else; run verify_sums.py on it first), and writes a Central Portal
bundle: the four files under sh/reddy/diffyman/diffyman/VERSION/ with their
.md5, .sha1, .sha256 and .sha512 checksums and, with --sign, a detached
ASCII-armored GPG signature (.asc) each, made with the default secret key of
GNUPGHOME and the passphrase in MAVEN_GPG_PASSPHRASE (and checked with gpg
--verify). The files are copied byte for byte: nothing is rebuilt, so what
is uploaded is what was checksummed and attested.

`upload` sends a signed bundle to the Central Portal Publisher API with
publishingType=USER_MANAGED (the plugin's autoPublish=false), with the
portal user token in MAVEN_CENTRAL_USERNAME and MAVEN_CENTRAL_PASSWORD,
then waits until the portal has validated the deployment. It never
publishes: the owner reviews the validated deployment at
https://central.sonatype.com/publishing/deployments and presses Publish (or
Drop). When the version is already published, it uploads nothing.

Python 3.11 or later, standard library only (and gpg for --sign).
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path
from typing import Any

GROUP = "sh.reddy.diffyman"
ARTIFACT = "diffyman"
PORTAL = "https://central.sonatype.com"
CHECKSUMS = {"md5": hashlib.md5, "sha1": hashlib.sha1, "sha256": hashlib.sha256, "sha512": hashlib.sha512}
POM_NS = "{http://maven.apache.org/POM/4.0.0}"
# Fixed zip entry times, so the same files give the same bundle.
ZIP_TIME = (1980, 1, 1, 0, 0, 0)


class Problem(Exception):
    pass


def file_names(version: str) -> list[str]:
    stem = f"{ARTIFACT}-{version}"
    return [f"{stem}.pom", f"{stem}.jar", f"{stem}-sources.jar", f"{stem}-javadoc.jar"]


def base_path(version: str) -> str:
    return f"{GROUP.replace('.', '/')}/{ARTIFACT}/{version}"


def check_pom(path: Path, version: str) -> None:
    root = ET.parse(path).getroot()
    found = {key: (root.findtext(f"{POM_NS}{key}") or "").strip() for key in ("groupId", "artifactId", "version", "packaging")}
    expected = {"groupId": GROUP, "artifactId": ARTIFACT, "version": version, "packaging": "jar"}
    if found != expected:
        raise Problem(f"{path.name}: coordinates {found}, expected {expected}")
    for key in ("name", "description", "url", "licenses", "developers", "scm"):
        if root.find(f"{POM_NS}{key}") is None:
            raise Problem(f"{path.name}: no <{key}>, which Maven Central requires")


def sign(path: Path, passphrase: str) -> bytes:
    signature = path.with_name(path.name + ".asc")
    common = ["gpg", "--batch", "--yes", "--pinentry-mode", "loopback", "--passphrase-fd", "0"]
    subprocess.run(
        [*common, "--armor", "--detach-sign", "--output", str(signature), str(path)],
        input=passphrase.encode(),
        check=True,
    )
    subprocess.run(["gpg", "--batch", "--verify", str(signature), str(path)], check=True, capture_output=True)
    return signature.read_bytes()


def bundle(version: str, files: Path, out: Path, signed: bool) -> list[str]:
    names = file_names(version)
    present = sorted(p.name for p in files.iterdir())
    if present != sorted(names):
        raise Problem(f"{files} holds {present}, expected exactly {sorted(names)}")
    check_pom(files / f"{ARTIFACT}-{version}.pom", version)
    passphrase = os.environ.get("MAVEN_GPG_PASSPHRASE", "")
    if signed and not passphrase:
        raise Problem("--sign needs the key's passphrase in MAVEN_GPG_PASSPHRASE")
    entries: list[tuple[str, bytes]] = []
    with tempfile.TemporaryDirectory() as scratch:
        for name in names:
            data = (files / name).read_bytes()
            entries.append((name, data))
            for extension, algorithm in CHECKSUMS.items():
                entries.append((f"{name}.{extension}", algorithm(data).hexdigest().encode()))
            if signed:
                copy = Path(scratch) / name
                copy.write_bytes(data)
                entries.append((f"{name}.asc", sign(copy, passphrase)))
    out.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for name, data in entries:
            info = zipfile.ZipInfo(f"{base_path(version)}/{name}", date_time=ZIP_TIME)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            archive.writestr(info, data)
    return [name for name, _ in entries]


# --- the Publisher API -----------------------------------------------------------


def authorization() -> str:
    username = os.environ.get("MAVEN_CENTRAL_USERNAME", "")
    password = os.environ.get("MAVEN_CENTRAL_PASSWORD", "")
    if not username or not password:
        raise Problem("set MAVEN_CENTRAL_USERNAME and MAVEN_CENTRAL_PASSWORD (a Central Portal user token)")
    return "Bearer " + base64.b64encode(f"{username}:{password}".encode()).decode()


def request(method: str, path: str, query: dict[str, str], body: bytes | None = None, content_type: str | None = None) -> bytes:
    url = f"{PORTAL}{path}?{urllib.parse.urlencode(query)}"
    headers = {"Authorization": authorization(), "User-Agent": "diffyman release (https://github.com/reddy-sh/diffyman)"}
    if content_type:
        headers["Content-Type"] = content_type
    req = urllib.request.Request(url, data=body, method=method, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=300) as response:
            data: bytes = response.read()
            return data
    except urllib.error.HTTPError as error:
        detail = error.read().decode("utf-8", "replace").strip()
        raise Problem(f"{method} {path}: HTTP {error.code} {detail}") from None


def is_published(version: str) -> bool:
    answer: dict[str, Any] = json.loads(
        request("GET", "/api/v1/publisher/published", {"namespace": GROUP, "name": ARTIFACT, "version": version})
    )
    return bool(answer.get("published"))


def multipart(field: str, filename: str, data: bytes) -> tuple[bytes, str]:
    boundary = uuid.uuid4().hex
    head = (
        f"--{boundary}\r\n"
        f'Content-Disposition: form-data; name="{field}"; filename="{filename}"\r\n'
        "Content-Type: application/octet-stream\r\n\r\n"
    ).encode()
    return head + data + f"\r\n--{boundary}--\r\n".encode(), f"multipart/form-data; boundary={boundary}"


def check_signed(path: Path, version: str) -> None:
    with zipfile.ZipFile(path) as archive:
        inside = set(archive.namelist())
    for name in file_names(version):
        for suffix in ("", ".asc", ".md5", ".sha1"):
            if f"{base_path(version)}/{name}{suffix}" not in inside:
                raise Problem(f"{path.name}: no {name}{suffix}; build it with `bundle --sign`")


def upload(version: str, path: Path, wait_minutes: int) -> int:
    check_signed(path, version)
    if is_published(version):
        print(f"{GROUP}:{ARTIFACT}:{version} is already on Maven Central; nothing uploaded")
        return 0
    body, content_type = multipart("bundle", path.name, path.read_bytes())
    deployment = request(
        "POST",
        "/api/v1/publisher/upload",
        {"name": f"{ARTIFACT}-{version}", "publishingType": "USER_MANAGED"},
        body,
        content_type,
    ).decode().strip()
    print(f"uploaded {path.name} as deployment {deployment}; waiting for the portal's validation")
    deadline = time.monotonic() + wait_minutes * 60
    while True:
        status: dict[str, Any] = json.loads(request("POST", "/api/v1/publisher/status", {"id": deployment}))
        state = status.get("deploymentState")
        print(f"deployment {deployment}: {state}")
        if state == "VALIDATED":
            print(
                f"Validated, not published. Review deployment {deployment} at {PORTAL}/publishing/deployments "
                "and press Publish to release it to Maven Central, or Drop to discard it."
            )
            return 0
        if state == "FAILED":
            print(json.dumps(status.get("errors", status), indent=2), file=sys.stderr)
            raise Problem(f"the portal rejected deployment {deployment}")
        if state in ("PUBLISHING", "PUBLISHED"):
            raise Problem(f"deployment {deployment} is {state}, but it was uploaded to wait for a manual release")
        if time.monotonic() > deadline:
            raise Problem(f"deployment {deployment} is still {state} after {wait_minutes} minutes; check the portal")
        time.sleep(10)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    make = sub.add_parser("bundle", help="the Central Portal bundle of the built files")
    make.add_argument("--version", required=True)
    make.add_argument("--files", required=True, type=Path, help="the release-maven artifact")
    make.add_argument("--out", required=True, type=Path)
    make.add_argument("--sign", action="store_true", help="add GPG signatures (MAVEN_GPG_PASSPHRASE)")
    send = sub.add_parser("upload", help="upload a signed bundle as a deployment that waits for a manual release")
    send.add_argument("--version", required=True)
    send.add_argument("--bundle", required=True, type=Path)
    send.add_argument("--wait-minutes", type=int, default=30)
    args = parser.parse_args()
    try:
        if args.command == "bundle":
            entries = bundle(args.version, args.files, args.out, args.sign)
            print(f"{args.out}: {len(entries)} files under {base_path(args.version)}/{'' if args.sign else ' (unsigned)'}")
            for name in entries:
                print(f"  {name}")
            return 0
        return upload(args.version, args.bundle, args.wait_minutes)
    except (Problem, OSError, subprocess.CalledProcessError, ET.ParseError) as problem:
        print(f"maven_bundle.py: {problem}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
