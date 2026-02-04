#![deny(warnings)]

use {
    anyhow::{Context as _, anyhow},
    bytes::Bytes,
    std::{borrow::Cow, io::Cursor},
    wasm_encoder::{CustomSection, Section as _},
    wasmtime::{
        Config, Engine, Store,
        component::{Component, Linker, ResourceTable},
    },
    wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe},
    wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView},
    wasmtime_wizer::{WasmtimeWizerComponent, Wizer},
    wit_component::metadata,
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

pub struct Ctx {
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

#[expect(clippy::type_complexity)]
pub async fn componentize(
    wit: &str,
    world: Option<&str>,
    js: &str,
    add_to_linker: Option<&dyn Fn(&mut Linker<Ctx>) -> anyhow::Result<()>>,
) -> anyhow::Result<Vec<u8>> {
    let mut resolve = Resolve::default();
    let package = resolve.push_str("wit", wit)?;
    let world = resolve.select_world(&[package], world)?;

    let mut bindings = wit_dylib::create(
        &resolve,
        world,
        Some(&mut DylibOpts {
            interpreter: Some("libcomponentize_js_runtime.so".into()),
            async_: Default::default(),
        }),
    );

    CustomSection {
        name: Cow::Borrowed("component-type:componentize-js"),
        data: Cow::Owned(metadata::encode(
            &resolve,
            world,
            wit_component::StringEncoding::UTF8,
            None,
        )?),
    }
    .append_to(&mut bindings);

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

    let wizer = Wizer::new();
    let (cx, component) = wizer.instrument_component(&component)?;
    let component = Component::new(&engine, &component)?;

    let mut linker = Linker::new(&engine);
    if let Some(add_to_linker) = add_to_linker {
        add_to_linker(&mut linker)?;
    } else {
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    }

    let instance = linker.instantiate_async(&mut store, &component).await?;
    {
        let instance = Init::new(&mut store, &instance)?;
        instance
            .call_init(&mut store, js)
            .await?
            .map_err(|e| anyhow!("{e}"))
            .with_context(move || {
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&stdout.contents()),
                    String::from_utf8_lossy(&stderr.contents())
                )
            })?;
    }

    let component = wizer
        .snapshot_component(
            cx,
            &mut WasmtimeWizerComponent {
                store: &mut store,
                instance,
            },
        )
        .await?;

    tokio::fs::write("/tmp/foo.wasm", &component).await?;

    Ok(component)
}
