#![deny(warnings)]
#![expect(
    clippy::not_unsafe_ptr_arg_deref,
    reason = "wit_dylib_ffi::export produces code that does this"
)]

use {
    anyhow::{Context as _, anyhow, bail},
    mozjs::{
        context::JSContext,
        conversions::{Utf8Chars, jsstr_to_string},
        gc::Handle,
        glue::{
            CallObjectTracer, CallValueTracer, CreateRustJSPrincipals, DestroyRustJSPrincipals,
            JSPrincipalsCallbacks, PrintAndClearException,
        },
        jsapi::{
            GCTraceKindToAscii, HandleValueArray, Heap, JS_CallArgsFromVp, JS_GetFunctionObject,
            JS_HoldPrincipals, JSAutoRealm, JSCLASS_GLOBAL_FLAGS, JSClass, JSClassOps,
            JSContext as RawJSContext, JSObject, JSTracer, OnNewGlobalHookOption, TraceKind, Value,
        },
        jsval::{
            BigIntValue, BooleanValue, DoubleValue, Int32Value, NullValue, ObjectValue,
            StringValue, UInt32Value, UndefinedValue,
        },
        rooted,
        rust::{
            self, CompileOptionsWrapper, JSEngine, RealmOptions, Runtime,
            wrappers2::{
                BigIntFromInt64, BigIntFromUint64, BigIntToString, CurrentGlobalOrNull, Evaluate,
                JS_AddExtraGCRootsTracer, JS_CallFunctionValue, JS_DeleteProperty1, JS_GetElement,
                JS_GetProperty, JS_InitDestroyPrincipalsCallback, JS_NewFunction,
                JS_NewGlobalObject, JS_NewObject, JS_NewStringCopyUTF8N, JS_SetProperty,
                JS_ValueToObject, NewArrayObject, NewArrayObject1, RunJobs,
            },
        },
        typedarray::{ArrayBuffer, CreateWith},
    },
    std::{
        alloc::{self, Layout},
        collections::{BTreeMap, HashMap, HashSet},
        ffi::{CString, c_char, c_void},
        hash::{BuildHasherDefault, DefaultHasher, Hash, Hasher},
        marker::PhantomData,
        mem,
        ptr::{self, NonNull},
        slice,
        sync::{Arc, Mutex, OnceLock},
    },
    wit_dylib_ffi::{
        self as wit, Call, ExportFunction, Interpreter, List, Type, Wit, WitOption, WitResult,
    },
};

mod bindings {
    wit_bindgen::generate!({
        world: "init",
        path: "../init.wit",
        generate_all,
        disable_run_ctors_once_workaround: true,
    });

    use super::MyExports;

    export!(MyExports);
}

#[link(wasm_import_module = "$root")]
unsafe extern "C" {
    #[link_name = "[subtask-drop]"]
    pub fn subtask_drop(task: u32);
}
#[link(wasm_import_module = "$root")]
unsafe extern "C" {
    #[link_name = "[waitable-set-new]"]
    pub fn waitable_set_new() -> u32;
}
#[link(wasm_import_module = "$root")]
unsafe extern "C" {
    #[link_name = "[waitable-join]"]
    pub fn waitable_join(waitable: u32, set: u32);
}
#[link(wasm_import_module = "$root")]
unsafe extern "C" {
    #[link_name = "[waitable-set-drop]"]
    pub fn waitable_set_drop(set: u32);
}
#[link(wasm_import_module = "$root")]
unsafe extern "C" {
    #[link_name = "[context-get-0]"]
    pub fn context_get() -> u32;
}
#[link(wasm_import_module = "$root")]
unsafe extern "C" {
    #[link_name = "[context-set-0]"]
    pub fn context_set(value: u32);
}

pub const EVENT_NONE: u32 = 0;
pub const EVENT_SUBTASK: u32 = 1;

pub const STATUS_STARTING: u32 = 0;
pub const STATUS_STARTED: u32 = 1;
pub const STATUS_RETURNED: u32 = 2;

pub const CALLBACK_CODE_EXIT: u32 = 0;
pub const CALLBACK_CODE_WAIT: u32 = 2;

struct Borrow {
    value: Box<Heap<*mut JSObject>>,
    handle: u32,
    drop: unsafe extern "C" fn(u32),
}

struct EmptyResource {
    value: Box<Heap<*mut JSObject>>,
    #[expect(dead_code, reason = "will be used later")]
    handle: u32,
}

enum Pending {
    ImportCall {
        index: usize,
        call: MyCall<'static>,
        buffer: *mut u8,
    },
    #[expect(unused)]
    StreamRead, // etc.
}

#[derive(Default)]
struct TaskState {
    pending: HashMap<u32, Pending>,
    waitable_set: Option<u32>,
}

type JsFunction = unsafe extern "C" fn(*mut RawJSContext, u32, *mut Value) -> bool;

#[derive(Copy, Clone)]
struct SyncSend<T>(T);

unsafe impl<T> Sync for SyncSend<T> {}
unsafe impl<T> Send for SyncSend<T> {}

struct ArcHash<T>(Arc<T>);

impl<T> Hash for ArcHash<T> {
    fn hash<H>(&self, state: &mut H)
    where
        H: Hasher,
    {
        (Arc::as_ptr(&self.0) as usize).hash(state)
    }
}

impl<T> PartialEq for ArcHash<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<T> Eq for ArcHash<T> {}

enum TableEntry<T> {
    Occupied(T),
    Free(Option<usize>),
}

struct Table<T> {
    entries: Vec<TableEntry<T>>,
    free: Option<usize>,
}

impl<T> Table<T> {
    const fn new() -> Self {
        Self {
            entries: Vec::new(),
            free: None,
        }
    }

    fn insert(&mut self, v: T) -> usize {
        if let Some(free) = self.free {
            let &TableEntry::Free(next) = &self.entries[free] else {
                unreachable!();
            };
            self.free = next;
            self.entries[free] = TableEntry::Occupied(v);
            free
        } else {
            let index = self.entries.len();
            self.entries.push(TableEntry::Occupied(v));
            index
        }
    }

    fn get(&self, index: usize) -> &T {
        let TableEntry::Occupied(value) = &self.entries[index] else {
            unreachable!();
        };
        value
    }

    fn remove(&mut self, index: usize) -> T {
        let TableEntry::Occupied(value) =
            mem::replace(&mut self.entries[index], TableEntry::Free(self.free))
        else {
            unreachable!();
        };
        self.free = Some(index);
        value
    }

    fn iter(&self) -> impl Iterator<Item = &T> {
        self.entries.iter().filter_map(|v| {
            if let TableEntry::Occupied(v) = v {
                Some(v)
            } else {
                None
            }
        })
    }
}

type TracedSet = HashSet<ArcHash<Mutex<Traced>>, BuildHasherDefault<DefaultHasher>>;

static WIT: OnceLock<Wit> = OnceLock::new();
static CONTEXT: OnceLock<SyncSend<NonNull<RawJSContext>>> = OnceLock::new();
static TRACED: Mutex<SyncSend<TracedSet>> =
    Mutex::new(SyncSend(HashSet::with_hasher(BuildHasherDefault::new())));
static CURRENT_TASK_STATE: Mutex<Option<SyncSend<TaskState>>> = Mutex::new(None);
static EXPORTED_RESOURCES: Mutex<SyncSend<Table<Box<Heap<*mut JSObject>>>>> =
    Mutex::new(SyncSend(Table::new()));

