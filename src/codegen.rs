use {
    heck::ToLowerCamelCase as _,
    std::collections::BTreeMap,
    wit_dylib::metadata::{Metadata, Type},
};

pub struct GeneratedCode {
    pub globals: String,
    pub modules: Vec<(String, String)>,
    pub script: String,
}

pub fn generate(metadata: &Metadata) -> GeneratedCode {
    let mut modules = Vec::new();
    let mut world_module = String::new();

    // First, generate JS functions for any and all imported functions, grouping
    // them by interface and emitting one ES module per interface, plus another
    // for world-level imports, if applicable.  Each function will forward its
    // parameters to `_componentizeJsCallImport`, which will be provided by
    // the runtime to call the imported function itself.
    let mut imports = BTreeMap::<_, Vec<_>>::new();
    for (index, func) in metadata.import_funcs.iter().enumerate() {
        imports
            .entry(&func.interface)
            .or_default()
            .push((index, func));
    }

    for (interface, funcs) in imports {
        let funcs = funcs
            .into_iter()
            .map(|(index, func)| {
                let name = mangle_name(&func.name);
                let params = (0..func.args.len())
                    .map(|i| format!("p{i}"))
                    .collect::<Vec<_>>()
                    .join(",");
                let value = if func.async_import_elem_index.is_some() {
                    format!(
                        "new Promise((a,b)=>\
                             _componentizeJsCallImport({index},[{params}],a,b))"
                    )
                } else {
                    format!("_componentizeJsCallImport({index},[{params}])")
                };
                format!("export function {name}({params}){{return {value}}}\n")
            })
            .collect::<Vec<_>>()
            .concat();

        if let Some(interface) = interface {
            modules.push((interface.to_string(), funcs));
        } else {
            world_module.push_str(&funcs);
        };
    }

    // Next, generate wrapper functions for any and all async function exports
    // so that they call back into the runtime when the promises resolve.
    let mut async_exports = BTreeMap::<_, Vec<_>>::new();
    for (index, func) in metadata.export_funcs.iter().enumerate() {
        // TODO: As of this writing `wit-dylib`, won't tell us which functions
        // are async, so here we conservatively generate async wrappers for all
        // of them; the wrappers for the sync functions won't actually be used.
        // We _could_ consult the original `Resolve` for that information, but
        // it would probably be easier to modify `wit-dylib` to keep track of it
        // so it's available in `Metadata`.
        async_exports
            .entry(&func.interface)
            .or_default()
            .push((index, func));
    }

    let async_exports = async_exports
        .into_iter()
        .map(|(interface, funcs)| {
            let interface = interface.as_deref().map(mangle_name);
            let funcs = funcs
                .into_iter()
                .map(|(index, func)| {
                    let interface = interface
                        .as_ref()
                        .map(|v| format!("{v}."))
                        .unwrap_or_else(String::new);
                    let name = mangle_name(&func.name);
                    let params = (0..func.args.len())
                        .map(|i| format!("p{i}"))
                        .collect::<Vec<_>>()
                        .join(",");
                    let comma = if func.args.is_empty() { "" } else { "," };
                    format!(
                        "{name}:function(t{comma}{params}){{\n\
                         return {interface}{name}({params})\n\
                         .then((v)=>_componentizeJsCallTaskReturn({index},v,t))}}"
                    )
                })
                .collect::<Vec<_>>()
                .join(",");

            if let Some(interface) = interface {
                format!("{interface}:{{{funcs}}}")
            } else {
                funcs
            }
        })
        .collect::<Vec<_>>()
        .join(",");

    // Next, generate constructors for any and all future and stream types.
    //
    // TODO: As of this writing, `wit-dylib` may generate multiple stream and/or
    // future types for a given payload type; that's a but which should be
    // fixed.  Meanwhile, we work around it here by deduplicating the
    // constructors.
    world_module.push_str(
        &metadata
            .streams
            .iter()
            .enumerate()
            .map(|(index, stream)| {
                let name = if let Some(ty) = stream.ty {
                    let payload = mangle_ty(metadata, ty).to_lower_camel_case();
                    format!("{payload}Stream")
                } else {
                    "unitStream".into()
                };
                let code = format!(
                    "export function {name}(){{return _componentizeJsMakeStream({index})}}\n"
                );
                (name, code)
            })
            .chain(metadata.futures.iter().enumerate().map(|(index, future)| {
                let name = if let Some(ty) = future.ty {
                    let payload = mangle_ty(metadata, ty).to_lower_camel_case();
                    format!("{payload}Future")
                } else {
                    "unitFuture".into()
                };
                let code = format!(
                    "export function {name}(){{return _componentizeJsMakeFuture({index})}}\n"
                );
                (name, code)
            }))
            .collect::<BTreeMap<_, _>>()
            .into_values()
            .collect::<Vec<_>>()
            .concat(),
    );

    // Next, generate a bit of JS utility code for use with streams.
    let write_all = "async function(buffer) {
  let total = 0
  while (buffer.length > 0 && !this.reader_dropped) {
    count = await this.write(buffer)
    buffer = buffer.slice(count)
    total += count
  }
  return total
}";

    modules.push(("wit-world".to_string(), world_module));

    // Finally, return the result:
    GeneratedCode {
        globals: format!("var _componentizeJsWriteAll = {write_all}\n"),
        modules,
        script: format!("export const _componentizeJsAsyncExports = {{{async_exports}}}"),
    }
}

