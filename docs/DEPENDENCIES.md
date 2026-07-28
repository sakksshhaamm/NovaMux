# Dependency review

## Runtime dependencies

None outside the Rust standard library.

The `novamux` executable depends only on the local `novamux-core` workspace
crate. This keeps the initial supply-chain surface minimal and allows fully
offline builds after installing the Rust toolchain.

Every future external crate must document:

- the exact capability it provides;
- its security and unsafe-code exposure;
- maintenance activity;
- a practical replacement or removal path.
