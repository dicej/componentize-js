#![deny(warnings)]

use {
    anyhow::{Context as _, anyhow},
    bytes::Bytes,
    indexmap::IndexSet,
    std::{
        borrow::Cow,
        collections::HashMap,
        io::Cursor,
        path::{Path, PathBuf},
    },
    wasm_encoder::{CustomSection, Section as _},
    wasmtime::{
        Config, Engine, Store,
        component::{Component, Linker, ResourceTable, ResourceType},
    },
    wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe},
    wasmtime_wasi::{DirPerms, FilePerms, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView},
    wasmtime_wizer::{WasmtimeWizerComponent, Wizer},
    wit_component::metadata,
    wit_dylib::DylibOpts,
    wit_parser::{
        FunctionKind, Resolve, TypeDefKind, UnresolvedPackageGroup, WorldId, WorldItem, WorldKey,
    },
};

wasmtime::component::bindgen!({
    path: "init.wit",
    world: "init",
    exports: { default: async },
});

mod codegen;
pub mod command;
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

pub enum Wit<'a, P = PathBuf> {
    String(&'a str),
    Paths(&'a [P]),
}

#[expect(clippy::type_complexity)]
pub async fn componentize(
    wit: Wit<'_, impl AsRef<Path>>,
    world: Option<&str>,
    features: &[String],
    all_features: bool,
    js: &str,
    js_base_directory: Option<impl AsRef<Path>>,
    add_to_linker: Option<&dyn Fn(&mut Linker<Ctx>) -> anyhow::Result<()>>,
) -> anyhow::Result<Vec<u8>> {
    let mut resolve = Resolve {
        all_features,
        ..Default::default()
    };

    for features in features {
        for feature in features
            .split(',')
            .flat_map(|s| s.split_whitespace())
            .filter(|f| !f.is_empty())
        {
            resolve.features.insert(feature.to_string());
        }
    }

    let package = match wit {
        Wit::String(wit) => resolve.push_str("wit", wit)?,
        Wit::Paths(paths) => {
            let mut last_pkg = None;
            for path in paths.iter().map(AsRef::as_ref) {
                let pkg = if path.is_dir() {
                    resolve.push_dir(path)?.0
                } else {
                    let pkg = UnresolvedPackageGroup::parse_file(path)?;
                    resolve.push_group(pkg)?
                };
                last_pkg = Some(pkg);
            }
            last_pkg.unwrap() // The paths should not be empty
        }
    };
    let world = resolve.select_world(&[package], world)?;

    let (mut bindings, metadata) = wit_dylib::create_with_metadata(
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

    let generated_code = codegen::generate(&metadata);
    let generated_script = &generated_code.script;
    let js = &format!("{js}\n{generated_script}");

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
    if let Some(dir) = js_base_directory {
        wasi.preopened_dir(dir, "/", DirPerms::all(), FilePerms::all())?;
    }
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
        add_wasi_and_stubs(&resolve, &[world].into_iter().collect(), &mut linker)?;
    }

    let instance = linker.instantiate_async(&mut store, &component).await?;
    {
        let instance = Init::new(&mut store, &instance)?;
        instance
            .call_init(
                &mut store,
                &generated_code.globals,
                &generated_code.modules,
                js,
            )
            .await
            .and_then(|v| v.map_err(|e| anyhow!("{e}")))
            .with_context(move || {
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&stdout.contents()),
                    String::from_utf8_lossy(&stderr.contents())
                )
            })?;
    }

    wizer
        .snapshot_component(
            cx,
            &mut WasmtimeWizerComponent {
                store: &mut store,
                instance,
            },
        )
        .await
}

