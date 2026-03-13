use {
    anyhow::{Context as _, anyhow},
    componentize_js::Wit,
    tokio::fs,
    wasmtime::{
        Config, Engine, Store,
        component::{Component, Linker, ResourceTable},
    },
    wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView, p2::pipe::MemoryOutputPipe},
};

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

#[tokio::test]
async fn cli() -> anyhow::Result<()> {
    let mut config = Config::new();
    config.async_support(true);
    config.wasm_component_model(true);
    config.wasm_component_model_async(true);

    let engine = Engine::new(&config)?;

    let stdout = MemoryOutputPipe::new(10000);
    let stderr = MemoryOutputPipe::new(10000);

    let wasi = WasiCtxBuilder::new()
        .stdout(stdout.clone())
        .stderr(stderr.clone())
        .build();
    let table = ResourceTable::default();
    let mut store = Store::new(&engine, Ctx { wasi, table });

    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    wasmtime_wasi::p3::add_to_linker(&mut linker)?;

    let instance = linker
        .instantiate_async(
            &mut store,
            &Component::new(
                &engine,
                componentize_js::componentize(
                    Wit::Paths(&["wit"]),
                    Some("wasi:cli/command@0.3.0-rc-2026-01-06"),
                    &[],
                    false,
                    &fs::read_to_string("examples/cli/app.js").await?,
                    None,
                )
                .await?,
            )?,
        )
        .await?;

    let command = wasmtime_wasi::p3::bindings::Command::new(&mut store, &instance)?;
    store
        .run_concurrent(async |store| command.wasi_cli_run().call_run(store).await)
        .await
        .and_then(|v| v.map(|(v, _)| v.map_err(|()| anyhow!("task exited with error"))))
        .flatten()
        .with_context({
            let stdout = stdout.clone();
            move || {
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&stdout.contents()),
                    String::from_utf8_lossy(&stderr.contents())
                )
            }
        })?;

    assert_eq!("Hello, world!", String::from_utf8_lossy(&stdout.contents()));

    Ok(())
}
