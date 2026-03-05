## `componentize-js`, rebooted

This project generates [WebAssembly Components] from JavaScript code.  It is
intended to be a reboot of the existing [ComponentizeJS] project, using
[wit-dylib] instead of generating JS code to handle Component Model ABI details,
which should reduce the amount of code to maintain and provide a modest
performance boost.  It also uses the [mozjs] Rust wrapper around SpiderMonkey,
making it easier to extend the runtime using Rust instead of C++.

[WebAssembly Components]: https://github.com/WebAssembly/component-model
[ComponentizeJS]: https://github.com/bytecodealliance/ComponentizeJS
[wit-dylib]: https://github.com/bytecodealliance/wasm-tools/tree/main/crates/wit-dylib
[mozjs]: https://github.com/servo/mozjs

## Status

As of this writing, the binding generator generates ultra-minimal,
not-very-idiomatic code which works but isn't very pretty.  We plan to improve
that soon.

Note that this project is ultimately intended to become an integral part of
[StarlingMonkey](https://github.com/bytecodealliance/StarlingMonkey), and so
some of the below to-do items may be addressed after that integration happens,
and possibly at a higher level of abstraction outside of this crate.

- [x] support sync and async imports and exports
- [x] support streams and futures
- [x] support imported and exported resources
- [x] support arbitrary WIT types
- [x] add a license (Apache 2 + LLVM exception)
- [ ] run linting and tests in CI
- [ ] add a CLI interface
- [ ] move JS code generation out of guest code to minimize snapshot bloat
- [ ] make codegen match existing `ComponentizeJS` output
- [ ] generate (and validate in CI) TypeScript bindings (can probably use existing `ComponentizeJS` code)
- [ ] resource/stream/future disposal using [`Symbol.dispose`](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/Symbol/dispose)
- [ ] make streams (and futures?) more idiomatic (e.g. `ReadableStream` and `WritableStream`)
- [ ] integrate with (parts of) Servo and StarlingMonkey for WinterCG support
- [ ] add examples and lint and test them in CI (cf. the [`componentize-py` examples](https://github.com/bytecodealliance/componentize-py/tree/main/examples))
- [ ] investigate options (e.g. GC pinning?) for zero-copy `stream<u8>` reads and writes

## Building and Running

First, install [Rust](https://rustup.rs/) and the `wasm32-wasip1` target if you
don't already have them:

```
rustup update
rustup target add wasm32-wasip1
```

Next, install WASI-SDK 30 and point `WASI_SDK_PATH` to wherever you installed
it.  Replace `arm64-linux` with `x86_64-linux`, `arm64-macos`, `x86_64-macos`,
`arm64-windows`, or `x86_64-windows` below depending on your architecture and OS,
if necessary.

```shell
curl -LO https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-30/wasi-sdk-30.0-arm64-linux.tar.gz
tar xf wasi-sdk-30.0-arm64-linux.tar.gz
sudo mv wasi-sdk-30.0-arm64-linux /opt/
export WASI_SDK_PATH=/opt/wasi-sdk-30.0-arm64-linux
```

Finally, build and run the tests (CLI interface coming soon!):

```shell
cargo test --release
```
