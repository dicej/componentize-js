#![deny(warnings)]

use {
    anyhow::{Context as _, anyhow, bail},
    componentize_js::Wit,
    http_body_util::BodyExt as _,
    tokio::fs,
    wasmtime::{
        Config, Engine, Store,
        component::{Component, Instance, Linker, ResourceTable},
    },
    wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView, p2::pipe::MemoryOutputPipe},
    wasmtime_wasi_http::p3::{
        Request, WasiHttpCtx, WasiHttpCtxView, WasiHttpView, bindings::Service,
    },
};

struct MyWasiHttpCtx;

impl WasiHttpCtx for MyWasiHttpCtx {}

struct Ctx {
    wasi: WasiCtx,
    http: MyWasiHttpCtx,
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

impl WasiHttpView for Ctx {
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        WasiHttpCtxView {
            table: &mut self.table,
            ctx: &mut self.http,
        }
    }
}

async fn test(
    component: &[u8],
    fun: impl AsyncFnOnce(&mut Store<Ctx>, &Instance, MemoryOutputPipe) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
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
    let mut store = Store::new(
        &engine,
        Ctx {
            wasi,
            table,
            http: MyWasiHttpCtx,
        },
    );

    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    wasmtime_wasi::p3::add_to_linker(&mut linker)?;
    wasmtime_wasi_http::p3::add_to_linker(&mut linker)?;

    let instance = linker
        .instantiate_async(&mut store, &Component::new(&engine, component)?)
        .await?;

    fun(&mut store, &instance, stdout.clone())
        .await
        .with_context(move || {
            format!(
                "{}{}",
                String::from_utf8_lossy(&stdout.contents()),
                String::from_utf8_lossy(&stderr.contents())
            )
        })?;

    Ok(())
}

#[tokio::test]
async fn cli() -> anyhow::Result<()> {
    test(
        &componentize_js::componentize(
            Wit::Paths(&["wit"]),
            Some("wasi:cli/command@0.3.0-rc-2026-01-06"),
            &[],
            false,
            &fs::read_to_string("examples/cli/app.js").await?,
            Some("examples/cli"),
            None,
        )
        .await?,
        async |store, instance, stdout| {
            let command = wasmtime_wasi::p3::bindings::Command::new(&mut *store, instance)?;
            store
                .run_concurrent(async |store| command.wasi_cli_run().call_run(store).await)
                .await??
                .0
                .map_err(|()| anyhow!("command failed"))?;

            assert_eq!("Hello, world!", String::from_utf8_lossy(&stdout.contents()));

            Ok(())
        },
    )
    .await
}

#[tokio::test]
async fn http() -> anyhow::Result<()> {
    test(
        &componentize_js::componentize(
            Wit::Paths(&["wit"]),
            Some("wasi:http/service@0.3.0-rc-2026-01-06"),
            &[],
            false,
            &fs::read_to_string("examples/http/app.js").await?,
            Some("examples/http"),
            None,
        )
        .await?,
        async |store, instance, _| {
            let service = Service::new(&mut *store, instance)?;

            let body = "’Twas brillig, and the slithy toves
      Did gyre and gimble in the wabe:
All mimsy were the borogoves,
      And the mome raths outgrabe.";

            let request = store.data_mut().table.push(
                Request::from_http(
                    http::Request::builder()
                        .uri("http://localhost/echo")
                        .method(http::Method::POST)
                        .header("content-type", "text/plain")
                        .body(http_body_util::Full::from(body))?,
                )
                .0,
            )?;

            let response = store
                .run_concurrent(async |store| {
                    let response = service
                        .wasi_http_handler()
                        .call_handle(store, request)
                        .await?
                        .0?;

                    let response = store.with(|mut store| {
                        store
                            .get()
                            .table
                            .delete(response)?
                            .into_http(store, async { Ok(()) })
                    })?;

                    let (parts, body) = response.into_parts();
                    let body = body.collect().await.context("failed to collect body")?;

                    anyhow::Ok(http::Response::from_parts(parts, body))
                })
                .await??;

            if !response.status().is_success() {
                bail!("unexpected response status: {}", response.status());
            }
            assert_eq!(
                Some("text/plain"),
                response
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
            );
            assert_eq!(
                body,
                String::from_utf8_lossy(&response.into_body().to_bytes())
            );

            Ok(())
        },
    )
    .await
}
