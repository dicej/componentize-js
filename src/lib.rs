wasmtime::component::bindgen!({
    path: "init.wit",
    world: "init",
    exports: { default: async },
});

#[cfg(test)]
mod tests;

async fn componentize(wit: &str, world: Option<&str>, js: &str) -> Result<Vec<u8>> {
    let mut resolve = Resolve::default();
    let package = resolve.push_str("wit", wit)?;
    let world = resolve.select_world(&[package], world)?;

    let bindings = wit_dylib::create(
        resolve,
        world,
        Some(DlibOpts {
            interpreter: Some("libcomponentize_js_runtime.so".into()),
            async_: Default::default(),
        }),
    );

    let component = link::link_libraries(&[
        Library {
            name: "libcomponentize_js_runtime.so".into(),
            module: zstd::decode_all(Cursor::new(include_bytes!(concat!(
                env!("OUT_DIR"),
                "/libcomponentize_js_runtime.so.zst"
            ))))?,
            dl_openable: false,
        },
        Library {
            name: "libcomponentize_js_bindings.so".into(),
            module: bindings,
            dl_openable: false,
        },
    ])?;

    let stdout = MemoryOutputPipe::new(10000);
    let stderr = MemoryOutputPipe::new(10000);

    let mut wasi = WasiCtxBuilder::new();
    let wasi = wasi
        .stdin(MemoryInputPipe::new(Bytes::new()))
        .stdout(stdout.clone())
        .stderr(stderr.clone())
        .build();
    let table = ResourceTable::new();

    let mut config = Config::new();
    config.wasm_component_model(true);
    config.wasm_component_model_async(true);

    let engine = Engine::new(&config)?;
    let mut store = Store::new(&engine, Ctx { wasi, table });

    Wizer::new().run_component(store, &component, async |store, component| {
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_async(linker)?;
        let instance = linker.instantiate_async(&mut store, component).await?;
        Init::new(&instance)
            .await?
            .call_init(&mut store, js)
            .await?
            .map_err(|e| anyhow!("{e}"))?;
        Ok(instance)
    })
}
