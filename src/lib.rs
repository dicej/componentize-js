use {
    anyhow::{Context as _, anyhow},
    bytes::Bytes,
    std::io::Cursor,
    wasmtime::{
        Config, Engine, Store,
        component::{Linker, ResourceTable},
    },
    wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe},
    wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView},
    wasmtime_wizer::Wizer,
    wit_dylib::DylibOpts,
    wit_parser::Resolve,
};

wasmtime::component::bindgen!({
    path: "init.wit",
    world: "init",
    exports: { default: async },
});

#[cfg(test)]
mod tests;

struct Ctx {
    wasi: WasiCtx,
    table: ResourceTable,
}

impl WasiView for Ctx {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

pub async fn componentize(wit: &str, world: Option<&str>, js: &str) -> anyhow::Result<Vec<u8>> {
    let mut resolve = Resolve::default();
    let package = resolve.push_str("wit", wit)?;
    let world = resolve.select_world(&[package], world)?;

    let bindings = wit_dylib::create(
        &resolve,
        world,
        Some(&mut DylibOpts {
            interpreter: Some("libcomponentize_js_runtime.so".into()),
            async_: Default::default(),
        }),
    );

    let component = {
        let mut linker = wit_component::Linker::default()
            .validate(true)
            .use_built_in_libdl(true);

        linker = linker.library(
            "libcomponentize_js_runtime.so",
            &zstd::decode_all(Cursor::new(include_bytes!(concat!(
                env!("OUT_DIR"),
                "/libcomponentize_js_runtime.so.zst"
            ))))?,
            false,
        )?;

        linker = linker.library("libcomponentize_js_bindings.so", &bindings, false)?;

        linker = linker.library(
            "libc.so",
            &zstd::decode_all(Cursor::new(include_bytes!(concat!(
                env!("OUT_DIR"),
                "/libc.so.zst"
            ))))?,
            false,
        )?;

        linker = linker.library(
            "libwasi-emulated-getpid.so",
            &zstd::decode_all(Cursor::new(include_bytes!(concat!(
                env!("OUT_DIR"),
                "/libwasi-emulated-getpid.so.zst"
            ))))?,
            false,
        )?;

        linker = linker.adapter(
            "wasi_snapshot_preview1",
            &zstd::decode_all(Cursor::new(include_bytes!(concat!(
                env!("OUT_DIR"),
                "/wasi_snapshot_preview1.reactor.wasm.zst"
            ))))?,
        )?;

        linker.encode().map_err(|e| anyhow::anyhow!(e))
    }?;

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
    config.async_support(true);
    config.wasm_component_model(true);
    config.wasm_component_model_async(true);

    let engine = Engine::new(&config)?;
    let mut store = Store::new(&engine, Ctx { wasi, table });

    Wizer::new()
        .run_component(&mut store, &component, async |mut store, component| {
            let mut linker = Linker::new(&engine);
            wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
            let instance = linker.instantiate_async(&mut store, component).await?;
            {
                let instance = Init::new(&mut store, &instance)?;
                instance
                    .call_init(&mut store, js)
                    .await?
                    .map_err(|e| anyhow!("{e}"))?;
            }
            Ok(instance)
        })
        .await
        .with_context(move || {
            format!(
                "{}{}",
                String::from_utf8_lossy(&stdout.try_into_inner().unwrap()),
                String::from_utf8_lossy(&stderr.try_into_inner().unwrap())
            )
        })
}