// Stolen from https://github.com/bytecodealliance/componentize-py/blob/89af297898960efc48575d4c166d03b399568269/src/lib.rs#L761-L911
//
// TODO: deduplicate this so that `componentize-py` and this project can share
// it.
fn add_wasi_and_stubs(
    resolve: &Resolve,
    worlds: &IndexSet<WorldId>,
    linker: &mut Linker<Ctx>,
) -> anyhow::Result<()> {
    wasmtime_wasi::p2::add_to_linker_async(linker)?;

    enum Stub<'a> {
        Function(&'a String, &'a FunctionKind),
        Resource(&'a String),
    }

    let mut stubs = HashMap::<_, Vec<_>>::new();
    for &world in worlds {
        for (key, item) in &resolve.worlds[world].imports {
            match item {
                WorldItem::Interface { id, .. } => {
                    let interface_name = match key {
                        WorldKey::Name(name) => name.clone(),
                        WorldKey::Interface(interface) => resolve.id_of(*interface).unwrap(),
                    };

                    let interface = &resolve.interfaces[*id];
                    for (function_name, function) in &interface.functions {
                        stubs
                            .entry(Some(interface_name.clone()))
                            .or_default()
                            .push(Stub::Function(function_name, &function.kind));
                    }

                    for (type_name, id) in interface.types.iter() {
                        if let TypeDefKind::Resource = &resolve.types[*id].kind {
                            stubs
                                .entry(Some(interface_name.clone()))
                                .or_default()
                                .push(Stub::Resource(type_name));
                        }
                    }
                }
                WorldItem::Function(function) => {
                    stubs
                        .entry(None)
                        .or_default()
                        .push(Stub::Function(&function.name, &function.kind));
                }
                WorldItem::Type { id, .. } => {
                    let ty = &resolve.types[*id];
                    if let TypeDefKind::Resource = &ty.kind {
                        stubs
                            .entry(None)
                            .or_default()
                            .push(Stub::Resource(ty.name.as_ref().unwrap()));
                    }
                }
            }
        }
    }

    for (interface_name, stubs) in stubs {
        if let Some(interface_name) = interface_name {
            // Note that we do _not_ stub interfaces which appear to be part of
            // WASIp2 since those should be provided by the
            // `wasmtime_wasi::add_to_linker_async` call above, and adding stubs
            // to those same interfaces would just cause trouble.
            if !is_wasip2_cli(&interface_name)
                && let Ok(mut instance) = linker.instance(&interface_name)
            {
                for stub in stubs {
                    let interface_name = interface_name.clone();
                    match stub {
                        Stub::Function(name, kind) => {
                            if kind.is_async() {
                                instance.func_new_concurrent(name, {
                                    let name = name.clone();
                                    move |_, _, _, _| {
                                        let interface_name = interface_name.clone();
                                        let name = name.clone();
                                        Box::pin(async move {
                                            Err(anyhow!(
                                                "called trapping stub: {interface_name}#{name}"
                                            ))
                                        })
                                    }
                                })
                            } else {
                                instance.func_new(name, {
                                    let name = name.clone();
                                    move |_, _, _, _| {
                                        Err(anyhow!(
                                            "called trapping stub: {interface_name}#{name}"
                                        ))
                                    }
                                })
                            }
                        }
                        Stub::Resource(name) => instance
                            .resource(name, ResourceType::host::<()>(), {
                                let name = name.clone();
                                move |_, _| {
                                    Err(anyhow!("called trapping stub: {interface_name}#{name}"))
                                }
                            })
                            .map(drop),
                    }?;
                }
            }
        } else {
            let mut instance = linker.root();
            for stub in stubs {
                match stub {
                    Stub::Function(name, kind) => {
                        if kind.is_async() {
                            instance.func_new_concurrent(name, {
                                let name = name.clone();
                                move |_, _, _, _| {
                                    let name = name.clone();
                                    Box::pin(
                                        async move { Err(anyhow!("called trapping stub: {name}")) },
                                    )
                                }
                            })
                        } else {
                            instance.func_new(name, {
                                let name = name.clone();
                                move |_, _, _, _| Err(anyhow!("called trapping stub: {name}"))
                            })
                        }
                    }
                    Stub::Resource(name) => instance
                        .resource(name, ResourceType::host::<()>(), {
                            let name = name.clone();
                            move |_, _| Err(anyhow!("called trapping stub: {name}"))
                        })
                        .map(drop),
                }?;
            }
        }
    }

    Ok(())
}

fn is_wasip2_cli(interface_name: &str) -> bool {
    (interface_name.starts_with("wasi:cli/")
        || interface_name.starts_with("wasi:clocks/")
        || interface_name.starts_with("wasi:random/")
        || interface_name.starts_with("wasi:io/")
        || interface_name.starts_with("wasi:filesystem/")
        || interface_name.starts_with("wasi:sockets/"))
        && interface_name.contains("@0.2.")
}