fn init_runtime() -> anyhow::Result<()> {
    let engine = JSEngine::init()
        .map_err(|e| anyhow!("{e:?}"))
        .context("JSEngine::init failed")?;

    let mut runtime = Runtime::new(engine.handle());

    mem::forget(engine);

    let cx = runtime.cx();

    unsafe {
        JS_AddExtraGCRootsTracer(cx, Some(trace_roots), ptr::null_mut());
    }

    let realm_options = RealmOptions::default();

    let principals = unsafe {
        let raw = CreateRustJSPrincipals(
            &JSPrincipalsCallbacks {
                write: None,
                isSystemOrAddonPrincipal: None,
            },
            ptr::null_mut(),
        );
        JS_InitDestroyPrincipalsCallback(cx, Some(DestroyRustJSPrincipals));
        JS_HoldPrincipals(raw);
        raw
    };

    let global_class_ops = Box::into_raw(Box::new(JSClassOps {
        addProperty: None,
        delProperty: None,
        enumerate: None,
        newEnumerate: None,
        resolve: None,
        mayResolve: None,
        finalize: None,
        call: None,
        construct: None,
        trace: None,
    }));

    let global_class = Box::into_raw(Box::new(JSClass {
        name: c"GlobalObject".as_ptr(),
        flags: JSCLASS_GLOBAL_FLAGS,
        cOps: global_class_ops,
        spec: ptr::null(),
        ext: ptr::null(),
        oOps: ptr::null(),
    }));

    let global_object = unsafe {
        JS_NewGlobalObject(
            cx,
            global_class,
            principals,
            OnNewGlobalHookOption::DontFireOnNewGlobalHook,
            &*realm_options,
        )
    };

    let cx = runtime.cx();
    mem::forget(JSAutoRealm::new(
        unsafe { cx.raw_cx_no_gc() },
        global_object,
    ));

    CONTEXT
        .set(SyncSend(NonNull::new(unsafe { cx.raw_cx() }).unwrap()))
        .map_err(drop)
        .unwrap();

    mem::forget(runtime);

    Ok(())
}

fn context() -> JSContext {
    unsafe { JSContext::from_ptr(CONTEXT.get().unwrap().0) }
}

fn release_borrows(traced: &Mutex<Traced>) {
    let cx = &mut context();
    while let Some(Borrow {
        value,
        handle,
        drop,
    }) = traced.try_lock().unwrap().borrows.pop()
    {
        rooted!(&in(cx) let value = value.get());
        for name in [c"handle", c"componentize_js_index"] {
            if !unsafe { JS_DeleteProperty1(cx, value.handle(), name.as_ptr() as *const c_char) } {
                unsafe { PrintAndClearException(cx.raw_cx()) }
                panic!("JS_DeleteProperty failed for `{}`", name.to_str().unwrap())
            }
        }

        unsafe {
            drop(handle);
        }
    }
}

unsafe extern "C" fn call_import(cx: *mut RawJSContext, argc: u32, vp: *mut Value) -> bool {
    assert!(argc >= 2);

    let args = unsafe { JS_CallArgsFromVp(argc, vp) };
    let cx = &mut unsafe { JSContext::from_ptr(NonNull::new(cx).unwrap()) };
    let index = args.index(0);
    let params = args.index(1);
    rooted!(&in(cx) let params = params.to_object());
    rooted!(&in(cx) let mut length = UndefinedValue());
    // TODO: Is there a quicker way to get the array length, e.g. using
    // `JS_GetPropertyById`?
    if !unsafe {
        JS_GetProperty(
            cx,
            params.handle(),
            c"length".as_ptr() as *const c_char,
            length.handle_mut(),
        )
    } {
        unsafe { PrintAndClearException(cx.raw_cx()) }
        panic!("JS_GetProperty failed for `length`")
    }

    let length = u32::try_from(length.to_int32()).unwrap();
    let func = WIT
        .get()
        .unwrap()
        .import_func(usize::try_from(index.to_int32()).unwrap());
    assert_eq!(func.params().len(), usize::try_from(length).unwrap());

    let mut call = MyCall::new();
    for index in 0..length {
        rooted!(&in(cx) let mut value = UndefinedValue());
        if !unsafe { JS_GetElement(cx, params.handle(), length - index - 1, value.handle_mut()) } {
            unsafe { PrintAndClearException(cx.raw_cx()) }
            panic!("JS_GetElement failed for `{index}`")
        }
        call.push(value.get());
    }

    if func.is_async() {
        assert_eq!(argc, 4);

        let resolve = args.index(2);
        let reject = args.index(3);

        if let Some(pending) = unsafe { func.call_import_async(&mut call) } {
            // Push the `resolve` and `reject` callbacks onto the call stack
            // where they can be traced; we'll pop them off again when we
            // receive an `EVENT_SUBTASK`/`STATUS_RETURNED` for the subtask.
            call.traced
                .try_lock()
                .unwrap()
                .stack
                .extend([Heap::boxed(resolve.get()), Heap::boxed(reject.get())]);

            let mut state = CURRENT_TASK_STATE.try_lock().unwrap();
            let state = &mut state.as_mut().unwrap().0;
            if state.waitable_set.is_none() {
                state.waitable_set = Some(unsafe { waitable_set_new() });
            }
            unsafe { waitable_join(pending.subtask, state.waitable_set.unwrap()) }
            state.pending.insert(
                pending.subtask,
                Pending::ImportCall {
                    index: usize::try_from(index.to_int32()).unwrap(),
                    call,
                    buffer: pending.buffer,
                },
            );
        } else {
            rooted!(&in(cx) let mut result = UndefinedValue());
            if func.result().is_some() {
                result.set(call.pop());
            }
            rooted!(&in(cx) let params = vec![result.get()]);
            rooted!(&in(cx) let mut result = UndefinedValue());
            if !unsafe {
                JS_CallFunctionValue(
                    cx,
                    Handle::<*mut JSObject>::null(),
                    Handle::from_raw(resolve),
                    &HandleValueArray::from(&params),
                    result.handle_mut(),
                )
            } {
                unsafe { PrintAndClearException(cx.raw_cx()) }
                panic!("JS_CallFunctionValue failed for `resolve`")
            }
        }

        args.rval().set(UndefinedValue())
    } else {
        assert_eq!(argc, 2);

        func.call_import_sync(&mut call);

        if func.result().is_some() {
            args.rval().set(call.pop());
        }
    }

    true
}

unsafe extern "C" fn call_task_return(_: *mut RawJSContext, argc: u32, vp: *mut Value) -> bool {
    assert_eq!(argc, 3);

    let args = unsafe { JS_CallArgsFromVp(argc, vp) };
    let index = args.index(0);
    let value = args.index(1);
    let borrows = args.index(2).to_int32();

    let func = WIT
        .get()
        .unwrap()
        .export_func(usize::try_from(index.to_int32()).unwrap());

    let mut call = MyCall::new();
    if func.result().is_some() {
        call.push(value.get());
    }

    func.call_task_return(&mut call);

    if borrows != 0 {
        release_borrows(unsafe { Arc::from_raw(borrows as *const Mutex<Traced>) }.as_ref());
    }

    args.rval().set(UndefinedValue());
    true
}

unsafe extern "C" fn log(cx: *mut RawJSContext, argc: u32, vp: *mut Value) -> bool {
    assert_eq!(argc, 1);
    let args = unsafe { JS_CallArgsFromVp(argc, vp) };
    let message = unsafe { jsstr_to_string(cx, NonNull::new(args.index(0).to_string()).unwrap()) };
    eprintln!("log: `{message}`");
    args.rval().set(UndefinedValue());
    true
}

