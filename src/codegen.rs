use {
    heck::{ToLowerCamelCase as _, ToUpperCamelCase as _},
    std::collections::BTreeMap,
    wit_dylib::metadata::{Metadata, Type},
};

pub struct GeneratedCode {
    pub globals: String,
    pub modules: Vec<(String, String)>,
    pub script: String,
}

#[derive(Default)]
struct Resource {
    constructor: Option<usize>,
    methods: Vec<usize>,
    statics: Vec<usize>,
}

#[derive(Default)]
struct Interface<'a> {
    resources: BTreeMap<&'a str, Resource>,
    freestanding: Vec<usize>,
}

impl<'a> Interface<'a> {
    fn insert(&mut self, name: &'a str, index: usize) {
        if let Some(ty) = name.strip_prefix("[constructor]") {
            self.resources.entry(ty).or_default().constructor = Some(index);
        } else if let Some(func) = name.strip_prefix("[method]") {
            self.resources
                .entry(func.split_once('.').unwrap().0)
                .or_default()
                .methods
                .push(index);
        } else if let Some(func) = name.strip_prefix("[static]") {
            self.resources
                .entry(func.split_once('.').unwrap().0)
                .or_default()
                .statics
                .push(index);
        } else {
            self.freestanding.push(index)
        }
    }
}

