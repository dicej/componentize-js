#![deny(warnings)]
#![expect(
    clippy::not_unsafe_ptr_arg_deref,
    reason = "wit_dylib_ffi::export produces code that does this"
)]

use {
    anyhow::{Context as _, anyhow, bail},
    mozjs::{
        context::JSContext,
        glue::{
            CallObjectTracer, CallValueTracer, CreateRustJSPrincipals, DestroyRustJSPrincipals,
            JSPrincipalsCallbacks, PrintAndClearException,
        },
        jsapi::{
            GCTraceKindToAscii, HandleValueArray, Heap, JS_CallArgsFromVp, JS_GetFunctionObject,
            JS_HoldPrincipals, JSCLASS_GLOBAL_FLAGS, JSClass, JSClassOps,
            JSContext as RawJSContext, JSObject, JSTracer, OnNewGlobalHookOption, TraceKind, Value,
        },
        jsval::{ObjectValue, UInt32Value, UndefinedValue},
        realm::AutoRealm,
        rooted,
        rust::{
            self, CompileOptionsWrapper, JSEngine, RealmOptions, Runtime,
            wrappers2::{
                CurrentGlobalOrNull, Evaluate, JS_AddExtraGCRootsTracer, JS_CallFunctionValue,
                JS_GetElement, JS_GetProperty, JS_InitDestroyPrincipalsCallback, JS_NewFunction,
                JS_NewGlobalObject, JS_SetProperty, JS_ValueToObject,
            },
        },
    },
    std::{
        alloc::{self, Layout},
        collections::{BTreeMap, HashSet},
        ffi::{CString, c_char, c_void},
        hash::{BuildHasherDefault, DefaultHasher, Hash, Hasher},
        marker::PhantomData,
        mem,
        ops::DerefMut,
        ptr::{self, NonNull},
        sync::{Arc, Mutex, OnceLock},
    },
    wit_dylib_ffi::{
        self as wit, Call, ExportFunction, Interpreter, List, Wit, WitOption, WitResult,
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

static WIT: OnceLock<Wit> = OnceLock::new();

struct Borrow;
struct EmptyResource;

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

type Stacks = HashSet<ArcHash<Mutex<Vec<Box<Heap<Value>>>>>, BuildHasherDefault<DefaultHasher>>;

static RUNTIME: Mutex<Option<SyncSend<Runtime>>> = Mutex::new(None);
static GLOBAL_OBJECT: Mutex<Option<SyncSend<Box<Heap<*mut JSObject>>>>> = Mutex::new(None);
static STACKS: Mutex<Stacks> = Mutex::new(HashSet::with_hasher(BuildHasherDefault::new()));

fn make_runtime() -> anyhow::Result<Runtime> {
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

    *GLOBAL_OBJECT.try_lock().unwrap() = Some(SyncSend(Heap::boxed(unsafe {
        JS_NewGlobalObject(
            cx,
            global_class,
            principals,
            OnNewGlobalHookOption::DontFireOnNewGlobalHook,
            &*realm_options,
        )
    })));

    Ok(runtime)
}

fn with_context<T: 'static>(fun: impl FnOnce(&mut JSContext) -> T) -> T {
    let mut runtime = RUNTIME.lock().unwrap();
    if runtime.is_none() {
        *runtime = Some(SyncSend(make_runtime().unwrap()));
    }

    let runtime = &mut runtime.as_mut().unwrap().0;
    let mut realm = AutoRealm::new(
        runtime.cx(),
        NonNull::new(GLOBAL_OBJECT.try_lock().unwrap().as_ref().unwrap().0.get()).unwrap(),
    );
    fun(&mut realm)
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
        if !unsafe { JS_GetElement(cx, params.handle(), index, value.handle_mut()) } {
            unsafe { PrintAndClearException(cx.raw_cx()) }
            panic!("JS_GetProperty failed for `{index}`")
        }
        call.stack
            .try_lock()
            .unwrap()
            .push(Heap::boxed(value.get()));
    }

    if func.is_async() {
        assert_eq!(argc, 4);

        let resolve = args.index(2);
        let reject = args.index(3);

        if let Some(pending) = func.call_import_async(call) {
            let state = CURRENT_TASK_STATE.try_lock().unwrap().as_mut().unwrap();
            state.pending.insert(
                pending.subtask,
                Promise::ImportCall {
                    index,
                    call,
                    buffer: pending.buffer,
                    resolve: Heap::boxed(resolve),
                    reject: Heap::boxed(reject),
                },
            );
        } else {
            rooted!(&in(cx) let result = UndefinedValue());
            if func.result.is_some() {
                result.set(call.stack.try_lock().unwrap().pop().unwrap().get());
            }
            rooted!(&in(cx) let params = vec![result.get()]);
            rooted!(&in(cx) let object = UndefinedValue());
            rooted!(&in(cx) let result = UndefinedValue());
            if !unsafe {
                JS_CallFunctionValue(
                    cx,
                    object.handle(),
                    resolve.handle(),
                    &HandleValueArray::from(&params),
                    result.handle_mut(),
                )
            } {
                unsafe { PrintAndClearException(cx.raw_cx()) }
                panic!("JS_CallFunctionValue failed for `{}`", name())
            }
        }

        args.rval(UndefinedValue())
    } else {
        assert_eq!(argc, 2);

        func.call_import_sync(&mut call);
        if func.result().is_some() {
            args.rval()
                .set(call.stack.try_lock().unwrap().pop().unwrap().get());
        }
    }

    true
}