fn init(script: &str) -> anyhow::Result<()> {
    init_runtime()?;

    let cx = &mut context();

    // First, add some Rust-defined functions to the global object.
    for (name, func) in [
        (c"componentize_js_call_import", call_import as JsFunction),
        (
            c"componentize_js_call_task_return",
            call_task_return as JsFunction,
        ),
        (c"componentize_js_log", log as JsFunction),
    ] {
        let func = unsafe { JS_NewFunction(cx, Some(func), 2, 0, ptr::null()) };
        rooted!(&in(cx) let mut func = ObjectValue(unsafe {
            JS_GetFunctionObject(func)
        }));
        rooted!(&in(cx) let global_object = unsafe { CurrentGlobalOrNull(cx) });
        if !unsafe {
            JS_SetProperty(
                cx,
                global_object.handle(),
                name.as_ptr() as *const c_char,
                func.handle(),
            )
        } {
            unsafe { PrintAndClearException(cx.raw_cx()) }
            bail!("JS_SetProperty failed")
        }
    }

    // Next, generate JS code which will add an `imports` property to the
    // global object containing any and all imported functions, each of
    // which will forward their parameters to `call_import`.
    //
    // TODO: Move this code to the host side of `componentize-js` and
    // thereby avoid creating a lot of temporary guest allocations that get
    // baked into the pre-init snapshot but never used at runtime.
    let mut imports = BTreeMap::<_, Vec<_>>::new();
    for (index, func) in WIT.get().unwrap().iter_import_funcs().enumerate() {
        imports
            .entry(func.interface())
            .or_default()
            .push((index, func));
    }

    let imports = imports
        .into_iter()
        .map(|(interface, funcs)| {
            let funcs = funcs
                .into_iter()
                .map(|(index, func)| {
                    let name = func.name().replace('-', "_");
                    let params = (0..func.params().len())
                        .map(|i| format!("p{i}"))
                        .collect::<Vec<_>>()
                        .join(",");
                    let value = if func.is_async() {
                        format!(
                            "new Promise((a,b)=>\
                                 componentize_js_call_import({index},[{params}],a,b))"
                        )
                    } else {
                        format!("componentize_js_call_import({index},[{params}])")
                    };
                    format!("{name}:function({params}){{return {value}}}")
                })
                .collect::<Vec<_>>()
                .join(",");

            if let Some(interface) = interface {
                let name = interface.replace([':', '/', '-'], "_");
                format!("{name}:{{{funcs}}}")
            } else {
                funcs
            }
        })
        .collect::<Vec<_>>()
        .join(",");

    // Next, generate JS code which will add a
    // `componentize_js_async_exports` property to the global object which
    // will wrap any and all async exports defined in the script so that
    // they call back into Rust when the promises resolve.
    //
    // TODO: As above, move this code to the host side of `componentize-js`.
    let mut exports = BTreeMap::<_, Vec<_>>::new();
    for (index, func) in WIT.get().unwrap().iter_export_funcs().enumerate() {
        // TODO: As of this writing `wit-dylib`, won't tell us which
        // functions are async, so here we conservatively generate async
        // wrappers for all of them; the wrappers for the sync functions
        // won't actually be used.  Once we move this code to the host side,
        // we'll have that information and can be more precise.
        exports
            .entry(func.interface())
            .or_default()
            .push((index, func));
    }

    // For some reason I have not yet determined, we need an `await` to
    // appear somewhere in the script to force `Promise` to be in scope.
    //
    // TODO: Figure out the right way to add `Promise` to the global scope without
    // this hack:
    let promise_hack =
        "var componentize_js_promise_hack = (async function(){await Promise.resolve(42)})()";

    let exports = exports
        .into_iter()
        .map(|(interface, funcs)| {
            let interface = interface.map(|v| v.replace([':', '/', '-'], "_"));
            let funcs = funcs
                .into_iter()
                .map(|(index, func)| {
                    let interface = interface
                        .as_ref()
                        .map(|v| format!("{v}."))
                        .unwrap_or_else(String::new);
                    let name = func.name().replace('-', "_");
                    let params = (0..func.params().len())
                        .map(|i| format!("p{i}"))
                        .collect::<Vec<_>>()
                        .join(",");
                    let comma = if func.params().len() == 0 { "" } else { "," };
                    format!(
                        "{name}:function(t{comma}{params}){{\n\
                             return exports.{interface}{name}({params})\n\
                             .then((v)=>componentize_js_call_task_return({index},v,t))}}"
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

    // Finally, append the generated code to the script and execute the
    // result.
    let script = format!(
        "{script}\n\
             var imports = {{{imports}}}\n\
             var componentize_js_async_exports = {{{exports}}}\n\
             {promise_hack}"
    );
    let compile_options = CompileOptionsWrapper::new(cx, c"script".into(), 1);
    let script = script.encode_utf16().collect::<Vec<_>>();
    let mut script = rust::transform_u16_to_source_text(&script);
    rooted!(&in(cx) let mut result = UndefinedValue());
    if !unsafe { Evaluate(cx, compile_options.ptr, &mut script, result.handle_mut()) } {
        unsafe { PrintAndClearException(cx.raw_cx()) }
        bail!("Evaluate failed")
    }
    Ok(())
}

fn poll(cx: &mut JSContext) -> u32 {
    unsafe { RunJobs(cx) }

    let mut state = CURRENT_TASK_STATE.try_lock().unwrap().take().unwrap().0;
    if state.pending.is_empty() {
        if let Some(set) = state.waitable_set.take() {
            unsafe { waitable_set_drop(set) }
        }

        CALLBACK_CODE_EXIT
    } else {
        let set = state.waitable_set.unwrap();
        unsafe { context_set(u32::try_from(Box::into_raw(Box::new(state)) as usize).unwrap()) }

        CALLBACK_CODE_WAIT | (set << 4)
    }
}

struct MyExports;

impl bindings::Guest for MyExports {
    fn init(script: String) -> Result<(), String> {
        let result = init(&script).map_err(|e| format!("{e:?}"));

        // This tells the WASI Preview 1 component adapter to reset its state.
        // In particular, we want it to forget about any open handles and
        // re-request the stdio handles at runtime since we'll be running under
        // a brand new host.
        #[link(wasm_import_module = "wasi_snapshot_preview1")]
        unsafe extern "C" {
            #[link_name = "reset_adapter_state"]
            fn reset_adapter_state();
        }

        // This tells wasi-libc to reset its preopen state, forcing
        // re-initialization at runtime.
        #[link(wasm_import_module = "env")]
        unsafe extern "C" {
            #[link_name = "__wasilibc_reset_preopens"]
            fn wasilibc_reset_preopens();
        }

        unsafe {
            reset_adapter_state();
            wasilibc_reset_preopens();
        }

        result
    }
}

struct MyInterpreter;

impl MyInterpreter {
    fn export_call_(func: ExportFunction, call: &mut MyCall<'_>, async_: bool) -> u32 {
        let name = || {
            if let Some(interface) = func.interface() {
                format!("{interface}#{}", func.name())
            } else {
                func.name().into()
            }
        };

        if async_ {
            *CURRENT_TASK_STATE.try_lock().unwrap() = Some(SyncSend(TaskState::default()));
        }

        let cx = &mut context();
        rooted!(&in(cx) let global_object = unsafe { CurrentGlobalOrNull(cx) });
        rooted!(&in(cx) let mut object = ptr::null_mut::<JSObject>());

        {
            rooted!(&in(cx) let mut value = UndefinedValue());
            if !unsafe {
                JS_GetProperty(
                    cx,
                    global_object.handle(),
                    if async_ {
                        c"componentize_js_async_exports"
                    } else {
                        c"exports"
                    }
                    .as_ptr() as *const c_char,
                    value.handle_mut(),
                )
            } {
                unsafe { PrintAndClearException(cx.raw_cx()) }
                panic!("JS_GetProperty failed for `{}`", name())
            }
            if !unsafe { JS_ValueToObject(cx, value.handle(), object.handle_mut()) } {
                unsafe { PrintAndClearException(cx.raw_cx()) }
                panic!("JS_ValueToObject failed for `{}`", name())
            }
        }

        if let Some(interface) = func.interface() {
            rooted!(&in(cx) let mut value = UndefinedValue());
            if !unsafe {
                JS_GetProperty(
                    cx,
                    object.handle(),
                    CString::new(interface.replace([':', '/', '-'], "_"))
                        .unwrap()
                        .as_bytes_with_nul()
                        .as_ptr() as *const c_char,
                    value.handle_mut(),
                )
            } {
                unsafe { PrintAndClearException(cx.raw_cx()) }
                panic!("JS_GetProperty failed for `{}`", name())
            }
            if !unsafe { JS_ValueToObject(cx, value.handle(), object.handle_mut()) } {
                unsafe { PrintAndClearException(cx.raw_cx()) }
                panic!("JS_ValueToObject failed for `{}`", name())
            }
        }

        rooted!(&in(cx) let mut function = UndefinedValue());
        if !unsafe {
            JS_GetProperty(
                cx,
                object.handle(),
                CString::new(func.name().replace('-', "_"))
                    .unwrap()
                    .as_bytes_with_nul()
                    .as_ptr() as *const c_char,
                function.handle_mut(),
            )
        } {
            unsafe { PrintAndClearException(cx.raw_cx()) }
            panic!("JS_GetProperty failed for `{}`", name())
        }

        let params = if async_ {
            Some(UInt32Value(
                (Arc::into_raw(call.traced.clone()) as usize)
                    .try_into()
                    .unwrap(),
            ))
        } else {
            None
        }
        .into_iter()
        .chain(
            mem::take(&mut call.traced.try_lock().unwrap().stack)
                .into_iter()
                .map(|v| v.get()),
        )
        .collect::<Vec<_>>();
        rooted!(&in(cx) let params = params);
        rooted!(&in(cx) let mut result = UndefinedValue());
        if !unsafe {
            JS_CallFunctionValue(
                cx,
                object.handle(),
                function.handle(),
                &HandleValueArray::from(&params),
                result.handle_mut(),
            )
        } {
            unsafe { PrintAndClearException(cx.raw_cx()) }
            panic!("JS_CallFunctionValue failed for `{}`", name())
        }

        if async_ {
            poll(cx)
        } else {
            if func.result().is_some() {
                call.push(result.get());
            }

            release_borrows(&call.traced);

            0
        }
    }
}

impl Interpreter for MyInterpreter {
    type CallCx<'a> = MyCall<'a>;

    fn initialize(wit: Wit) {
        WIT.set(wit).map_err(drop).unwrap();
    }

    fn export_start<'a>(_: Wit, _: ExportFunction) -> Box<MyCall<'a>> {
        Box::new(MyCall::new())
    }

    fn export_call(_: Wit, func: ExportFunction, cx: &mut MyCall<'_>) {
        Self::export_call_(func, cx, false);
    }

    fn export_async_start(_: Wit, func: ExportFunction, mut cx: Box<MyCall<'_>>) -> u32 {
        Self::export_call_(func, &mut cx, true)
    }

    fn export_async_callback(event0: u32, event1: u32, event2: u32) -> u32 {
        *CURRENT_TASK_STATE.try_lock().unwrap() = Some(SyncSend(*unsafe {
            Box::from_raw(context_get() as *mut TaskState)
        }));
        unsafe { context_set(0) }

        let cx = &mut context();

        match event0 {
            EVENT_NONE => {}
            EVENT_SUBTASK => match event2 {
                STATUS_STARTING => unreachable!(),
                STATUS_STARTED => {}
                STATUS_RETURNED => {
                    unsafe {
                        waitable_join(event1, 0);
                        subtask_drop(event1);
                    }

                    let Pending::ImportCall {
                        index,
                        buffer,
                        mut call,
                    } = CURRENT_TASK_STATE
                        .try_lock()
                        .unwrap()
                        .as_mut()
                        .unwrap()
                        .0
                        .pending
                        .remove(&event1)
                        .unwrap()
                    else {
                        unreachable!()
                    };

                    let func = WIT.get().unwrap().import_func(index);

                    unsafe { func.lift_import_async_result(&mut call, buffer) };
                    assert!(call.len() < 4);

                    rooted!(&in(cx) let mut result = UndefinedValue());
                    if func.result().is_some() {
                        result.set(call.pop());
                    }
                    _ = call.pop(); // skip `reject` callback
                    rooted!(&in(cx) let resolve = call.pop());
                    rooted!(&in(cx) let params = vec![result.get()]);
                    rooted!(&in(cx) let mut result = UndefinedValue());
                    if !unsafe {
                        JS_CallFunctionValue(
                            cx,
                            Handle::<*mut JSObject>::null(),
                            resolve.handle(),
                            &HandleValueArray::from(&params),
                            result.handle_mut(),
                        )
                    } {
                        unsafe { PrintAndClearException(cx.raw_cx()) }
                        panic!("JS_CallFunctionValue failed for `resolve`")
                    }
                }
                _ => todo!(),
            },
            _ => todo!(),
        }

        poll(cx)
    }

    fn resource_dtor(ty: wit::Resource, handle: usize) {
        _ = (ty, handle);
        todo!()
    }
}

struct Traced {
    #[expect(
        clippy::vec_box,
        reason = "`Heap` values must be boxed to ensure they are not moved"
    )]
    stack: Vec<Box<Heap<Value>>>,
    resources: Option<Vec<EmptyResource>>,
    borrows: Vec<Borrow>,
}

struct MyCall<'a> {
    _phantom: PhantomData<&'a ()>,
    iter_stack: Vec<usize>,
    deferred_deallocations: Vec<(*mut u8, Layout)>,
    strings: Vec<String>,
    traced: Arc<Mutex<Traced>>,
}

impl MyCall<'_> {
    #[expect(clippy::arc_with_non_send_sync)]
    fn new() -> Self {
        let traced = Arc::new(Mutex::new(Traced {
            stack: Vec::new(),
            resources: None,
            borrows: Vec::new(),
        }));
        assert!(TRACED.try_lock().unwrap().0.insert(ArcHash(traced.clone())));
        Self {
            _phantom: PhantomData,
            iter_stack: Vec::new(),
            deferred_deallocations: Vec::new(),
            strings: Vec::new(),
            traced,
        }
    }

    fn push(&mut self, value: Value) {
        self.traced
            .try_lock()
            .unwrap()
            .stack
            .push(Heap::boxed(value));
    }

    fn pop(&mut self) -> Value {
        self.traced.try_lock().unwrap().stack.pop().unwrap().get()
    }

    fn last(&self) -> Value {
        self.traced.try_lock().unwrap().stack.last().unwrap().get()
    }

    fn len(&self) -> usize {
        self.traced.try_lock().unwrap().stack.len()
    }

    fn imported_resource_to_canon(&mut self, value: Value, owned: bool) -> u32 {
        let cx = &mut context();
        rooted!(&in(cx) let value = value.to_object());
        rooted!(&in(cx) let mut handle = UndefinedValue());
        if !unsafe {
            JS_GetProperty(
                cx,
                value.handle(),
                c"handle".as_ptr() as *const c_char,
                handle.handle_mut(),
            )
        } {
            unsafe { PrintAndClearException(cx.raw_cx()) }
            panic!("JS_GetProperty failed for `handle`")
        }
        let handle = handle.to_int32() as u32;

        if owned {
            if !unsafe {
                JS_DeleteProperty1(cx, value.handle(), c"handle".as_ptr() as *const c_char)
            } {
                unsafe { PrintAndClearException(cx.raw_cx()) }
                panic!("JS_DeleteProperty failed for `handle`")
            }

            if let Some(resources) = &mut self.traced.try_lock().unwrap().resources.as_mut() {
                resources.push(EmptyResource {
                    value: Heap::boxed(value.get()),
                    handle,
                });
            }
        }

        handle
    }
}

impl Drop for MyCall<'_> {
    fn drop(&mut self) {
        assert!(
            TRACED
                .try_lock()
                .unwrap()
                .0
                .remove(&ArcHash(self.traced.clone()))
        );

        for &(ptr, layout) in &self.deferred_deallocations {
            unsafe {
                alloc::dealloc(ptr, layout);
            }
        }
    }
}