fn mangle_name(name: &str) -> String {
    name.replace(['@', ':', '/', '-', '[', ']', '.'], "_")
        .to_lower_camel_case()
}

fn mangle_ty(metadata: &Metadata, ty: Type) -> String {
    // TODO: Ensure the returned name is always distinct for distinct types
    // (e.g. by incorporating interface version numbers and/or additional
    // mangling as needed).

    let full_name = |interface, name| {
        let interface = if let Some(name) = interface {
            let name = mangle_name(name);
            format!("{name}_")
        } else {
            String::new()
        };
        let name = mangle_name(name);
        format!("{interface}{name}")
    };

    match ty {
        Type::Bool => "bool".into(),
        Type::U8 => "u8".into(),
        Type::U16 => "u16".into(),
        Type::U32 => "u32".into(),
        Type::U64 => "u64".into(),
        Type::S8 => "s8".into(),
        Type::S16 => "s16".into(),
        Type::S32 => "s32".into(),
        Type::S64 => "s64".into(),
        Type::ErrorContext => "error_context".into(),
        Type::F32 => "f32".into(),
        Type::F64 => "f64".into(),
        Type::Char => "char".into(),
        Type::String => "string".into(),
        Type::Record(ty) => {
            let ty = &metadata.records[ty];
            full_name(ty.interface.as_deref(), &ty.name)
        }
        Type::Own(ty) | Type::Borrow(ty) => {
            let ty = &metadata.resources[ty];
            full_name(ty.interface.as_deref(), &ty.name)
        }
        Type::Flags(ty) => {
            let ty = &metadata.flags[ty];
            full_name(ty.interface.as_deref(), &ty.name)
        }
        Type::Enum(ty) => {
            let ty = &metadata.enums[ty];
            full_name(ty.interface.as_deref(), &ty.name)
        }
        Type::Variant(ty) => {
            let ty = &metadata.variants[ty];
            full_name(ty.interface.as_deref(), &ty.name)
        }
        Type::Tuple(ty) => {
            let ty = &metadata.tuples[ty];
            let count = ty.types.len();
            let types = ty
                .types
                .iter()
                .map(|&ty| {
                    let name = mangle_ty(metadata, ty);
                    format!("_{name}")
                })
                .collect::<Vec<_>>()
                .concat();
            format!("tuple{count}{types}")
        }
        Type::Option(ty) => {
            let ty = &metadata.options[ty];
            let name = mangle_ty(metadata, ty.ty);
            format!("option_{name}")
        }
        Type::Result(ty) => {
            let ty = &metadata.results[ty];
            let ok = if let Some(ty) = ty.ok {
                mangle_ty(metadata, ty)
            } else {
                "unit".into()
            };
            let err = if let Some(ty) = ty.err {
                mangle_ty(metadata, ty)
            } else {
                "unit".into()
            };
            format!("result_{ok}_{err}")
        }
        Type::List(ty) => {
            let ty = &metadata.lists[ty];
            let name = mangle_ty(metadata, ty.ty);
            format!("list_{name}")
        }
        Type::FixedLengthList(_) => todo!(),
        Type::Future(ty) => {
            let ty = &metadata.futures[ty];
            let ty = if let Some(ty) = ty.ty {
                mangle_ty(metadata, ty)
            } else {
                "unit".into()
            };
            format!("future_{ty}")
        }
        Type::Stream(ty) => {
            let ty = &metadata.streams[ty];
            let ty = if let Some(ty) = ty.ty {
                mangle_ty(metadata, ty)
            } else {
                "unit".into()
            };
            format!("stream_{ty}")
        }
        Type::Alias(ty) => {
            let ty = &metadata.aliases[ty];
            mangle_ty(metadata, ty.ty)
        }
    }
}
