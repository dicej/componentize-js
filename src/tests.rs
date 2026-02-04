use {
    super::Ctx,
    std::sync::LazyLock,
    tokio::sync::OnceCell,
    wasmtime::{
        Config, Engine, Store,
        component::{Component, Linker, ResourceTable},
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

async fn pre() -> &'static TestsPre<Ctx> {
    let make = async {
        let mut linker = Linker::new(&ENGINE);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
        TestsPre::new(linker.instantiate_pre(&Component::new(
            &ENGINE,
            crate::componentize(include_str!("tests.wit"), None, include_str!("tests.js")).await?,
        )?)?)
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
            .componentize_js_test_simple_export()
            .call_foo(&mut store, 42)
            .await?
    );
    Ok(())
}