impl Call for MyCall<'_> {
    unsafe fn defer_deallocate(&mut self, ptr: *mut u8, layout: Layout) {
        self.deferred_deallocations.push((ptr, layout));
    }

    fn pop_u8(&mut self) -> u8 {
        (self.pop().to_int32() as u32).try_into().unwrap()
    }

    fn pop_u16(&mut self) -> u16 {
        (self.pop().to_int32() as u32).try_into().unwrap()
    }

    fn pop_u32(&mut self) -> u32 {
        self.pop().to_number() as u32
    }

    fn pop_u64(&mut self) -> u64 {
        let value = self.pop();
        if value.is_int32() {
            value.to_int32() as u64
        } else {
            // TODO: is there a more efficient way to do this?
            let cx = &mut context();
            rooted!(&in(cx) let value = value.to_bigint());
            rooted!(&in(cx) let value = unsafe { BigIntToString(cx, value.handle(), 10) });
            unsafe { jsstr_to_string(cx.raw_cx(), NonNull::new(value.get()).unwrap()) }
                .parse()
                .unwrap()
        }
    }

    fn pop_s8(&mut self) -> i8 {
        self.pop().to_int32().try_into().unwrap()
    }

    fn pop_s16(&mut self) -> i16 {
        self.pop().to_int32().try_into().unwrap()
    }

    fn pop_s32(&mut self) -> i32 {
        self.pop().to_int32()
    }

    fn pop_s64(&mut self) -> i64 {
        let value = self.pop();
        if value.is_int32() {
            value.to_int32() as i64
        } else {
            // TODO: is there a more efficient way to do this?
            let cx = &mut context();
            rooted!(&in(cx) let value = value.to_bigint());
            rooted!(&in(cx) let value = unsafe { BigIntToString(cx, value.handle(), 10) });
            unsafe { jsstr_to_string(cx.raw_cx(), NonNull::new(value.get()).unwrap()) }
                .parse()
                .unwrap()
        }
    }

    fn pop_bool(&mut self) -> bool {
        self.pop().to_boolean()
    }

    fn pop_char(&mut self) -> char {
        let cx = &mut context();
        let value =
            unsafe { jsstr_to_string(cx.raw_cx(), NonNull::new(self.pop().to_string()).unwrap()) };
        let mut chars = value.chars();
        let value = chars.next().unwrap();
        assert!(chars.next().is_none());
        value
    }

    fn pop_f32(&mut self) -> f32 {
        // TODO: Assert that the number fits into an f32 losslessly
        self.pop().to_number() as f32
    }

    fn pop_f64(&mut self) -> f64 {
        self.pop().to_number()
    }

    fn pop_string(&mut self) -> &str {
        let cx = &mut context();
        let value =
            unsafe { jsstr_to_string(cx.raw_cx(), NonNull::new(self.pop().to_string()).unwrap()) };
        self.strings.push(value);
        self.strings.last().unwrap()
    }

    fn pop_borrow(&mut self, ty: wit::Resource) -> u32 {
        let value = self.pop();
        if let Some(new) = ty.new() {
            // exported resource type
            exported_resource_to_canon(ty, new, value)
        } else {
            // imported resource type
            self.imported_resource_to_canon(value, false)
        }
    }

    fn pop_own(&mut self, ty: wit::Resource) -> u32 {
        let value = self.pop();
        if let Some(new) = ty.new() {
            // exported resource type
            exported_resource_to_canon(ty, new, value)
        } else {
            // imported resource type
            self.imported_resource_to_canon(value, true)
        }
    }

    fn pop_enum(&mut self, _ty: wit::Enum) -> u32 {
        let cx = &mut context();
        rooted!(&in(cx) let wrapper = self.pop().to_object());
        rooted!(&in(cx) let mut value = UndefinedValue());
        if !unsafe {
            JS_GetProperty(
                cx,
                wrapper.handle(),
                c"discriminant".as_ptr() as *const c_char,
                value.handle_mut(),
            )
        } {
            unsafe { PrintAndClearException(cx.raw_cx()) }
            panic!("JS_GetProperty failed for `value`")
        }
        value.get().to_int32() as u32
    }

    fn pop_flags(&mut self, _ty: wit::Flags) -> u32 {
        let cx = &mut context();
        rooted!(&in(cx) let wrapper = self.pop().to_object());
        rooted!(&in(cx) let mut value = UndefinedValue());
        if !unsafe {
            JS_GetProperty(
                cx,
                wrapper.handle(),
                c"value".as_ptr() as *const c_char,
                value.handle_mut(),
            )
        } {
            unsafe { PrintAndClearException(cx.raw_cx()) }
            panic!("JS_GetProperty failed for `value`")
        }
        value.get().to_int32() as u32
    }

    fn pop_future(&mut self, _ty: wit::Future) -> u32 {
        let value = self.pop();
        self.imported_resource_to_canon(value, true)
    }

    fn pop_stream(&mut self, _ty: wit::Stream) -> u32 {
        let value = self.pop();
        self.imported_resource_to_canon(value, true)
    }

    fn pop_option(&mut self, ty: WitOption) -> u32 {
        if self.last().is_null() {
            self.pop();
            0
        } else {
            if let Type::Option(_) = ty.ty() {
                let cx = &mut context();
                rooted!(&in(cx) let wrapper = self.pop().to_object());
                rooted!(&in(cx) let mut value = UndefinedValue());
                if !unsafe {
                    JS_GetProperty(
                        cx,
                        wrapper.handle(),
                        c"value".as_ptr() as *const c_char,
                        value.handle_mut(),
                    )
                } {
                    unsafe { PrintAndClearException(cx.raw_cx()) }
                    panic!("JS_GetProperty failed for `value`")
                }
                self.push(value.get());
            } else {
                // Leave value on the stack as-is.
            }
            1
        }
    }

    fn pop_result(&mut self, ty: WitResult) -> u32 {
        let cx = &mut context();
        rooted!(&in(cx) let wrapper = self.pop().to_object());
        rooted!(&in(cx) let mut discriminant = UndefinedValue());
        if !unsafe {
            JS_GetProperty(
                cx,
                wrapper.handle(),
                c"discriminant".as_ptr() as *const c_char,
                discriminant.handle_mut(),
            )
        } {
            unsafe { PrintAndClearException(cx.raw_cx()) }
            panic!("JS_GetProperty failed for `discriminant`")
        }
        let discriminant = discriminant.to_int32() as u32;

        let has_payload = match discriminant {
            0 => ty.ok().is_some(),
            1 => ty.err().is_some(),
            _ => unreachable!(),
        };

        if has_payload {
            rooted!(&in(cx) let mut payload = UndefinedValue());
            if !unsafe {
                JS_GetProperty(
                    cx,
                    wrapper.handle(),
                    c"value".as_ptr() as *const c_char,
                    payload.handle_mut(),
                )
            } {
                unsafe { PrintAndClearException(cx.raw_cx()) }
                panic!("JS_GetProperty failed for `value3`")
            }
            self.push(payload.get());
        }

        discriminant
    }

    fn pop_variant(&mut self, ty: wit::Variant) -> u32 {
        let cx = &mut context();
        rooted!(&in(cx) let wrapper = self.pop().to_object());
        rooted!(&in(cx) let mut discriminant = UndefinedValue());
        if !unsafe {
            JS_GetProperty(
                cx,
                wrapper.handle(),
                c"discriminant".as_ptr() as *const c_char,
                discriminant.handle_mut(),
            )
        } {
            unsafe { PrintAndClearException(cx.raw_cx()) }
            panic!("JS_GetProperty failed for `discriminant`")
        }
        let discriminant = discriminant.to_int32() as u32;

        let has_payload = ty
            .cases()
            .nth(usize::try_from(discriminant).unwrap())
            .unwrap()
            .1
            .is_some();

        if has_payload {
            rooted!(&in(cx) let mut payload = UndefinedValue());
            if !unsafe {
                JS_GetProperty(
                    cx,
                    wrapper.handle(),
                    c"value".as_ptr() as *const c_char,
                    payload.handle_mut(),
                )
            } {
                unsafe { PrintAndClearException(cx.raw_cx()) }
                panic!("JS_GetProperty failed for `value3`")
            }
            self.push(payload.get());
        }

        discriminant
    }

    fn pop_record(&mut self, ty: wit::Record) {
        let cx = &mut context();
        rooted!(&in(cx) let record = self.pop().to_object());
        for (name, _) in ty.fields() {
            rooted!(&in(cx) let mut field = UndefinedValue());
            if !unsafe {
                JS_GetProperty(
                    cx,
                    record.handle(),
                    CString::new(name.replace('-', "_"))
                        .unwrap()
                        .as_bytes_with_nul()
                        .as_ptr() as *const c_char,
                    field.handle_mut(),
                )
            } {
                unsafe { PrintAndClearException(cx.raw_cx()) }
                panic!("JS_GetProperty failed for `{name}`")
            }
            self.push(field.get());
        }
    }

    fn pop_tuple(&mut self, ty: wit::Tuple) {
        let count = ty.types().len();
        let cx = &mut context();
        rooted!(&in(cx) let tuple = self.pop().to_object());
        for index in 0..count {
            rooted!(&in(cx) let mut value = UndefinedValue());
            if !unsafe {
                JS_GetElement(
                    cx,
                    tuple.handle(),
                    u32::try_from(count - index - 1).unwrap(),
                    value.handle_mut(),
                )
            } {
                unsafe { PrintAndClearException(cx.raw_cx()) }
                panic!("JS_GetElement failed for `{index}`")
            }
            self.push(value.get());
        }
    }

    unsafe fn maybe_pop_list(&mut self, ty: List) -> Option<(*const u8, usize)> {
        if let Type::U8 | Type::S8 = ty.ty() {
            let buffer = ArrayBuffer::from(self.pop().to_object()).unwrap();
            let len = buffer.len();
            let dst = unsafe {
                let dst = alloc::alloc(Layout::from_size_align(len, 1).unwrap());
                slice::from_raw_parts_mut(dst, len).copy_from_slice(buffer.as_slice());
                dst
            };
            Some((dst as _, len))
        } else {
            None
        }
    }

    fn pop_list(&mut self, _ty: List) -> usize {
        self.iter_stack.push(0);
        let cx = &mut context();
        rooted!(&in(cx) let list = self.last().to_object());
        rooted!(&in(cx) let mut length = UndefinedValue());
        // TODO: Is there a quicker way to get the array length, e.g. using
        // `JS_GetPropertyById`?
        if !unsafe {
            JS_GetProperty(
                cx,
                list.handle(),
                c"length".as_ptr() as *const c_char,
                length.handle_mut(),
            )
        } {
            unsafe { PrintAndClearException(cx.raw_cx()) }
            panic!("JS_GetProperty failed for `length`")
        }
        length.to_int32().try_into().unwrap()
    }

    fn pop_iter_next(&mut self, _ty: List) {
        let index = *self.iter_stack.last().unwrap();
        let cx = &mut context();
        rooted!(&in(cx) let list = self.last().to_object());
        rooted!(&in(cx) let mut value = UndefinedValue());
        if !unsafe {
            JS_GetElement(
                cx,
                list.handle(),
                u32::try_from(index).unwrap(),
                value.handle_mut(),
            )
        } {
            unsafe { PrintAndClearException(cx.raw_cx()) }
            panic!("JS_GetElement failed for `{index}`")
        }
        *self.iter_stack.last_mut().unwrap() = index + 1;
        self.push(value.get());
    }

    fn pop_iter(&mut self, _ty: List) {
        self.iter_stack.pop().unwrap();
        self.pop();
    }

    fn push_bool(&mut self, val: bool) {
        self.push(BooleanValue(val));
    }

    fn push_char(&mut self, val: char) {
        let cx = &mut context();
        self.push(StringValue(unsafe {
            &*JS_NewStringCopyUTF8N(cx, &*Utf8Chars::from(val.to_string().as_str()))
        }));
    }

    fn push_u8(&mut self, val: u8) {
        self.push(UInt32Value(val as u32));
    }

    fn push_s8(&mut self, val: i8) {
        self.push(Int32Value(val as i32));
    }

    fn push_u16(&mut self, val: u16) {
        self.push(UInt32Value(val as u32));
    }

    fn push_s16(&mut self, val: i16) {
        self.push(Int32Value(val as i32));
    }

    fn push_u32(&mut self, val: u32) {
        self.push(UInt32Value(val));
    }

    fn push_s32(&mut self, val: i32) {
        self.push(Int32Value(val));
    }

    fn push_u64(&mut self, val: u64) {
        if let Ok(val) = u32::try_from(val) {
            self.push(UInt32Value(val));
        } else {
            let cx = &mut context();
            self.push(BigIntValue(unsafe { &*BigIntFromUint64(cx, val) }));
        }
    }

    fn push_s64(&mut self, val: i64) {
        if let Ok(val) = i32::try_from(val) {
            self.push(Int32Value(val));
        } else {
            let cx = &mut context();
            self.push(BigIntValue(unsafe { &*BigIntFromInt64(cx, val) }));
        }
    }

    fn push_f32(&mut self, mut val: f32) {
        if val.is_nan() {
            // As of this writing, an assertion in `DoubleValue` will panic for
            // certain flavors of NaN, so we canonicalize here:
            val = f32::NAN;
        }
        self.push(DoubleValue(val as f64))
    }

    fn push_f64(&mut self, mut val: f64) {
        if val.is_nan() {
            // As of this writing, an assertion in `DoubleValue` will panic for
            // certain flavors of NaN, so we canonicalize here:
            val = f64::NAN;
        }
        self.push(DoubleValue(val))
    }

    fn push_string(&mut self, val: String) {
        let cx = &mut context();
        self.push(StringValue(unsafe {
            &*JS_NewStringCopyUTF8N(cx, &*Utf8Chars::from(val.as_str()))
        }));
    }

    fn push_record(&mut self, ty: wit::Record) {
        let cx = &mut context();
        rooted!(&in(cx) let value = unsafe { JS_NewObject(cx, ptr::null_mut()) });
        for (name, _) in ty.fields() {
            rooted!(&in(cx) let field = self.pop());
            if !unsafe {
                JS_SetProperty(
                    cx,
                    value.handle(),
                    CString::new(name.replace('-', "_"))
                        .unwrap()
                        .as_bytes_with_nul()
                        .as_ptr() as *const c_char,
                    field.handle(),
                )
            } {
                unsafe { PrintAndClearException(cx.raw_cx()) }
                panic!("JS_SetProperty failed")
            }
        }
        self.push(ObjectValue(value.get()));
    }

    fn push_tuple(&mut self, ty: wit::Tuple) {
        let start = self.len().checked_sub(ty.types().len()).unwrap();
        let elements = self
            .traced
            .try_lock()
            .unwrap()
            .stack
            .drain(start..)
            .map(|v| v.get())
            .collect::<Vec<_>>();

        let cx = &mut context();
        rooted!(&in(cx) let elements = elements);
        self.push(ObjectValue(unsafe {
            NewArrayObject(cx, &HandleValueArray::from(&elements))
        }));
    }

    fn push_flags(&mut self, _ty: wit::Flags, bits: u32) {
        let cx = &mut context();
        rooted!(&in(cx) let wrapper = unsafe { JS_NewObject(cx, ptr::null_mut()) });
        rooted!(&in(cx) let value = UInt32Value(bits));
        if !unsafe {
            JS_SetProperty(
                cx,
                wrapper.handle(),
                c"value".as_ptr() as *const c_char,
                value.handle(),
            )
        } {
            unsafe { PrintAndClearException(cx.raw_cx()) }
            panic!("JS_SetProperty failed")
        }
        self.push(ObjectValue(wrapper.get()));
    }

    fn push_enum(&mut self, _ty: wit::Enum, discriminant: u32) {
        let cx = &mut context();
        rooted!(&in(cx) let wrapper = unsafe { JS_NewObject(cx, ptr::null_mut()) });
        rooted!(&in(cx) let discriminant = UInt32Value(discriminant));
        if !unsafe {
            JS_SetProperty(
                cx,
                wrapper.handle(),
                c"discriminant".as_ptr() as *const c_char,
                discriminant.handle(),
            )
        } {
            unsafe { PrintAndClearException(cx.raw_cx()) }
            panic!("JS_SetProperty failed")
        }
        self.push(ObjectValue(wrapper.get()));
    }

    fn push_borrow(&mut self, ty: wit::Resource, handle: u32) {
        self.push(ObjectValue(if ty.rep().is_some() {
            // exported resource type
            EXPORTED_RESOURCES
                .try_lock()
                .unwrap()
                .0
                .get(handle.try_into().unwrap())
                .get()
        } else {
            // imported resource type
            let value = imported_resource_from_canon(ty.index(), handle);

            self.traced.try_lock().unwrap().borrows.push(Borrow {
                value: Heap::boxed(value),
                handle,
                drop: ty.drop(),
            });

            value
        }));
    }

    fn push_own(&mut self, ty: wit::Resource, handle: u32) {
        self.push(ObjectValue(if let Some(rep) = ty.rep() {
            // exported resource type
            let cx = &mut context();
            let rep = unsafe { rep(handle) };
            rooted!(&in(cx) let value = EXPORTED_RESOURCES.try_lock().unwrap().0.remove(rep).get());

            for name in [c"componentize_js_handle", c"componentize_js_index"] {
                if !unsafe {
                    JS_DeleteProperty1(cx, value.handle(), name.as_ptr() as *const c_char)
                } {
                    unsafe { PrintAndClearException(cx.raw_cx()) }
                    panic!("JS_DeleteProperty failed for `{}`", name.to_str().unwrap())
                }
            }

            value.get()
        } else {
            // imported resource type
            imported_resource_from_canon(ty.index(), handle)
        }));
    }

    fn push_future(&mut self, ty: wit::Future, handle: u32) {
        self.push(ObjectValue(imported_resource_from_canon(
            ty.index(),
            handle,
        )))
    }

    fn push_stream(&mut self, ty: wit::Stream, handle: u32) {
        self.push(ObjectValue(imported_resource_from_canon(
            ty.index(),
            handle,
        )))
    }

    fn push_variant(&mut self, ty: wit::Variant, discriminant: u32) {
        let cx = &mut context();
        rooted!(&in(cx) let wrapper = unsafe { JS_NewObject(cx, ptr::null_mut()) });
        {
            rooted!(&in(cx) let discriminant = UInt32Value(discriminant));
            if !unsafe {
                JS_SetProperty(
                    cx,
                    wrapper.handle(),
                    c"discriminant".as_ptr() as *const c_char,
                    discriminant.handle(),
                )
            } {
                unsafe { PrintAndClearException(cx.raw_cx()) }
                panic!("JS_SetProperty failed")
            }
        }

        if ty
            .cases()
            .nth(discriminant.try_into().unwrap())
            .unwrap()
            .1
            .is_some()
        {
            rooted!(&in(cx) let value = self.pop());
            if !unsafe {
                JS_SetProperty(
                    cx,
                    wrapper.handle(),
                    c"value".as_ptr() as *const c_char,
                    value.handle(),
                )
            } {
                unsafe { PrintAndClearException(cx.raw_cx()) }
                panic!("JS_SetProperty failed")
            }
        }

        self.push(ObjectValue(wrapper.get()));
    }

    fn push_option(&mut self, ty: WitOption, is_some: bool) {
        if is_some {
            if let Type::Option(_) = ty.ty() {
                let cx = &mut context();
                rooted!(&in(cx) let wrapper = unsafe { JS_NewObject(cx, ptr::null_mut()) });
                rooted!(&in(cx) let value = self.pop());
                if !unsafe {
                    JS_SetProperty(
                        cx,
                        wrapper.handle(),
                        c"value".as_ptr() as *const c_char,
                        value.handle(),
                    )
                } {
                    unsafe { PrintAndClearException(cx.raw_cx()) }
                    panic!("JS_SetProperty failed")
                }
                self.push(ObjectValue(wrapper.get()));
            } else {
                // Leave payload on the stack as-is.
            }
        } else {
            self.push(NullValue());
        }
    }

    fn push_result(&mut self, ty: WitResult, is_err: bool) {
        let cx = &mut context();
        rooted!(&in(cx) let wrapper = unsafe { JS_NewObject(cx, ptr::null_mut()) });
        rooted!(&in(cx) let discriminant = UInt32Value(if is_err { 1 } else { 0}));
        if !unsafe {
            JS_SetProperty(
                cx,
                wrapper.handle(),
                c"discriminant".as_ptr() as *const c_char,
                discriminant.handle(),
            )
        } {
            unsafe { PrintAndClearException(cx.raw_cx()) }
            panic!("JS_SetProperty failed")
        }
        if (is_err && ty.err().is_some()) || (!is_err && ty.ok().is_some()) {
            rooted!(&in(cx) let value = self.pop());
            if !unsafe {
                JS_SetProperty(
                    cx,
                    wrapper.handle(),
                    c"value".as_ptr() as *const c_char,
                    value.handle(),
                )
            } {
                unsafe { PrintAndClearException(cx.raw_cx()) }
                panic!("JS_SetProperty failed")
            }
        }
        self.push(ObjectValue(wrapper.get()));
    }

    unsafe fn push_raw_list(&mut self, ty: List, src: *mut u8, len: usize) -> bool {
        if let Type::U8 | Type::S8 = ty.ty() {
            let cx = &mut context();
            rooted!(&in(cx) let mut buffer = ptr::null_mut::<JSObject>());
            unsafe {
                ArrayBuffer::create(
                    cx.raw_cx(),
                    CreateWith::Slice(slice::from_raw_parts(src, len)),
                    buffer.handle_mut(),
                )
                .unwrap()
            }
            self.push(ObjectValue(buffer.get()));
            true
        } else {
            false
        }
    }

    fn push_list(&mut self, _ty: List, _capacity: usize) {
        // TODO: Ideally, we'd create a new JS Array with a length of `capacity`
        // and then fill in the elements using `list_append`, but that would
        // require keeping track of where we are in the array, which
        // `wit_dylib_ffi` doesn't help us with at this point.  Consider
        // modifying `wit_dylib_ffi` to support that (e.g. mirroring the
        // `pop_list`/`pop_iter_next`/`pop_iter` paradigm).
        let cx = &mut context();
        self.push(ObjectValue(unsafe { NewArrayObject1(cx, 0) }));
    }

    fn list_append(&mut self, _ty: List) {
        let cx = &mut context();
        rooted!(&in(cx) let element = self.pop());
        rooted!(&in(cx) let list = self.last().to_object());
        rooted!(&in(cx) let mut push = UndefinedValue());
        if !unsafe {
            JS_GetProperty(
                cx,
                list.handle(),
                c"push".as_ptr() as *const c_char,
                push.handle_mut(),
            )
        } {
            unsafe { PrintAndClearException(cx.raw_cx()) }
            panic!("JS_GetProperty failed for `push`")
        }

        rooted!(&in(cx) let params = vec![element.get()]);
        rooted!(&in(cx) let mut result = UndefinedValue());
        if !unsafe {
            JS_CallFunctionValue(
                cx,
                list.handle(),
                push.handle(),
                &HandleValueArray::from(&params),
                result.handle_mut(),
            )
        } {
            unsafe { PrintAndClearException(cx.raw_cx()) }
            panic!("JS_CallFunctionValue failed for `push`")
        }
    }
}

