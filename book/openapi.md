# OpenAPI reference

`diffyman-server` describes its v1 API in an OpenAPI 3.1 document. A running server serves it at
`GET /openapi.json`, and the same document is in the repository as
[`docs/api/openapi.json`](../docs/api/openapi.json) (download it to generate a client or load it
into an API tool). `cargo test -p diffyman-server --test openapi` keeps the file equal to what the
server serves.

This page is generated from that file each time the site is built. The
[API reference](../docs/api/README.md) explains authentication, limits and error codes, with a
`curl` example for every endpoint.

<!-- openapi-reference -->
