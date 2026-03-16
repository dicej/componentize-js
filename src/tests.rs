use {
    crate::{Ctx, Wit},
    componentize_js::tests::echoes::{EnumType, FlagsType, RecordType, ResourceType, VariantType},
    exports::componentize_js::tests::streams_and_futures,
    futures::{FutureExt as _, TryStreamExt as _, stream::FuturesUnordered},
    proptest::{
        prelude::{Just, Strategy},
        test_runner::{self, TestRng, TestRunner},
    },
    rand::RngExt,
    std::{
        collections::BTreeMap,
        env, mem,
        ops::DerefMut,
        pin::Pin,
        sync::{Arc, LazyLock, Mutex},
        task::{self, Context, Poll},
    },
    tokio::{runtime::Runtime, sync::OnceCell},
    wasmtime::{
        Config, Engine, Store, StoreContextMut,
        component::{
            Accessor, Component, Destination, FutureConsumer, FutureProducer, FutureReader,
            HasSelf, Lift, Linker, Resource, ResourceTable, Source, StreamConsumer, StreamProducer,
            StreamReader, StreamResult, VecBuffer,
        },
    },
    wasmtime_wasi::{WasiCtxBuilder, WasiView as _},
};

wasmtime::component::bindgen!({
    path: "src/tests.wit",
    world: "tests",
    imports: { default: async | trappable },
    exports: { default: async | task_exit },
    additional_derives: [PartialEq, Eq],
    with: {
        "componentize-js:tests/host-thing-interface.host-thing": ThingString,
    },
});

pub struct ThingString(String);

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
                    Wit::<String>::String(include_str!("tests.wit")),
                    None,
                    &[],
                    false,
                    include_str!("tests.js"),
                    None::<String>,
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

impl TestsImports for Ctx {}