wit_dylib_ffi::export!(MyInterpreter);

fn imported_resource_from_canon(index: usize, handle: u32) -> *mut JSObject {
    let cx = &mut context();
    rooted!(&in(cx) let wrapper = unsafe { JS_NewObject(cx, ptr::null_mut()) });

    rooted!(&in(cx) let handle = UInt32Value(handle));
    if !unsafe {
        JS_SetProperty(
            cx,
            wrapper.handle(),
            c"handle".as_ptr() as *const c_char,
            handle.handle(),
        )
    } {
        unsafe { PrintAndClearException(cx.raw_cx()) }
        panic!("JS_SetProperty failed")
    }

    rooted!(&in(cx) let index = UInt32Value(index.try_into().unwrap()));
    if !unsafe {
        JS_SetProperty(
            cx,
            wrapper.handle(),
            c"componentize_js_type".as_ptr() as *const c_char,
            index.handle(),
        )
    } {
        unsafe { PrintAndClearException(cx.raw_cx()) }
        panic!("JS_SetProperty failed")
    }

    wrapper.get()
}

fn exported_resource_to_canon(
    ty: wit::Resource,
    new: unsafe extern "C" fn(usize) -> u32,
    value: Value,
) -> u32 {
    let cx = &mut context();
    rooted!(&in(cx) let value = value.to_object());
    rooted!(&in(cx) let mut handle = UndefinedValue());
    if !unsafe {
        JS_GetProperty(
            cx,
            value.handle(),
            c"componentize_js_handle".as_ptr() as *const c_char,
            handle.handle_mut(),
        )
    } {
        unsafe { PrintAndClearException(cx.raw_cx()) }
        panic!("JS_GetProperty failed for `componentize_js_handle`")
    }

    if handle.is_int32() {
        handle.to_int32() as u32
    } else {
        let rep = EXPORTED_RESOURCES
            .try_lock()
            .unwrap()
            .0
            .insert(Heap::boxed(value.get()));
        let handle = unsafe { new(rep as usize) };
        {
            rooted!(&in(cx) let handle = UInt32Value(handle));
            if !unsafe {
                JS_SetProperty(
                    cx,
                    value.handle(),
                    c"componentize_js_handle".as_ptr() as *const c_char,
                    handle.handle(),
                )
            } {
                unsafe { PrintAndClearException(cx.raw_cx()) }
                panic!("JS_SetProperty failed")
            }
        }

        rooted!(&in(cx) let index = UInt32Value(ty.index().try_into().unwrap()));
        if !unsafe {
            JS_SetProperty(
                cx,
                value.handle(),
                c"componentize_js_type".as_ptr() as *const c_char,
                index.handle(),
            )
        } {
            unsafe { PrintAndClearException(cx.raw_cx()) }
            panic!("JS_SetProperty failed")
        }

        handle
    }
}

