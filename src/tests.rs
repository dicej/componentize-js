use {
    super::Ctx,
    proptest::{
        prelude::{Just, Strategy},
        test_runner::{self, TestRng, TestRunner},
    },
    rand::RngExt,
    std::{env, sync::LazyLock},
    tokio::{runtime::Runtime, sync::OnceCell},
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

impl componentize_js::tests::echoes::Host for Ctx {
    async fn echo_nothing(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn echo_bool(&mut self, v: bool) -> anyhow::Result<bool> {
        Ok(v)
    }

    async fn echo_u8(&mut self, v: u8) -> anyhow::Result<u8> {
        Ok(v)
    }

    async fn echo_s8(&mut self, v: i8) -> anyhow::Result<i8> {
        Ok(v)
    }

    async fn echo_u16(&mut self, v: u16) -> anyhow::Result<u16> {
        Ok(v)
    }

    async fn echo_s16(&mut self, v: i16) -> anyhow::Result<i16> {
        Ok(v)
    }

    async fn echo_u32(&mut self, v: u32) -> anyhow::Result<u32> {
        Ok(v)
    }

    async fn echo_s32(&mut self, v: i32) -> anyhow::Result<i32> {
        Ok(v)
    }

    async fn echo_char(&mut self, v: char) -> anyhow::Result<char> {
        Ok(v)
    }

    async fn echo_u64(&mut self, v: u64) -> anyhow::Result<u64> {
        Ok(v)
    }

    async fn echo_s64(&mut self, v: i64) -> anyhow::Result<i64> {
        Ok(v)
    }

    async fn echo_f32(&mut self, v: f32) -> anyhow::Result<f32> {
        Ok(v)
    }

    async fn echo_f64(&mut self, v: f64) -> anyhow::Result<f64> {
        Ok(v)
    }

    async fn echo_string(&mut self, v: String) -> anyhow::Result<String> {
        Ok(v)
    }

    async fn echo_list_bool(&mut self, v: Vec<bool>) -> anyhow::Result<Vec<bool>> {
        Ok(v)
    }

    async fn echo_list_u8(&mut self, v: Vec<u8>) -> anyhow::Result<Vec<u8>> {
        Ok(v)
    }

    async fn echo_list_s8(&mut self, v: Vec<i8>) -> anyhow::Result<Vec<i8>> {
        Ok(v)
    }

    async fn echo_list_u16(&mut self, v: Vec<u16>) -> anyhow::Result<Vec<u16>> {
        Ok(v)
    }

    async fn echo_list_s16(&mut self, v: Vec<i16>) -> anyhow::Result<Vec<i16>> {
        Ok(v)
    }

    async fn echo_list_u32(&mut self, v: Vec<u32>) -> anyhow::Result<Vec<u32>> {
        Ok(v)
    }

    async fn echo_list_s32(&mut self, v: Vec<i32>) -> anyhow::Result<Vec<i32>> {
        Ok(v)
    }

    async fn echo_list_char(&mut self, v: Vec<char>) -> anyhow::Result<Vec<char>> {
        Ok(v)
    }

    async fn echo_list_u64(&mut self, v: Vec<u64>) -> anyhow::Result<Vec<u64>> {
        Ok(v)
    }

    async fn echo_list_s64(&mut self, v: Vec<i64>) -> anyhow::Result<Vec<i64>> {
        Ok(v)
    }

    async fn echo_list_f32(&mut self, v: Vec<f32>) -> anyhow::Result<Vec<f32>> {
        Ok(v)
    }

    async fn echo_list_f64(&mut self, v: Vec<f64>) -> anyhow::Result<Vec<f64>> {
        Ok(v)
    }

    async fn echo_list_string(&mut self, v: Vec<String>) -> anyhow::Result<Vec<String>> {
        Ok(v)
    }

    async fn echo_list_list_u8(&mut self, v: Vec<Vec<u8>>) -> anyhow::Result<Vec<Vec<u8>>> {
        Ok(v)
    }

    async fn echo_list_list_list_u8(
        &mut self,
        v: Vec<Vec<Vec<u8>>>,
    ) -> anyhow::Result<Vec<Vec<Vec<u8>>>> {
        Ok(v)
    }

    async fn echo_option_u8(&mut self, v: Option<u8>) -> anyhow::Result<Option<u8>> {
        Ok(v)
    }

    async fn echo_option_option_u8(
        &mut self,
        v: Option<Option<u8>>,
    ) -> anyhow::Result<Option<Option<u8>>> {
        Ok(v)
    }

    async fn echo_many(
        &mut self,
        v1: bool,
        v2: u8,
        v3: u16,
        v4: u32,
        v5: u64,
        v6: i8,
        v7: i16,
        v8: i32,
        v9: i64,
        v10: f32,
        v11: f64,
        v12: char,
        v13: String,
        v14: Vec<bool>,
        v15: Vec<u8>,
        v16: Vec<u16>,
    ) -> anyhow::Result<(
        bool,
        u8,
        u16,
        u32,
        u64,
        i8,
        i16,
        i32,
        i64,
        f32,
        f64,
        char,
        String,
        Vec<bool>,
        Vec<u8>,
        Vec<u16>,
    )> {
        Ok((
            v1, v2, v3, v4, v5, v6, v7, v8, v9, v10, v11, v12, v13, v14, v15, v16,
        ))
    }
}

fn get_seed() -> anyhow::Result<[u8; 32]> {
    let seed = if let Ok(seed) = env::var("COMPONENTIZE_JS_TEST_SEED") {
        <[u8; 32]>::try_from(hex::decode(&seed)?.as_slice())?
    } else {
        rand::rng().random()
    };

    eprintln!(
        "using seed {} (set COMPONENTIZE_JS_TEST_SEED env var to override)",
        hex::encode(seed)
    );

    Ok(seed)
}

static SEED: LazyLock<[u8; 32]> = LazyLock::new(|| get_seed().unwrap());

fn proptest<S: Strategy>(
    strategy: &S,
    test: impl AsyncFn(S::Value) -> anyhow::Result<()>,
) -> anyhow::Result<()>
where
    S::Value: Send + Sync + 'static,
{
    let runtime = Runtime::new()?;
    let config = test_runner::Config::default();
    let algorithm = config.rng_algorithm;
    let mut runner = TestRunner::new_with_rng(config, TestRng::from_seed(algorithm, &*SEED));

    Ok(runner.run(strategy, move |v| {
        runtime.block_on(test(v)).unwrap();
        Ok(())
    })?)
}

#[test]
fn echo_nothing() -> anyhow::Result<()> {
    proptest(&Just(()), async |()| {
        let mut store = store();
        let instance = pre().await.instantiate_async(&mut store).await?;
        instance
            .componentize_js_tests_echoes()
            .call_echo_nothing(&mut store)
            .await
    })
}

#[test]
fn echo_bools() -> anyhow::Result<()> {
    proptest(&proptest::bool::ANY, async |value| {
        let mut store = store();
        let instance = pre().await.instantiate_async(&mut store).await?;
        assert_eq!(
            value,
            instance
                .componentize_js_tests_echoes()
                .call_echo_bool(&mut store, value)
                .await?
        );
        Ok(())
    })
}

#[test]
fn echo_u8s() -> anyhow::Result<()> {
    proptest(&proptest::num::u8::ANY, async |value| {
        let mut store = store();
        let instance = pre().await.instantiate_async(&mut store).await?;
        assert_eq!(
            value,
            instance
                .componentize_js_tests_echoes()
                .call_echo_u8(&mut store, value)
                .await?
        );
        Ok(())
    })
}

#[test]
fn echo_s8s() -> anyhow::Result<()> {
    proptest(&proptest::num::i8::ANY, async |value| {
        let mut store = store();
        let instance = pre().await.instantiate_async(&mut store).await?;
        assert_eq!(
            value,
            instance
                .componentize_js_tests_echoes()
                .call_echo_s8(&mut store, value)
                .await?
        );
        Ok(())
    })
}

#[test]
fn echo_u16s() -> anyhow::Result<()> {
    proptest(&proptest::num::u16::ANY, async |value| {
        let mut store = store();
        let instance = pre().await.instantiate_async(&mut store).await?;
        assert_eq!(
            value,
            instance
                .componentize_js_tests_echoes()
                .call_echo_u16(&mut store, value)
                .await?
        );
        Ok(())
    })
}

#[test]
fn echo_s16s() -> anyhow::Result<()> {
    proptest(&proptest::num::i16::ANY, async |value| {
        let mut store = store();
        let instance = pre().await.instantiate_async(&mut store).await?;
        assert_eq!(
            value,
            instance
                .componentize_js_tests_echoes()
                .call_echo_s16(&mut store, value)
                .await?
        );
        Ok(())
    })
}

#[test]
fn echo_u32s() -> anyhow::Result<()> {
    proptest(&proptest::num::u32::ANY, async |value| {
        let mut store = store();
        let instance = pre().await.instantiate_async(&mut store).await?;
        assert_eq!(
            value,
            instance
                .componentize_js_tests_echoes()
                .call_echo_u32(&mut store, value)
                .await?
        );
        Ok(())
    })
}

#[test]
fn echo_s32s() -> anyhow::Result<()> {
    proptest(&proptest::num::i32::ANY, async |value| {
        let mut store = store();
        let instance = pre().await.instantiate_async(&mut store).await?;
        assert_eq!(
            value,
            instance
                .componentize_js_tests_echoes()
                .call_echo_s32(&mut store, value)
                .await?
        );
        Ok(())
    })
}

#[test]
fn echo_u64s() -> anyhow::Result<()> {
    proptest(&proptest::num::u64::ANY, async |value| {
        let mut store = store();
        let instance = pre().await.instantiate_async(&mut store).await?;
        assert_eq!(
            value,
            instance
                .componentize_js_tests_echoes()
                .call_echo_u64(&mut store, value)
                .await?
        );
        Ok(())
    })
}

#[test]
fn echo_s64s() -> anyhow::Result<()> {
    proptest(&proptest::num::i64::ANY, async |value| {
        let mut store = store();
        let instance = pre().await.instantiate_async(&mut store).await?;
        assert_eq!(
            value,
            instance
                .componentize_js_tests_echoes()
                .call_echo_s64(&mut store, value)
                .await?
        );
        Ok(())
    })
}

#[test]
fn echo_chars() -> anyhow::Result<()> {
    proptest(&proptest::char::any(), async |value| {
        let mut store = store();
        let instance = pre().await.instantiate_async(&mut store).await?;
        assert_eq!(
            value,
            instance
                .componentize_js_tests_echoes()
                .call_echo_char(&mut store, value)
                .await?
        );
        Ok(())
    })
}

#[derive(Debug, Copy, Clone)]
struct MyF32(f32);

impl PartialEq<MyF32> for MyF32 {
    fn eq(&self, other: &Self) -> bool {
        (self.0.is_nan() && other.0.is_nan()) || (self.0 == other.0)
    }
}

#[derive(Debug, Copy, Clone)]
struct MyF64(f64);

impl PartialEq<MyF64> for MyF64 {
    fn eq(&self, other: &Self) -> bool {
        (self.0.is_nan() && other.0.is_nan()) || (self.0 == other.0)
    }
}

#[test]
fn echo_f32s() -> anyhow::Result<()> {
    proptest(&proptest::num::f32::ANY.prop_map(MyF32), async |value| {
        let mut store = store();
        let instance = pre().await.instantiate_async(&mut store).await?;
        assert_eq!(
            value,
            MyF32(
                instance
                    .componentize_js_tests_echoes()
                    .call_echo_f32(&mut store, value.0)
                    .await?
            )
        );
        Ok(())
    })
}

#[test]
fn echo_f64s() -> anyhow::Result<()> {
    proptest(&proptest::num::f64::ANY.prop_map(MyF64), async |value| {
        let mut store = store();
        let instance = pre().await.instantiate_async(&mut store).await?;
        assert_eq!(
            value,
            MyF64(
                instance
                    .componentize_js_tests_echoes()
                    .call_echo_f64(&mut store, value.0)
                    .await?
            )
        );
        Ok(())
    })
}

#[test]
fn echo_strings() -> anyhow::Result<()> {
    proptest(&proptest::string::string_regex(".*")?, async |value| {
        let mut store = store();
        let instance = pre().await.instantiate_async(&mut store).await?;
        assert_eq!(
            value,
            instance
                .componentize_js_tests_echoes()
                .call_echo_string(&mut store, &value)
                .await?
        );
        Ok(())
    })
}

const MAX_SIZE: usize = 100;

#[test]
fn echo_lists_bool() -> anyhow::Result<()> {
    proptest(
        &proptest::collection::vec(proptest::bool::ANY, 0..MAX_SIZE),
        async |value| {
            let mut store = store();
            let instance = pre().await.instantiate_async(&mut store).await?;
            assert_eq!(
                value,
                instance
                    .componentize_js_tests_echoes()
                    .call_echo_list_bool(&mut store, &value)
                    .await?
            );
            Ok(())
        },
    )
}

#[test]
fn echo_lists_u8() -> anyhow::Result<()> {
    proptest(
        &proptest::collection::vec(proptest::num::u8::ANY, 0..MAX_SIZE),
        async |value| {
            let mut store = store();
            let instance = pre().await.instantiate_async(&mut store).await?;
            assert_eq!(
                value,
                instance
                    .componentize_js_tests_echoes()
                    .call_echo_list_u8(&mut store, &value)
                    .await?
            );
            Ok(())
        },
    )
}