fn init(script: &str) -> anyhow::Result<()> {
    with_context(|cx| {
        // First, add `call_import` to the global object.
        let call_import = unsafe { JS_NewFunction(cx, Some(call_import), 2, 0, ptr::null()) };
        rooted!(&in(cx) let mut call_import = ObjectValue(unsafe {
            JS_GetFunctionObject(call_import)
        }));
        rooted!(&in(cx) let global_object = unsafe { CurrentGlobalOrNull(cx) });
        if !unsafe {
            JS_SetProperty(
                cx,
                global_object.handle(),
                c"componentize_js_call_import".as_ptr() as *const c_char,
                call_import.handle(),
            )
        } {
            unsafe { PrintAndClearException(cx.raw_cx()) }
            bail!("JS_SetProperty failed")
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
                                 componentize_js_call_import({index},[{params}],a,b)"
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
            imports
                .entry(func.interface())
                .or_default()
                .push((index, func));
        }

        let exports = exports
            .into_iter()
            .map(|(interface, funcs)| {
                let interface = interface.map(|v| v.replace([':', '/', '-'], "_"));
                let funcs = exports
                    .into_iter()
                    .map(|(index, func)| {
                        let interface = interface.map(|v| format!("{v}."));
                        let name = func.name().replace('-', "_");
                        let params = (0..func.params().len())
                            .map(|i| format!(",p{i}"))
                            .collect::<Vec<_>>()
                            .concat();
                        format!(
                            "{name}:function({params}){{\
                             return exports.{interface}{name}({params})\
                             .then((a,b)=>componentize_js_resolve({index},a,b)))\
                             }}"
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
            "{script}\nvar imports = {{{imports}}}\nvar componentize_js_async_exports = {{{imports}}}"
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
    })
}

fn poll(cx: &mut JSContext) -> u32 {
    unsafe { RunJobs(cx) }

    let state = CURRENT_TASK_STATE.try_lock().unwrap().take().unwrap();
    if state.pending.is_empty() {
        if let Some(set) = state.waitable_set.take() {
            waitable_set_drop(set);
        }

        CALLBACK_CODE_EXIT
    } else {
        let set = state.waitable_set.unwrap();
        context_set(Box::into_raw(state));

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
            *CURRENT_TASK_STATE.try_lock().unwrap() = Some(Box::new(TaskState::new()));
        }

        with_context(|cx| {
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
                    CString::new(func.name())
                        .unwrap()
                        .as_bytes_with_nul()
                        .as_ptr() as *const c_char,
                    function.handle_mut(),
                )
            } {
                unsafe { PrintAndClearException(cx.raw_cx()) }
                panic!("JS_GetProperty failed for `{}`", name())
            }

            let params = mem::take(call.stack.try_lock().unwrap().deref_mut())
                .into_iter()
                .map(|v| v.get())
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
                // TODO: Do we need to manually add the returned promise to the
                // job queue?
                poll(cx)
            } else {
                if func.result().is_some() {
                    call.stack
                        .try_lock()
                        .unwrap()
                        .push(Heap::boxed(result.get()));
                }

                0
            }
        })
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
        let state = unsafe { Box::from_raw(context_get() as *mut TaskState) };

        match event0 {
            EVENT_NONE => {}
            EVENT_SUBTASK => match event2 {
                STATUS_STARTING => unreachable!(),
                STATUS_STARTED => {}
                STATUS_RETURNED => {
                    waitable_join(event1, 0);
                    subtask_drop(event1);

                    let Promise::ImportCall {
                        index,
                        buffer,
                        call,
                        ..
                    } = state.pending.get_mut(event1).unwrap()
                    else {
                        unreachable!()
                    };

                    let func = WIT
                        .get()
                        .unwrap()
                        .import_func(usize::try_from(index).unwrap());

                    unsafe { func.lift_import_async_result(call, buffer) };
                    assert!(call.stack.len() < 2);

                    with_context(|cx| {
                        let Promise::ImportCall { call, resolve, .. } =
                            state.pending.remove(event1).unwrap()
                        else {
                            unreachable!()
                        };

                        rooted!(&in(cx) let resolve = resolve.get());
                        rooted!(&in(cx) let result = UndefinedValue());
                        if func.result.is_some() {
                            result.set(call.stack.try_lock().unwrap().pop().unwrap().get());
                        }
                        rooted!(&in(cx) let params = vec![result.get()]);
                        rooted!(&in(cx) let object = UndefinedValue());
                        rooted!(&in(cx) let mut result = UndefinedValue());
                        if !unsafe {
                            JS_CallFunctionValue(
                                cx,
                                object.handle(),
                                resolve.handle(),
                                &HandleValueArray::from(&params),
                                result.handle_mut(),
                            )
                        } {
                            unsafe { PrintAndClearException(cx.raw_cx()) }
                            panic!("JS_CallFunctionValue failed for `{}`", name())
                        }
                    });
                }
                _ => todo!(),
            },
            _ => todo!(),
        }

        with_context(poll)
    }

    fn resource_dtor(ty: wit::Resource, handle: usize) {
        _ = (ty, handle);
        todo!()
    }
}

#[expect(dead_code, clippy::vec_box)]
struct MyCall<'a> {
    _phantom: PhantomData<&'a ()>,
    iter_stack: Vec<usize>,
    deferred_deallocations: Vec<(*mut u8, Layout)>,
    strings: Vec<String>,
    borrows: Vec<Borrow>,
    stack: Arc<Mutex<Vec<Box<Heap<Value>>>>>,
    resources: Option<Vec<EmptyResource>>,
}