pub fn generate(metadata: &Metadata) -> GeneratedCode {
    let mut modules = Vec::new();
    let mut world_module = String::new();

    // First, generate JS functions for any and all imported functions and/or
    // resources, grouping them by interface and emitting one ES module per
    // interface, plus another for world-level imports, if applicable.  Each
    // function will forward its parameters to `_componentizeJsCallImport`,
    // which will be provided by the runtime to call the imported function
    // itself.

    let mut imports = BTreeMap::<_, Interface>::new();

    for ty in metadata.resources.iter() {
        if ty.rep_elem_index.is_none() {
            imports
                .entry(&ty.interface)
                .or_default()
                .resources
                .insert(&ty.name, Resource::default());
        }
    }

    for (index, func) in metadata.import_funcs.iter().enumerate() {
        imports
            .entry(&func.interface)
            .or_default()
            .insert(&func.name, index);
    }

    for (interface_name, interface) in imports {
        let code = |index: usize, has_this| {
            let func = &metadata.import_funcs[index];
            let params = (if has_this { 1 } else { 0 }..func.args.len())
                .map(|i| format!("p{i}"))
                .collect::<Vec<_>>()
                .join(",");
            let this = if has_this { "this," } else { "" };
            let value = if func.async_import_elem_index.is_some() {
                format!(
                    "new Promise((a,b)=>\
                     _componentizeJsCallImport({index},[{this}{params}],a,b))"
                )
            } else {
                format!("_componentizeJsCallImport({index},[{this}{params}])")
            };
            format!("({params}){{return {value}}}\n")
        };

        let exports = interface
            .freestanding
            .into_iter()
            .map(|index| {
                let func = &metadata.import_funcs[index];
                let name = func.name.to_lower_camel_case();
                let code = code(index, false);
                format!("export function {name}{code}\n")
            })
            .chain(interface.resources.into_iter().map(|(ty, resource)| {
                let funcs = resource
                    .constructor
                    .into_iter()
                    .map(|index| {
                        let func = &metadata.import_funcs[index];
                        assert!(func.async_import_elem_index.is_none());
                        let code = code(index, false);
                        format!("constructor{code}\n")
                    })
                    .chain(resource.methods.into_iter().map(|index| {
                        let func = &metadata.import_funcs[index];
                        let name = func.name.split_once('.').unwrap().1.to_lower_camel_case();
                        let code = code(index, true);
                        format!("{name}{code}\n")
                    }))
                    .chain(resource.statics.into_iter().map(|index| {
                        let func = &metadata.import_funcs[index];
                        let name = func.name.split_once('.').unwrap().1.to_lower_camel_case();
                        let code = code(index, false);
                        format!("static {name}{code}\n")
                    }))
                    .chain(Some(
                        "drop(){{_componentizeJsDropResource.call(this)}}".to_string(),
                    ))
                    .collect::<Vec<_>>()
                    .concat();

                let ty = ty.to_upper_camel_case();
                format!("export class {ty} {{{funcs}}}\n")
            }))
            .collect::<Vec<_>>()
            .concat();

        if let Some(name) = interface_name {
            modules.push((name.to_string(), exports));
        } else {
            world_module.push_str(&exports);
        };
    }

    // Next, generate wrapper functions for any and all async function exports
    // so that they call back into the runtime when the promises resolve.

    let mut async_exports = BTreeMap::<_, Interface>::new();
    for (index, func) in metadata.export_funcs.iter().enumerate() {
        // TODO: As of this writing `wit-dylib`, won't tell us which functions
        // are async, so here we conservatively generate async wrappers for all
        // of them (except the constructors, which can't be async); the wrappers
        // for the sync functions won't actually be used.  We _could_ consult
        // the original `Resolve` for that information, but it would probably be
        // easier to modify `wit-dylib` to keep track of it so it's available in
        // `Metadata`.
        async_exports
            .entry(&func.interface)
            .or_default()
            .insert(&func.name, index);
    }

    let async_exports = async_exports
        .into_iter()
        .map(|(interface_name, interface)| {
            let interface_name = interface_name.as_deref().map(mangle_name);
            let fields = {
                let interface_name = interface_name
                    .as_ref()
                    .map(|v| format!("{v}."))
                    .unwrap_or_else(String::new);

                let params = |n| {
                    (0..n)
                        .map(|i| format!("p{i}"))
                        .collect::<Vec<_>>()
                        .join(",")
                };

                tbc(
                    "emit code that calls `Promise.catch` as well as `Promise.then`, \
                     passing another param to _componentizeJsCallTaskReturn indicating which one",
                );

                interface
                    .freestanding
                    .into_iter()
                    .map(|index| {
                        let func = &metadata.export_funcs[index];
                        let name = func.name.to_lower_camel_case();
                        let params = params(func.args.len());
                        let comma = if params.is_empty() { "" } else { "," };
                        format!(
                            "{name}:function(t{comma}{params}){{\n\
                             return {interface_name}{name}({params})\n\
                             .then((v)=>_componentizeJsCallTaskReturn({index},v,t))}}"
                        )
                    })
                    .chain(interface.resources.into_iter().map(|(ty, resource)| {
                        let ty = ty.to_upper_camel_case();
                        let funcs = resource
                            .methods
                            .into_iter()
                            .map(|index| {
                                let func = &metadata.export_funcs[index];
                                let name =
                                    func.name.split_once('.').unwrap().1.to_lower_camel_case();
                                let params = params(func.args.len() - 1);
                                let comma = if params.is_empty() { "" } else { "," };
                                format!(
                                    "{name}:function(t{comma}{params}){{\n\
                                     return this.{name}({params})\n\
                                     .then((v)=>_componentizeJsCallTaskReturn({index},v,t))}}"
                                )
                            })
                            .chain(resource.statics.into_iter().map(|index| {
                                let func = &metadata.export_funcs[index];
                                let name =
                                    func.name.split_once('.').unwrap().1.to_lower_camel_case();
                                let params = params(func.args.len());
                                let comma = if params.is_empty() { "" } else { "," };
                                format!(
                                    "{name}:function(t{comma}{params}){{\n\
                                     return {interface_name}{ty}.{name}({params})\n\
                                     .then((v)=>_componentizeJsCallTaskReturn({index},v,t))}}"
                                )
                            }))
                            .collect::<Vec<_>>()
                            .join(",\n");

                        let ty = ty.to_upper_camel_case();
                        format!("{ty}:{{{funcs}}}")
                    }))
                    .collect::<Vec<_>>()
                    .join(",\n")
            };

            if let Some(interface_name) = interface_name {
                format!("{interface_name}:{{{fields}}}")
            } else {
                fields
            }
        })
        .collect::<Vec<_>>()
        .join(",");

    // Next, generate constructors for any and all future and stream types.
    //
    // TODO: As of this writing, `wit-dylib` may generate multiple stream and/or
    // future types for a given payload type; that's a bug which should be
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

    // Next, generate a bit of utility code to add to the global object.
    //
    // `ComponentError` is used to represent `err` `result` values.  TODO:
    // ensure `Error` is added to the global object in the runtime so this can
    // extend it, per
    // https://github.com/bytecodealliance/jco/blob/bb56a3e2a30cc107c408a84591ef8788e3abbdf5/crates/js-component-bindgen/src/intrinsics/mod.rs#L281-L287
    //
    // `_componentizeJsWriteAll` is a utility function for use with streams that
    // happens to be easier to write in JS than in Rust.
    let globals = "class ComponentError {
  constructor(value) {
    this.payload = value
  }
}

var _componentizeJsWriteAll = async function(buffer) {
  let total = 0
  while (buffer.length > 0 && !this.readerDropped) {
    count = await this.write(buffer)
    buffer = buffer.slice(count)
    total += count
  }
  return total
}"
    .to_string();

    modules.push(("wit-world".to_string(), world_module));

    // Finally, return the result:
    GeneratedCode {
        globals,
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