impl TestsImportsWithStore for HasSelf<Ctx> {
    async fn delay<T>(_: &Accessor<T, Self>) -> anyhow::Result<()> {
        delay_via_yield().await;
        Ok(())
    }
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

async fn delay_via_yield() {
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
}

impl componentize_js::tests::simple_async_import_and_export::Host for Ctx {}

impl componentize_js::tests::simple_async_import_and_export::HostWithStore for HasSelf<Ctx> {
    async fn foo<T>(_: &Accessor<T, Self>, v: u32) -> anyhow::Result<u32> {
        delay_via_yield().await;
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

impl componentize_js::tests::types::HostResourceType for Ctx {
    async fn drop(&mut self, v: Resource<ResourceType>) -> anyhow::Result<()> {
        _ = v;
        Ok(())
    }
}

impl componentize_js::tests::types::Host for Ctx {}

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

    async fn echo_result_u8_u8(&mut self, v: Result<u8, u8>) -> anyhow::Result<Result<u8, u8>> {
        Ok(v)
    }

    async fn echo_result_result_u8_u8_u8(
        &mut self,
        v: Result<Result<u8, u8>, u8>,
    ) -> anyhow::Result<Result<Result<u8, u8>, u8>> {
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

    async fn echo_resource(
        &mut self,
        v: Resource<ResourceType>,
    ) -> anyhow::Result<Resource<ResourceType>> {
        Ok(v)
    }

    async fn accept_borrow(&mut self, v: Resource<ResourceType>) -> anyhow::Result<()> {
        _ = v;
        Ok(())
    }

    async fn echo_record(&mut self, v: RecordType) -> anyhow::Result<RecordType> {
        Ok(v)
    }

    async fn echo_enum(&mut self, v: EnumType) -> anyhow::Result<EnumType> {
        Ok(v)
    }

    async fn echo_flags(&mut self, v: FlagsType) -> anyhow::Result<FlagsType> {
        Ok(v)
    }

    async fn echo_variant(&mut self, v: VariantType) -> anyhow::Result<VariantType> {
        Ok(v)
    }

    async fn echo_stream(&mut self, v: StreamReader<u8>) -> anyhow::Result<StreamReader<u8>> {
        Ok(v)
    }

    async fn echo_future(&mut self, v: FutureReader<u8>) -> anyhow::Result<FutureReader<u8>> {
        Ok(v)
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

#[tokio::test]
async fn echo_nothing() -> anyhow::Result<()> {
    let mut store = store();
    let instance = pre().await.instantiate_async(&mut store).await?;
    instance
        .componentize_js_tests_echoes()
        .call_echo_nothing(&mut store)
        .await
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

#[derive(Debug, Copy, Clone)]
struct MyF64(f64);

impl PartialEq<MyF64> for MyF64 {
    fn eq(&self, other: &Self) -> bool {
        (self.0.is_nan() && other.0.is_nan()) || (self.0 == other.0)
    }
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

#[test]
fn echo_lists_list_u8() -> anyhow::Result<()> {
    proptest(
        &proptest::collection::vec(
            proptest::collection::vec(proptest::num::u8::ANY, 0..MAX_SIZE / 2),
            0..MAX_SIZE,
        ),
        async |value| {
            let mut store = store();
            let instance = pre().await.instantiate_async(&mut store).await?;
            assert_eq!(
                value,
                instance
                    .componentize_js_tests_echoes()
                    .call_echo_list_list_u8(&mut store, &value)
                    .await?
            );
            Ok(())
        },
    )
}

#[test]
fn echo_lists_list_list_u8() -> anyhow::Result<()> {
    proptest(
        &proptest::collection::vec(
            proptest::collection::vec(
                proptest::collection::vec(proptest::num::u8::ANY, 0..MAX_SIZE / 4),
                0..MAX_SIZE / 2,
            ),
            0..MAX_SIZE,
        ),
        async |value| {
            let mut store = store();
            let instance = pre().await.instantiate_async(&mut store).await?;
            assert_eq!(
                value,
                instance
                    .componentize_js_tests_echoes()
                    .call_echo_list_list_list_u8(&mut store, &value)
                    .await?
            );
            Ok(())
        },
    )
}

#[test]
fn echo_options_u8() -> anyhow::Result<()> {
    proptest(
        &proptest::option::of(proptest::num::u8::ANY),
        async |value| {
            let mut store = store();
            let instance = pre().await.instantiate_async(&mut store).await?;
            assert_eq!(
                value,
                instance
                    .componentize_js_tests_echoes()
                    .call_echo_option_u8(&mut store, value)
                    .await?
            );
            Ok(())
        },
    )
}

#[test]
fn echo_options_option_u8() -> anyhow::Result<()> {
    proptest(
        &proptest::option::of(proptest::option::of(proptest::num::u8::ANY)),
        async |value| {
            let mut store = store();
            let instance = pre().await.instantiate_async(&mut store).await?;
            assert_eq!(
                value,
                instance
                    .componentize_js_tests_echoes()
                    .call_echo_option_option_u8(&mut store, value)
                    .await?
            );
            Ok(())
        },
    )
}

#[test]
fn echo_results_u8_u8() -> anyhow::Result<()> {
    proptest(
        &proptest::result::maybe_ok(proptest::num::u8::ANY, proptest::num::u8::ANY),
        async |value| {
            let mut store = store();
            let instance = pre().await.instantiate_async(&mut store).await?;
            assert_eq!(
                value,
                instance
                    .componentize_js_tests_echoes()
                    .call_echo_result_u8_u8(&mut store, value)
                    .await?
            );
            Ok(())
        },
    )
}

#[test]
fn echo_results_result_u8_u8_u8() -> anyhow::Result<()> {
    proptest(
        &proptest::result::maybe_ok(
            proptest::result::maybe_ok(proptest::num::u8::ANY, proptest::num::u8::ANY),
            proptest::num::u8::ANY,
        ),
        async |value| {
            let mut store = store();
            let instance = pre().await.instantiate_async(&mut store).await?;
            assert_eq!(
                value,
                instance
                    .componentize_js_tests_echoes()
                    .call_echo_result_result_u8_u8_u8(&mut store, value)
                    .await?
            );
            Ok(())
        },
    )
}

#[test]
fn echo_lists_s8() -> anyhow::Result<()> {
    proptest(
        &proptest::collection::vec(proptest::num::i8::ANY, 0..MAX_SIZE),
        async |value| {
            let mut store = store();
            let instance = pre().await.instantiate_async(&mut store).await?;
            assert_eq!(
                value,
                instance
                    .componentize_js_tests_echoes()
                    .call_echo_list_s8(&mut store, &value)
                    .await?
            );
            Ok(())
        },
    )
}

#[test]
fn echo_lists_u16() -> anyhow::Result<()> {
    proptest(
        &proptest::collection::vec(proptest::num::u16::ANY, 0..MAX_SIZE),
        async |value| {
            let mut store = store();
            let instance = pre().await.instantiate_async(&mut store).await?;
            assert_eq!(
                value,
                instance
                    .componentize_js_tests_echoes()
                    .call_echo_list_u16(&mut store, &value)
                    .await?
            );
            Ok(())
        },
    )
}

#[test]
fn echo_lists_s16() -> anyhow::Result<()> {
    proptest(
        &proptest::collection::vec(proptest::num::i16::ANY, 0..MAX_SIZE),
        async |value| {
            let mut store = store();
            let instance = pre().await.instantiate_async(&mut store).await?;
            assert_eq!(
                value,
                instance
                    .componentize_js_tests_echoes()
                    .call_echo_list_s16(&mut store, &value)
                    .await?
            );
            Ok(())
        },
    )
}

#[test]
fn echo_lists_u32() -> anyhow::Result<()> {
    proptest(
        &proptest::collection::vec(proptest::num::u32::ANY, 0..MAX_SIZE),
        async |value| {
            let mut store = store();
            let instance = pre().await.instantiate_async(&mut store).await?;
            assert_eq!(
                value,
                instance
                    .componentize_js_tests_echoes()
                    .call_echo_list_u32(&mut store, &value)
                    .await?
            );
            Ok(())
        },
    )
}

#[test]
fn echo_lists_s32() -> anyhow::Result<()> {
    proptest(
        &proptest::collection::vec(proptest::num::i32::ANY, 0..MAX_SIZE),
        async |value| {
            let mut store = store();
            let instance = pre().await.instantiate_async(&mut store).await?;
            assert_eq!(
                value,
                instance
                    .componentize_js_tests_echoes()
                    .call_echo_list_s32(&mut store, &value)
                    .await?
            );
            Ok(())
        },
    )
}

#[test]
fn echo_lists_u64() -> anyhow::Result<()> {
    proptest(
        &proptest::collection::vec(proptest::num::u64::ANY, 0..MAX_SIZE),
        async |value| {
            let mut store = store();
            let instance = pre().await.instantiate_async(&mut store).await?;
            assert_eq!(
                value,
                instance
                    .componentize_js_tests_echoes()
                    .call_echo_list_u64(&mut store, &value)
                    .await?
            );
            Ok(())
        },
    )
}

#[test]
fn echo_lists_s64() -> anyhow::Result<()> {
    proptest(
        &proptest::collection::vec(proptest::num::i64::ANY, 0..MAX_SIZE),
        async |value| {
            let mut store = store();
            let instance = pre().await.instantiate_async(&mut store).await?;
            assert_eq!(
                value,
                instance
                    .componentize_js_tests_echoes()
                    .call_echo_list_s64(&mut store, &value)
                    .await?
            );
            Ok(())
        },
    )
}

#[test]
fn echo_lists_char() -> anyhow::Result<()> {
    proptest(
        &proptest::collection::vec(proptest::char::any(), 0..MAX_SIZE),
        async |value| {
            let mut store = store();
            let instance = pre().await.instantiate_async(&mut store).await?;
            assert_eq!(
                value,
                instance
                    .componentize_js_tests_echoes()
                    .call_echo_list_char(&mut store, &value)
                    .await?
            );
            Ok(())
        },
    )
}

#[test]
fn echo_lists_f32() -> anyhow::Result<()> {
    proptest(
        &proptest::collection::vec(proptest::num::f32::ANY.prop_map(MyF32), 0..MAX_SIZE),
        async |value| {
            let mut store = store();
            let instance = pre().await.instantiate_async(&mut store).await?;
            assert_eq!(
                value,
                instance
                    .componentize_js_tests_echoes()
                    .call_echo_list_f32(&mut store, &value.iter().map(|v| v.0).collect::<Vec<_>>())
                    .await?
                    .into_iter()
                    .map(MyF32)
                    .collect::<Vec<_>>()
            );
            Ok(())
        },
    )
}

#[test]
fn echo_lists_f64() -> anyhow::Result<()> {
    proptest(
        &proptest::collection::vec(proptest::num::f64::ANY.prop_map(MyF64), 0..MAX_SIZE),
        async |value| {
            let mut store = store();
            let instance = pre().await.instantiate_async(&mut store).await?;
            assert_eq!(
                value,
                instance
                    .componentize_js_tests_echoes()
                    .call_echo_list_f64(&mut store, &value.iter().map(|v| v.0).collect::<Vec<_>>())
                    .await?
                    .into_iter()
                    .map(MyF64)
                    .collect::<Vec<_>>()
            );
            Ok(())
        },
    )
}

#[test]
fn echo_many() -> anyhow::Result<()> {
    proptest(
        &(
            (
                proptest::bool::ANY,
                proptest::num::u8::ANY,
                proptest::num::u16::ANY,
                proptest::num::u32::ANY,
                proptest::num::u64::ANY,
                proptest::num::i8::ANY,
                proptest::num::i16::ANY,
                proptest::num::i32::ANY,
            ),
            (
                proptest::num::i64::ANY,
                proptest::num::f32::ANY.prop_map(MyF32),
                proptest::num::f64::ANY.prop_map(MyF64),
                proptest::char::any(),
                proptest::string::string_regex(".*")?,
                proptest::collection::vec(proptest::bool::ANY, 0..MAX_SIZE),
                proptest::collection::vec(proptest::num::u8::ANY, 0..MAX_SIZE),
                proptest::collection::vec(proptest::num::u16::ANY, 0..MAX_SIZE),
            ),
        ),
        async |((v1, v2, v3, v4, v5, v6, v7, v8), (v9, v10, v11, v12, v13, v14, v15, v16))| {
            let mut store = store();
            let instance = pre().await.instantiate_async(&mut store).await?;
            let (r1, r2, r3, r4, r5, r6, r7, r8, r9, r10, r11, r12, r13, r14, r15, r16) = instance
                .componentize_js_tests_echoes()
                .call_echo_many(
                    &mut store, v1, v2, v3, v4, v5, v6, v7, v8, v9, v10.0, v11.0, v12, &v13, &v14,
                    &v15, &v16,
                )
                .await?;
            assert_eq!(
                (
                    (v1, v2, v3, v4, v5, v6, v7, v8),
                    (v9, v10, v11, v12, v13, v14, v15, v16)
                ),
                (
                    (r1, r2, r3, r4, r5, r6, r7, r8),
                    (r9, MyF32(r10), MyF64(r11), r12, r13, r14, r15, r16)
                ),
            );
            Ok(())
        },
    )
}

#[tokio::test]
async fn echo_resource() -> anyhow::Result<()> {
    let mut store = store();
    let instance = pre().await.instantiate_async(&mut store).await?;
    assert_eq!(
        42,
        instance
            .componentize_js_tests_echoes()
            .call_echo_resource(&mut store, Resource::new_own(42))
            .await?
            .rep()
    );
    Ok(())
}

#[tokio::test]
async fn accept_borrow() -> anyhow::Result<()> {
    let mut store = store();
    let instance = pre().await.instantiate_async(&mut store).await?;
    instance
        .componentize_js_tests_echoes()
        .call_accept_borrow(&mut store, Resource::new_borrow(42))
        .await?;
    Ok(())
}

#[test]
fn echo_records() -> anyhow::Result<()> {
    proptest(
        &(
            proptest::num::u32::ANY,
            proptest::string::string_regex(".*")?,
            proptest::bool::ANY.prop_flat_map(|v| {
                if v {
                    proptest::num::u32::ANY.prop_map(Ok).boxed()
                } else {
                    proptest::num::u64::ANY.prop_map(Err).boxed()
                }
            }),
        )
            .prop_map(|(a, b, c)| RecordType { a, b, c }),
        async |v| {
            let mut store = store();
            let instance = pre().await.instantiate_async(&mut store).await?;
            assert_eq!(
                v,
                instance
                    .componentize_js_tests_echoes()
                    .call_echo_record(&mut store, &v,)
                    .await?
            );
            Ok(())
        },
    )
}

#[test]
fn echo_enums() -> anyhow::Result<()> {
    proptest(
        &(0..3).prop_map(|v| match v {
            0 => EnumType::A,
            1 => EnumType::B,
            2 => EnumType::C,
            _ => unreachable!(),
        }),
        async |v| {
            let mut store = store();
            let instance = pre().await.instantiate_async(&mut store).await?;
            assert_eq!(
                v,
                instance
                    .componentize_js_tests_echoes()
                    .call_echo_enum(&mut store, v)
                    .await?
            );
            Ok(())
        },
    )
}

#[test]
fn echo_flags() -> anyhow::Result<()> {
    proptest(
        &(
            proptest::bool::ANY,
            proptest::bool::ANY,
            proptest::bool::ANY,
        )
            .prop_map(|(a, b, c)| {
                let mut flags = FlagsType::default();
                if a {
                    flags |= FlagsType::A;
                }
                if b {
                    flags |= FlagsType::B;
                }
                if c {
                    flags |= FlagsType::C;
                }
                flags
            }),
        async |v| {
            let mut store = store();
            let instance = pre().await.instantiate_async(&mut store).await?;
            assert_eq!(
                v,
                instance
                    .componentize_js_tests_echoes()
                    .call_echo_flags(&mut store, v)
                    .await?
            );
            Ok(())
        },
    )
}

#[test]
fn echo_variants() -> anyhow::Result<()> {
    proptest(
        &(0..5).prop_flat_map(|v| match v {
            0 => proptest::num::u32::ANY.prop_map(VariantType::A).boxed(),
            1 => proptest::string::string_regex(".*")
                .unwrap()
                .prop_map(VariantType::B)
                .boxed(),
            2 => proptest::num::u32::ANY
                .prop_map(|v| VariantType::C(Ok(v)))
                .boxed(),
            3 => proptest::num::u64::ANY
                .prop_map(|v| VariantType::C(Err(v)))
                .boxed(),
            4 => Just(VariantType::D).boxed(),
            _ => unreachable!(),
        }),
        async |v| {
            let mut store = store();
            let instance = pre().await.instantiate_async(&mut store).await?;
            assert_eq!(
                v,
                instance
                    .componentize_js_tests_echoes()
                    .call_echo_variant(&mut store, &v)
                    .await?
            );
            Ok(())
        },
    )
}

#[tokio::test]
async fn echo_stream() -> anyhow::Result<()> {
    let mut store = store();
    let instance = pre().await.instantiate_async(&mut store).await?;
    let stream = StreamReader::new(&mut store, vec![42]);
    // TODO: read from returned stream and assert content matches what was
    // produced.
    instance
        .componentize_js_tests_echoes()
        .call_echo_stream(&mut store, stream)
        .await?;
    Ok(())
}

#[tokio::test]
async fn echo_future() -> anyhow::Result<()> {
    let mut store = store();
    let instance = pre().await.instantiate_async(&mut store).await?;
    let future = FutureReader::new(&mut store, async { anyhow::Ok(42) });
    // TODO: read from returned future and assert content matches what was
    // produced.
    instance
        .componentize_js_tests_echoes()
        .call_echo_future(&mut store, future)
        .await?;
    Ok(())
}

struct VecProducer<T> {
    source: Vec<T>,
    sleep: Pin<Box<dyn Future<Output = ()> + Send>>,
}

impl<T> VecProducer<T> {
    fn new(source: Vec<T>, delay: bool) -> Self {
        Self {
            source,
            sleep: if delay {
                delay_via_yield().boxed()
            } else {
                async {}.boxed()
            },
        }
    }
}

impl<D, T: Lift + Unpin + 'static> StreamProducer<D> for VecProducer<T> {
    type Item = T;
    type Buffer = VecBuffer<T>;

    fn poll_produce(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        _: StoreContextMut<D>,
        mut destination: Destination<Self::Item, Self::Buffer>,
        _: bool,
    ) -> Poll<anyhow::Result<StreamResult>> {
        let sleep = &mut self.as_mut().get_mut().sleep;
        task::ready!(sleep.as_mut().poll(cx));
        *sleep = async {}.boxed();

        destination.set_buffer(mem::take(&mut self.get_mut().source).into());
        Poll::Ready(Ok(StreamResult::Dropped))
    }
}

struct VecConsumer<T> {
    destination: Arc<Mutex<Vec<T>>>,
    sleep: Pin<Box<dyn Future<Output = ()> + Send>>,
}

impl<T> VecConsumer<T> {
    fn new(destination: Arc<Mutex<Vec<T>>>, delay: bool) -> Self {
        Self {
            destination,
            sleep: if delay {
                delay_via_yield().boxed()
            } else {
                async {}.boxed()
            },
        }
    }
}

impl<D, T: Lift + 'static> StreamConsumer<D> for VecConsumer<T> {
    type Item = T;

    fn poll_consume(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        store: StoreContextMut<D>,
        mut source: Source<Self::Item>,
        _: bool,
    ) -> Poll<anyhow::Result<StreamResult>> {
        let sleep = &mut self.as_mut().get_mut().sleep;
        task::ready!(sleep.as_mut().poll(cx));
        *sleep = async {}.boxed();

        source.read(store, self.destination.lock().unwrap().deref_mut())?;
        Poll::Ready(Ok(StreamResult::Completed))
    }
}

#[tokio::test]
async fn echo_stream_u8() -> anyhow::Result<()> {
    test_echo_stream_u8(false).await
}

#[tokio::test]
async fn echo_stream_u8_with_delay() -> anyhow::Result<()> {
    test_echo_stream_u8(true).await
}

async fn test_echo_stream_u8(delay: bool) -> anyhow::Result<()> {
    let mut store = store();
    let instance = pre().await.instantiate_async(&mut store).await?;
    store
        .run_concurrent(async |store| {
            let expected = b"Beware the Jubjub bird, and shun\n\tThe frumious Bandersnatch!";
            let stream = store
                .with(|store| StreamReader::new(store, VecProducer::new(expected.to_vec(), delay)));

            let (stream, task) = instance
                .componentize_js_tests_streams_and_futures()
                .call_echo_stream_u8(store, stream)
                .await?;

            let received = Arc::new(Mutex::new(Vec::with_capacity(expected.len())));
            store.with(|store| stream.pipe(store, VecConsumer::new(received.clone(), delay)));

            task.block(store).await;

            assert_eq!(expected, &received.lock().unwrap()[..]);

            anyhow::Ok(())
        })
        .await??;

    Ok(())
}

struct OptionProducer<T> {
    source: Option<T>,
    sleep: Pin<Box<dyn Future<Output = ()> + Send>>,
}

impl<T> OptionProducer<T> {
    fn new(source: Option<T>, delay: bool) -> Self {
        Self {
            source,
            sleep: if delay {
                delay_via_yield().boxed()
            } else {
                async {}.boxed()
            },
        }
    }
}

impl<D, T: Unpin + Send + 'static> FutureProducer<D> for OptionProducer<T> {
    type Item = T;

    fn poll_produce(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        _: StoreContextMut<D>,
        _: bool,
    ) -> Poll<anyhow::Result<Option<T>>> {
        let sleep = &mut self.as_mut().get_mut().sleep;
        task::ready!(sleep.as_mut().poll(cx));
        *sleep = async {}.boxed();

        Poll::Ready(Ok(self.get_mut().source.take()))
    }
}

struct OptionConsumer<T> {
    destination: Arc<Mutex<Option<T>>>,
    sleep: Pin<Box<dyn Future<Output = ()> + Send>>,
}

impl<T> OptionConsumer<T> {
    fn new(destination: Arc<Mutex<Option<T>>>, delay: bool) -> Self {
        Self {
            destination,
            sleep: if delay {
                delay_via_yield().boxed()
            } else {
                async {}.boxed()
            },
        }
    }
}

impl<D, T: Lift + 'static> FutureConsumer<D> for OptionConsumer<T> {
    type Item = T;

    fn poll_consume(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        store: StoreContextMut<D>,
        mut source: Source<Self::Item>,
        _: bool,
    ) -> Poll<anyhow::Result<()>> {
        let sleep = &mut self.as_mut().get_mut().sleep;
        task::ready!(sleep.as_mut().poll(cx));
        *sleep = async {}.boxed();

        source.read(store, self.destination.lock().unwrap().deref_mut())?;
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn echo_future_string() -> anyhow::Result<()> {
    test_echo_future_string(false).await
}

#[tokio::test]
async fn echo_future_string_with_delay() -> anyhow::Result<()> {
    test_echo_future_string(true).await
}

async fn test_echo_future_string(delay: bool) -> anyhow::Result<()> {
    let mut store = store();
    let instance = pre().await.instantiate_async(&mut store).await?;
    store
        .run_concurrent(async |store| {
            let expected = "Beware the Jubjub bird, and shun\n\tThe frumious Bandersnatch!";
            let future = store.with(|store| {
                FutureReader::new(
                    store,
                    OptionProducer::new(Some(expected.to_string()), delay),
                )
            });

            let (future, task) = instance
                .componentize_js_tests_streams_and_futures()
                .call_echo_future_string(store, future)
                .await?;

            let received = Arc::new(Mutex::new(None::<String>));
            store.with(|store| future.pipe(store, OptionConsumer::new(received.clone(), delay)));

            task.block(store).await;

            assert_eq!(
                expected,
                received.lock().unwrap().as_ref().unwrap().as_str()
            );

            anyhow::Ok(())
        })
        .await??;

    Ok(())
}

struct OneAtATime<T> {
    destination: Arc<Mutex<Vec<T>>>,
    sleep: Pin<Box<dyn Future<Output = ()> + Send>>,
    delay: bool,
}

impl<T> OneAtATime<T> {
    fn new(destination: Arc<Mutex<Vec<T>>>, delay: bool) -> Self {
        Self {
            destination,
            sleep: if delay {
                delay_via_yield().boxed()
            } else {
                async {}.boxed()
            },
            delay,
        }
    }
}

impl<D, T: Lift + 'static> StreamConsumer<D> for OneAtATime<T> {
    type Item = T;

    fn poll_consume(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        store: StoreContextMut<D>,
        mut source: Source<Self::Item>,
        _: bool,
    ) -> Poll<anyhow::Result<StreamResult>> {
        let delay = self.delay;
        let sleep = &mut self.as_mut().get_mut().sleep;
        task::ready!(sleep.as_mut().poll(cx));
        *sleep = if delay {
            delay_via_yield().boxed()
        } else {
            async {}.boxed()
        };

        let value = &mut None;
        source.read(store, value)?;
        self.destination.lock().unwrap().push(value.take().unwrap());
        Poll::Ready(Ok(StreamResult::Completed))
    }
}

#[tokio::test]
async fn short_reads() -> anyhow::Result<()> {
    test_short_reads(false).await
}

#[tokio::test]
async fn short_reads_with_delay() -> anyhow::Result<()> {
    test_short_reads(true).await
}

async fn test_short_reads(delay: bool) -> anyhow::Result<()> {
    let mut store = store();
    let instance = pre().await.instantiate_async(&mut store).await?;
    let instance = instance.componentize_js_tests_streams_and_futures();
    let thing = instance.thing();

    let strings = ["a", "b", "c", "d", "e"];
    let mut things = Vec::with_capacity(strings.len());
    for string in strings {
        things.push(thing.call_constructor(&mut store, string).await?);
    }

    let received_things = store
        .run_concurrent(async |store| {
            let count = things.len();
            // Write the items all at once.  The receiver will only read them
            // one at a time, forcing us to retake ownership of the unwritten
            // items between writes.
            let stream =
                store.with(|store| StreamReader::new(store, VecProducer::new(things, delay)));

            let (stream, task) = instance.call_short_reads(store, stream).await?;

            let received_things = Arc::new(Mutex::new(
                Vec::<streams_and_futures::Thing>::with_capacity(count),
            ));
            // Read only one item at a time, forcing the sender to retake
            // ownership of any unwritten items.
            store.with(|store| stream.pipe(store, OneAtATime::new(received_things.clone(), delay)));

            task.block(store).await;

            assert_eq!(count, received_things.lock().unwrap().len());

            let received_things = mem::take(received_things.lock().unwrap().deref_mut());

            // Dispatch the `thing.get` calls concurrently to test that
            // the runtime release borrows in async-lifted exports
            // correctly.
            let mut futures = FuturesUnordered::new();
            for (index, &it) in received_things.iter().enumerate() {
                futures.push(
                    thing
                        .call_get(store, it, delay)
                        .map(move |v| v.map(move |v| (index, v))),
                );
            }

            let mut received_strings = BTreeMap::new();
            while let Some((index, (string, _))) = futures.try_next().await? {
                received_strings.insert(index, string);
            }

            assert_eq!(
                &strings[..],
                &received_strings
                    .values()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
            );

            // Do the above again, but this time using `thing.get-static`.
            let mut futures = FuturesUnordered::new();
            for (index, &it) in received_things.iter().enumerate() {
                futures.push(
                    thing
                        .call_get_static(store, it, delay)
                        .map(move |v| v.map(move |v| (index, v))),
                );
            }

            let mut received_strings = BTreeMap::new();
            while let Some((index, (string, _))) = futures.try_next().await? {
                received_strings.insert(index, string);
            }

            assert_eq!(
                &strings[..],
                &received_strings
                    .values()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
            );

            anyhow::Ok(received_things)
        })
        .await??;

    for it in received_things {
        it.resource_drop_async(&mut store).await?;
    }

    Ok(())
}

impl componentize_js::tests::host_thing_interface::HostHostThingWithStore for HasSelf<Ctx> {
    async fn get<T>(
        accessor: &Accessor<T, Self>,
        this: Resource<ThingString>,
    ) -> anyhow::Result<String> {
        accessor.with(|mut store| Ok(store.get().table.get(&this)?.0.clone()))
    }

    async fn get_static<T>(
        accessor: &Accessor<T, Self>,
        this: Resource<ThingString>,
    ) -> anyhow::Result<String> {
        accessor.with(|mut store| Ok(store.get().table.get(&this)?.0.clone()))
    }
}

impl componentize_js::tests::host_thing_interface::HostHostThing for Ctx {
    async fn new(&mut self, v: String) -> anyhow::Result<Resource<ThingString>> {
        Ok(self.ctx().table.push(ThingString(v))?)
    }

    async fn drop(&mut self, this: Resource<ThingString>) -> anyhow::Result<()> {
        Ok(self.ctx().table.delete(this).map(|_| ())?)
    }
}

impl componentize_js::tests::host_thing_interface::Host for Ctx {}

#[tokio::test]
async fn short_reads_host() -> anyhow::Result<()> {
    test_short_reads_host(false).await
}

#[tokio::test]
async fn short_reads_host_with_delay() -> anyhow::Result<()> {
    test_short_reads_host(true).await
}

async fn test_short_reads_host(delay: bool) -> anyhow::Result<()> {
    let mut store = store();
    let instance = pre().await.instantiate_async(&mut store).await?;
    let instance = instance.componentize_js_tests_streams_and_futures();

    let strings = ["a", "b", "c", "d", "e"];
    let mut things = Vec::with_capacity(strings.len());
    for string in strings {
        things.push(store.data_mut().table.push(ThingString(string.into()))?);
    }

    store
        .run_concurrent(async |store| {
            let count = things.len();
            // Write the items all at once.  The receiver will only read them
            // one at a time, forcing us to retake ownership of the unwritten
            // items between writes.
            let stream =
                store.with(|store| StreamReader::new(store, VecProducer::new(things, delay)));

            let (stream, task) = instance.call_short_reads_host(store, stream).await?;

            let received_things = Arc::new(Mutex::new(
                Vec::<Resource<ThingString>>::with_capacity(count),
            ));
            // Read only one item at a time, forcing the sender to retake
            // ownership of any unwritten items.
            store.with(|store| stream.pipe(store, OneAtATime::new(received_things.clone(), delay)));

            task.block(store).await;

            assert_eq!(count, received_things.lock().unwrap().len());

            let received_strings = store.with(|mut store| {
                mem::take(received_things.lock().unwrap().deref_mut())
                    .into_iter()
                    .map(|v| Ok(store.get().table.delete(v)?.0))
                    .collect::<anyhow::Result<Vec<_>>>()
            })?;

            assert_eq!(
                &strings[..],
                &received_strings
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
            );

            anyhow::Ok(())
        })
        .await??;

    Ok(())
}

#[tokio::test]
async fn dropped_future_reader() -> anyhow::Result<()> {
    test_dropped_future_reader(false).await
}

#[tokio::test]
async fn dropped_future_reader_with_delay() -> anyhow::Result<()> {
    test_dropped_future_reader(true).await
}

async fn test_dropped_future_reader(delay: bool) -> anyhow::Result<()> {
    let mut store = store();
    let instance = pre().await.instantiate_async(&mut store).await?;
    let instance = instance.componentize_js_tests_streams_and_futures();
    let thing = instance.thing();

    let it = store
        .run_concurrent(async |store| {
            let expected = "Beware the Jubjub bird, and shun\n\tThe frumious Bandersnatch!";
            let ((mut rx1, rx2), task) = instance
                .call_dropped_future_reader(store, expected.into())
                .await?;
            // Close the future without reading the value.  This will
            // force the sender to retake ownership of the value it
            // tried to write.
            rx1.close_with(store);

            let received = Arc::new(Mutex::new(None::<streams_and_futures::Thing>));
            store.with(|store| rx2.pipe(store, OptionConsumer::new(received.clone(), delay)));

            task.block(store).await;

            let it = received.lock().unwrap().take().unwrap();

            assert_eq!(expected, &thing.call_get(store, it, false).await?.0);

            anyhow::Ok(it)
        })
        .await??;

    it.resource_drop_async(&mut store).await?;

    Ok(())
}

#[tokio::test]
async fn dropped_future_reader_host() -> anyhow::Result<()> {
    test_dropped_future_reader_host(false).await
}

#[tokio::test]
async fn dropped_future_reader_host_with_delay() -> anyhow::Result<()> {
    test_dropped_future_reader_host(true).await
}

async fn test_dropped_future_reader_host(delay: bool) -> anyhow::Result<()> {
    let mut store = store();
    let instance = pre().await.instantiate_async(&mut store).await?;
    let instance = instance.componentize_js_tests_streams_and_futures();

    store
        .run_concurrent(async |store| {
            let expected = "Beware the Jubjub bird, and shun\n\tThe frumious Bandersnatch!";
            let ((mut rx1, rx2), task) = instance
                .call_dropped_future_reader_host(store, expected.into())
                .await?;
            // Close the future without reading the value.  This will
            // force the sender to retake ownership of the value it
            // tried to write.
            rx1.close_with(store);

            let received = Arc::new(Mutex::new(None::<Resource<ThingString>>));
            store.with(|store| rx2.pipe(store, OptionConsumer::new(received.clone(), delay)));

            task.block(store).await;

            let it = store.with(|mut store| {
                anyhow::Ok(
                    store
                        .get()
                        .table
                        .delete(received.lock().unwrap().take().unwrap())?
                        .0,
                )
            })?;

            assert_eq!(expected, &it);

            anyhow::Ok(it)
        })
        .await??;

    Ok(())
}
