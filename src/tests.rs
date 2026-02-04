wasmtime::component::bindgen!({
    path: "src/test/wit",
    world: "tests",
    imports: { default: async | trappable },
    exports: { default: async | task_exit },
});

static ENGINE: LazyLock<Engine> = LazyLock::new(|| {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.wasm_component_model_async(true);
    Engine::new(&config).unwrap()
});

async fn pre() -> &'static TestsPre {
    let make = async {
        TestsPre::instantiate_pre(&Component::new(
            &ENGINE,
            crate::componentize(include_str!("tests.wit"), include_str!("tests.js")).await?,
        )?)
    };

    static PRE: OnceCell<TestsPre> = OnceCell::const_new();
    &PRE.get_or_init(async { make.await.unwrap() })
}

#[tokio::test]
async fn simple_export() -> Result<()> {
    let store = Store::new(&ENGINE, Ctx);
    let instance = pre().await.instantiate_async(&mut store).await?;
    assert_eq!(
        42 + 3,
        instance
            .componentize_js_test_simple()
            .call_foo(&mut store, 42)
            .await?
    );
    Ok(())
}
