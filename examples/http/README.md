# Example: `http`

This is an example of how to use [componentize-js] and [Wasmtime] to build and
run a JS-based component targetting version `0.3.0-rc-2026-01-06` of the
[wasi-http] `service` world.

[componentize-js]: https://github.com/bytecodealliance/componentize-js
[Wasmtime]: https://github.com/bytecodealliance/wasmtime
[wasi-http]: https://github.com/WebAssembly/wasi-http

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

First, build the app and run it:

```
componentize-js -d ../../wit -w wasi:http/service@0.3.0-rc-2026-01-06 componentize app.js -o http.wasm
wasmtime serve -Sp3,common -Wcomponent-model-async http.wasm
```

Then, in another terminal, use cURL to send a request to the app:

```
curl -i -H 'content-type: text/plain' --data-binary @- http://127.0.0.1:8080/echo <<EOF
’Twas brillig, and the slithy toves
      Did gyre and gimble in the wabe:
All mimsy were the borogoves,
      And the mome raths outgrabe.
EOF
```

The above should echo the request body in the response.

In addition to the `/echo` endpoint, the app supports a `/hash-all` endpoint
which concurrently downloads one or more URLs and streams the SHA-256 hashes of
their contents.  You can test it with e.g.:

```
curl -i \
    -H 'url: https://webassembly.github.io/spec/core/' \
    -H 'url: https://www.w3.org/groups/wg/wasm/' \
    -H 'url: https://bytecodealliance.org/' \
    http://127.0.0.1:8080/hash-all
```

If you run into any problems, please file an issue!
