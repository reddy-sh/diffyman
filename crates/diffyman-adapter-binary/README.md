# diffyman-adapter-binary

The byte-level diff of [diffyman](https://github.com/reddy-sh/diffyman): the changed byte ranges
of any two inputs, exact for small regions and content-defined chunks for large ones, under a
deterministic work budget.

This is a building block of the [`diffyman`](https://crates.io/crates/diffyman) engine; its API
follows the engine's needs and may change in any release. Depend on `diffyman` instead. The
two are released together with the same version.

- [Documentation](https://reddy-sh.github.io/diffyman)

## License

MIT OR Apache-2.0, at your option.
