use {
    super::Ctx,
    std::sync::LazyLock,
    tokio::sync::OnceCell,
    wasmtime::{
        Config, Engine, Store,
        component::{Accessor, Component, HasSelf, Linker, ResourceTable},
    },
    wasmtime_wasi::WasiCtxBuilder,
};

wasmtime::component::bindgen!({
    path: "src/tests.wit",
    world: "tests",
    imports: { default: async | trappable },
    exports: { default: async | task_exit },
});

static ENGINE: LazyLock<Engine> = LazyLock::new(|| {
    let mut config = Config::new();
    config.async_support(true);
    config.wasm_component_model(true);
    config.wasm_component_model_async(true);
    Engine::new(&config).unwrap()
});

fn add_to_linker(linker: &mut Linker<Ctx>) -> anyhow::Result<()> {
    wasmtime_wasi::p2::add_to_linker_async(linker)?;
    Tests::add_to_linker::<_, HasSelf<_>>(linker, |ctx| ctx)
}

async fn pre() -> &'static TestsPre<Ctx> {
    let make = async {
        let mut linker = Linker::new(&ENGINE);
        add_to_linker(&mut linker)?;
        TestsPre::new(
            linker.instantiate_pre(&Component::new(
                &ENGINE,
                crate::componentize(
                    include_str!("tests.wit"),
                    None,
                    include_str!("tests.js"),
                    Some(&add_to_linker),
                )
                .await?,
            )?)?,
        )
    };

    static PRE: OnceCell<TestsPre<Ctx>> = OnceCell::const_new();
    PRE.get_or_init(|| async { make.await.unwrap() }).await
}

fn store() -> Store<Ctx> {
    let wasi = WasiCtxBuilder::new()
        .inherit_stdout()
        .inherit_stderr()
        .build();
    let table = ResourceTable::default();
    Store::new(&ENGINE, Ctx { wasi, table })
}

#[tokio::test]
async fn simple_export() -> anyhow::Result<()> {
    let mut store = store();
    let instance = pre().await.instantiate_async(&mut store).await?;
    assert_eq!(
        42 + 3,
        instance
            .componentize_js_tests_simple_export()
            .call_foo(&mut store, 42)
            .await?
    );
    Ok(())
}

#[tokio::test]
async fn simple_async_export() -> anyhow::Result<()> {
    let mut store = store();
    let instance = pre().await.instantiate_async(&mut store).await?;
    assert_eq!(
        42 + 3,
        store
            .run_concurrent(async |accessor| {
                instance
                    .componentize_js_tests_simple_async_export()
                    .call_foo(accessor, 42)
                    .await
            })
            .await??
            .0
    );
    Ok(())
}

impl componentize_js::tests::simple_import_and_export::Host for Ctx {
    async fn foo(&mut self, v: u32) -> anyhow::Result<u32> {
        Ok(v + 2)
    }
}

#[tokio::test]
async fn simple_import_and_export() -> anyhow::Result<()> {
    let mut store = store();
    let instance = pre().await.instantiate_async(&mut store).await?;
    assert_eq!(
        42 + 3 + 2,
        instance
            .componentize_js_tests_simple_import_and_export()
            .call_foo(&mut store, 42)
            .await?
    );
    Ok(())
}

impl componentize_js::tests::simple_async_import_and_export::Host for Ctx {}

impl componentize_js::tests::simple_async_import_and_export::HostWithStore for HasSelf<Ctx> {
    async fn foo<T>(_: &Accessor<T, Self>, v: u32) -> anyhow::Result<u32> {
        for _ in 0..5 {
            tokio::task::yield_now().await;
        }
        Ok(v + 2)
    }
}

#[tokio::test]
async fn simple_async_import_and_export() -> anyhow::Result<()> {
    let mut store = store();
    let instance = pre().await.instantiate_async(&mut store).await?;
    assert_eq!(
        42 + 3 + 2,
        store
            .run_concurrent(async |accessor| {
                instance
                    .componentize_js_tests_simple_async_import_and_export()
                    .call_foo(accessor, 42)
                    .await
            })
            .await??
            .0
    );
    Ok(())
}
