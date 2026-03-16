# Example: `cli`

This is an example of how to use [componentize-js] and [Wasmtime] to build and
run a JS-based component targetting version `0.3.0-rc-2026-01-06` of the
[wasi-cli] `command` world.

[componentize-js]: https://github.com/dicej/componentize-js
[Wasmtime]: https://github.com/bytecodealliance/wasmtime
[wasi-cli]: https://github.com/WebAssembly/WASI/tree/v0.3.0-rc-2026-01-06/proposals/cli/wit-0.3.0-draft

## Prerequisites

* [Rust](https://rustup.rs/)
* `Wasmtime` 41.0.3
* `componentize-js`
* a clone of the `componentize-js` repository

Once you have Rust, you can install `Wasmtime` and `componentize-js` using
`cargo install`:

```
cargo install --version 41.0.3 wasmtime-cli
cargo install --git https://github.com/dicej/componentize-js
```

## Running the demo

```
componentize-js -d ../../wit -w wasi:cli/command@0.3.0-rc-2026-01-06 componentize app.js -o cli.wasm
wasmtime run -Sp3 -Wcomponent-model-async cli.wasm
```

The `wasmtime run` command above should print "Hello, world!".
