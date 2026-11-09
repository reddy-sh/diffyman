"""Create a bucket on a local S3-compatible server when the aws CLI is missing.

Usage: s3_bucket.py BUCKET

Reads AWS_ENDPOINT_URL_S3, AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY and
AWS_REGION from the environment (scripts/s3-bucket.sh sets them). Signs
path-style requests with AWS Signature Version 4 using the standard library
only. Prints "exists" or "created"; exits 1 on any other answer. Never prints
a credential.
"""

from __future__ import annotations

import datetime
import hashlib
import hmac
import os
import re
import sys
import urllib.error
import urllib.parse
import urllib.request

EMPTY_SHA256 = hashlib.sha256(b"").hexdigest()


def _hmac(key: bytes, text: str) -> bytes:
    return hmac.new(key, text.encode(), hashlib.sha256).digest()


def sign(
    method: str,
    url: str,
    *,
    region: str,
    access_key: str,
    secret_key: str,
    now: datetime.datetime,
    payload_sha256: str = EMPTY_SHA256,
    service: str = "s3",
) -> dict[str, str]:
    """Returns the headers that authenticate one request (SigV4)."""
    parts = urllib.parse.urlsplit(url)
    amz_date = now.strftime("%Y%m%dT%H%M%SZ")
    day = amz_date[:8]
    headers = {
        "host": parts.netloc,
        "x-amz-content-sha256": payload_sha256,
        "x-amz-date": amz_date,
    }
    signed = ";".join(sorted(headers))
    canonical = "\n".join(
        [
            method,
            urllib.parse.quote(parts.path or "/", safe="/-_.~"),
            parts.query,
            "".join(f"{k}:{headers[k]}\n" for k in sorted(headers)),
            signed,
            payload_sha256,
        ]
    )
    scope = f"{day}/{region}/{service}/aws4_request"
    to_sign = "\n".join(
        [
            "AWS4-HMAC-SHA256",
            amz_date,
            scope,
            hashlib.sha256(canonical.encode()).hexdigest(),
        ]
    )
    key = _hmac(("AWS4" + secret_key).encode(), day)
    for part in (region, service, "aws4_request"):
        key = _hmac(key, part)
    signature = hmac.new(key, to_sign.encode(), hashlib.sha256).hexdigest()
    headers["authorization"] = (
        f"AWS4-HMAC-SHA256 Credential={access_key}/{scope}, "
        f"SignedHeaders={signed}, Signature={signature}"
    )
    del headers["host"]  # urllib sends it
    return headers


def request(method: str, url: str) -> int:
    """Sends a signed request with an empty body; returns the HTTP status."""
    headers = sign(
        method,
        url,
        region=os.environ.get("AWS_REGION", "us-east-1"),
        access_key=os.environ["AWS_ACCESS_KEY_ID"],
        secret_key=os.environ["AWS_SECRET_ACCESS_KEY"],
        now=datetime.datetime.now(datetime.UTC),
    )
    body = b"" if method == "PUT" else None
    req = urllib.request.Request(url, method=method, headers=headers, data=body)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    try:
        with opener.open(req, timeout=10) as response:
            return int(response.status)
    except urllib.error.HTTPError as error:
        return int(error.code)


def main(argv: list[str]) -> int:
    if len(argv) != 2 or not re.fullmatch(r"[a-z0-9][a-z0-9.-]{1,61}[a-z0-9]", argv[1]):
        print(
            "usage: s3_bucket.py BUCKET (3-63 lowercase letters, digits, dots, dashes)",
            file=sys.stderr,
        )
        return 2
    endpoint = os.environ.get("AWS_ENDPOINT_URL_S3", "").rstrip("/")
    if not endpoint.startswith(("http://127.0.0.1", "http://localhost", "https://")):
        print("AWS_ENDPOINT_URL_S3 must be a local http:// or an https:// URL", file=sys.stderr)
        return 2
    url = f"{endpoint}/{argv[1]}"
    status = request("HEAD", url)
    if status == 200:
        print("exists")
        return 0
    if status != 404:
        print(f"HEAD bucket answered HTTP {status}", file=sys.stderr)
        return 1
    status = request("PUT", url)
    if status == 200:
        print("created")
        return 0
    print(f"PUT bucket answered HTTP {status}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