unsafe extern "C" fn trace_roots(tracer: *mut JSTracer, _: *mut c_void) {
    for traced in TRACED.try_lock().unwrap().0.iter() {
        let mut traced = traced.0.try_lock().unwrap();
        for value in traced.stack.iter_mut() {
            if value.get().is_markable() {
                unsafe {
                    CallValueTracer(
                        tracer,
                        value.ptr.get() as *mut _,
                        GCTraceKindToAscii(value.get().trace_kind()),
                    )
                }
            }
        }
        for Borrow { value, .. } in traced.borrows.iter_mut() {
            unsafe {
                CallObjectTracer(
                    tracer,
                    value.ptr.get() as *mut _,
                    GCTraceKindToAscii(TraceKind::Object),
                )
            }
        }
        if let Some(resources) = traced.resources.as_mut() {
            for EmptyResource { value, .. } in resources.iter_mut() {
                unsafe {
                    CallObjectTracer(
                        tracer,
                        value.ptr.get() as *mut _,
                        GCTraceKindToAscii(TraceKind::Object),
                    )
                }
            }
        }
    }

    for value in EXPORTED_RESOURCES.try_lock().unwrap().0.iter() {
        unsafe {
            CallObjectTracer(
                tracer,
                value.ptr.get() as *mut _,
                GCTraceKindToAscii(TraceKind::Object),
            )
        }
    }
}

// As of this writing, recent Rust `nightly` builds include a version of the
// `libc` crate that expects `wasi-libc` to define the following global
// variables, but `wasi-libc` defines them as preprocessor constants which
// aren't visible at link time, so we need to define them somewhere.  Ideally,
// we should fix this upstream, but for now we work around it:

#[unsafe(no_mangle)]
static _CLOCK_PROCESS_CPUTIME_ID: u8 = 2;
#[unsafe(no_mangle)]
static _CLOCK_THREAD_CPUTIME_ID: u8 = 3;