impl MyCall<'_> {
    fn new() -> Self {
        let stack = Arc::new(Mutex::new(Vec::new()));
        assert!(STACKS.try_lock().unwrap().insert(ArcHash(stack.clone())));
        Self {
            _phantom: PhantomData,
            iter_stack: Vec::new(),
            deferred_deallocations: Vec::new(),
            strings: Vec::new(),
            borrows: Vec::new(),
            stack,
            resources: None,
        }
    }
}

impl Drop for MyCall<'_> {
    fn drop(&mut self) {
        assert!(
            STACKS
                .try_lock()
                .unwrap()
                .remove(&ArcHash(self.stack.clone()))
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
        todo!()
    }

    fn pop_u16(&mut self) -> u16 {
        todo!()
    }

    fn pop_u32(&mut self) -> u32 {
        self.stack
            .try_lock()
            .unwrap()
            .pop()
            .unwrap()
            .get()
            .to_int32() as u32
    }

    fn pop_u64(&mut self) -> u64 {
        todo!()
    }

    fn pop_s8(&mut self) -> i8 {
        todo!()
    }

    fn pop_s16(&mut self) -> i16 {
        todo!()
    }

    fn pop_s32(&mut self) -> i32 {
        todo!()
    }

    fn pop_s64(&mut self) -> i64 {
        todo!()
    }

    fn pop_bool(&mut self) -> bool {
        todo!()
    }

    fn pop_char(&mut self) -> char {
        todo!()
    }

    fn pop_f32(&mut self) -> f32 {
        todo!()
    }

    fn pop_f64(&mut self) -> f64 {
        todo!()
    }

    fn pop_string(&mut self) -> &str {
        todo!()
    }

    fn pop_borrow(&mut self, ty: wit::Resource) -> u32 {
        _ = ty;
        todo!()
    }

    fn pop_own(&mut self, ty: wit::Resource) -> u32 {
        _ = ty;
        todo!()
    }

    fn pop_enum(&mut self, _ty: wit::Enum) -> u32 {
        todo!()
    }

    fn pop_flags(&mut self, _ty: wit::Flags) -> u32 {
        todo!()
    }

    fn pop_future(&mut self, _ty: wit::Future) -> u32 {
        todo!()
    }

    fn pop_stream(&mut self, _ty: wit::Stream) -> u32 {
        todo!()
    }

    fn pop_option(&mut self, ty: WitOption) -> u32 {
        _ = ty;
        todo!()
    }

    fn pop_result(&mut self, ty: WitResult) -> u32 {
        _ = ty;
        todo!()
    }

    fn pop_variant(&mut self, ty: wit::Variant) -> u32 {
        _ = ty;
        todo!()
    }

    fn pop_record(&mut self, ty: wit::Record) {
        _ = ty;
        todo!()
    }

    fn pop_tuple(&mut self, ty: wit::Tuple) {
        _ = ty;
        todo!()
    }

    unsafe fn maybe_pop_list(&mut self, ty: List) -> Option<(*const u8, usize)> {
        _ = ty;
        todo!()
    }

    fn pop_list(&mut self, _ty: List) -> usize {
        todo!()
    }

    fn pop_iter_next(&mut self, _ty: List) {
        todo!()
    }

    fn pop_iter(&mut self, _ty: List) {
        todo!()
    }

    fn push_bool(&mut self, val: bool) {
        _ = val;
        todo!()
    }

    fn push_char(&mut self, val: char) {
        _ = val;
        todo!()
    }

    fn push_u8(&mut self, val: u8) {
        _ = val;
        todo!()
    }

    fn push_s8(&mut self, val: i8) {
        _ = val;
        todo!()
    }

    fn push_u16(&mut self, val: u16) {
        _ = val;
        todo!()
    }

    fn push_s16(&mut self, val: i16) {
        _ = val;
        todo!()
    }

    fn push_u32(&mut self, val: u32) {
        self.stack
            .try_lock()
            .unwrap()
            .push(Heap::boxed(UInt32Value(val)));
    }

    fn push_s32(&mut self, val: i32) {
        _ = val;
        todo!()
    }

    fn push_u64(&mut self, val: u64) {
        _ = val;
        todo!()
    }

    fn push_s64(&mut self, val: i64) {
        _ = val;
        todo!()
    }

    fn push_f32(&mut self, val: f32) {
        _ = val;
        todo!()
    }

    fn push_f64(&mut self, val: f64) {
        _ = val;
        todo!()
    }

    fn push_string(&mut self, val: String) {
        _ = val;
        todo!()
    }

    fn push_record(&mut self, ty: wit::Record) {
        _ = ty;
        todo!()
    }

    fn push_tuple(&mut self, ty: wit::Tuple) {
        _ = ty;
        todo!()
    }

    fn push_flags(&mut self, ty: wit::Flags, bits: u32) {
        _ = (ty, bits);
        todo!()
    }

    fn push_enum(&mut self, ty: wit::Enum, discriminant: u32) {
        _ = (ty, discriminant);
        todo!()
    }

    fn push_borrow(&mut self, ty: wit::Resource, handle: u32) {
        _ = (ty, handle);
        todo!()
    }

    fn push_own(&mut self, ty: wit::Resource, handle: u32) {
        _ = (ty, handle);
        todo!()
    }

    fn push_future(&mut self, ty: wit::Future, handle: u32) {
        _ = (ty, handle);
        todo!()
    }

    fn push_stream(&mut self, ty: wit::Stream, handle: u32) {
        _ = (ty, handle);
        todo!()
    }

    fn push_variant(&mut self, ty: wit::Variant, discriminant: u32) {
        _ = (ty, discriminant);
        todo!()
    }

    fn push_option(&mut self, ty: WitOption, is_some: bool) {
        _ = (ty, is_some);
        todo!()
    }

    fn push_result(&mut self, ty: WitResult, is_err: bool) {
        _ = (ty, is_err);
        todo!()
    }

    unsafe fn push_raw_list(&mut self, ty: List, src: *mut u8, len: usize) -> bool {
        _ = (ty, src, len);
        todo!()
    }

    fn push_list(&mut self, _ty: List, _capacity: usize) {
        todo!()
    }

    fn list_append(&mut self, _ty: List) {
        todo!()
    }
}

wit_dylib_ffi::export!(MyInterpreter);

unsafe extern "C" fn trace_roots(tracer: *mut JSTracer, _: *mut c_void) {
    unsafe {
        CallObjectTracer(
            tracer,
            GLOBAL_OBJECT
                .try_lock()
                .unwrap()
                .as_mut()
                .unwrap()
                .0
                .ptr
                .get() as *mut _,
            GCTraceKindToAscii(TraceKind::Object),
        )
    }

    for stack in STACKS.try_lock().unwrap().iter() {
        for value in stack.0.try_lock().unwrap().iter_mut() {
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
