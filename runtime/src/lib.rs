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
            CreateRustJSPrincipals, DestroyRustJSPrincipals, JSPrincipalsCallbacks,
            PrintAndClearException,
        },
        jsapi::{
            HandleValueArray, JS_HoldPrincipals, JSCLASS_GLOBAL_FLAGS, JSClass, JSClassOps,
            JSObject, OnNewGlobalHookOption, Value,
        },
        jsval::{UInt32Value, UndefinedValue},
        realm::AutoRealm,
        rooted,
        rust::{
            self, CompileOptionsWrapper, JSEngine, RealmOptions, Runtime,
            wrappers2::{
                CurrentGlobalOrNull, Evaluate, JS_CallFunctionValue, JS_GetProperty,
                JS_InitDestroyPrincipalsCallback, JS_NewGlobalObject, JS_ValueToObject,
            },
        },
    },
    std::{
        alloc::{self, Layout},
        ffi::{CString, c_char},
        marker::PhantomData,
        mem,
        ptr::{self, NonNull},
        sync::{Mutex, OnceLock},
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

static RUNTIME: Mutex<Option<SyncSend<Runtime>>> = Mutex::new(None);
static GLOBAL_OBJECT: OnceLock<SyncSend<NonNull<JSObject>>> = OnceLock::new();

fn make_runtime() -> anyhow::Result<Runtime> {
    let engine = JSEngine::init()
        .map_err(|e| anyhow!("{e:?}"))
        .context("JSEngine::init failed")?;

    let mut runtime = Runtime::new(engine.handle());

    mem::forget(engine);

    let cx = runtime.cx();

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

    let global_object = NonNull::new(unsafe {
        JS_NewGlobalObject(
            cx,
            global_class,
            principals,
            OnNewGlobalHookOption::DontFireOnNewGlobalHook,
            &*realm_options,
        )
    })
    .unwrap();

    GLOBAL_OBJECT
        .set(SyncSend(global_object))
        .map_err(drop)
        .unwrap();

    Ok(runtime)
}

fn with_context<T: 'static>(fun: impl FnOnce(&mut JSContext) -> T) -> T {
    let mut runtime = RUNTIME.lock().unwrap();
    if runtime.is_none() {
        *runtime = Some(SyncSend(make_runtime().unwrap()));
    }

    let runtime = &mut runtime.as_mut().unwrap().0;
    let mut realm = AutoRealm::new(runtime.cx(), GLOBAL_OBJECT.get().unwrap().0);
    fun(&mut realm)
}

fn init(script: &str) -> anyhow::Result<()> {
    with_context(|cx| {
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
        if async_ {
            todo!()
        }

        let name = || {
            if let Some(interface) = func.interface() {
                format!("{interface}#{}", func.name())
            } else {
                func.name().into()
            }
        };

        with_context(|cx| {
            rooted!(&in(cx) let global_object = unsafe { CurrentGlobalOrNull(cx) });
            rooted!(&in(cx) let mut object = ptr::null_mut::<JSObject>());

            if let Some(interface) = func.interface() {
                rooted!(&in(cx) let mut value = UndefinedValue());
                if !unsafe {
                    JS_GetProperty(
                        cx,
                        global_object.handle(),
                        CString::new(interface.replace([':', '/', '-'], "_"))
                            .unwrap()
                            .as_bytes_with_nul()
                            .as_ptr() as *const c_char,
                        value.handle_mut(),
                    )
                } {
                    unsafe { PrintAndClearException(cx.raw_cx()) }
                    panic!("JS_GetProperty failed for {}", name())
                }
                if !unsafe { JS_ValueToObject(cx, value.handle(), object.handle_mut()) } {
                    unsafe { PrintAndClearException(cx.raw_cx()) }
                    panic!("JS_ValueToObject failed for {}", name())
                }
            } else {
                object.set(global_object.get());
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
                panic!("JS_GetProperty failed for {}", name())
            }

            rooted!(&in(cx) let params = mem::take(&mut call.stack));
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
                panic!("JS_CallFunctionValue failed for {}", name())
            }

            if func.result().is_some() {
                call.stack.push(result.get());
            }

            0
        })
    }
}

impl Interpreter for MyInterpreter {
    type CallCx<'a> = MyCall<'a>;

    fn initialize(wit: Wit) {
        WIT.set(wit).map_err(drop).unwrap();
    }

    fn export_start<'a>(_: Wit, _: ExportFunction) -> Box<MyCall<'a>> {
        Box::new(MyCall::new(Vec::new()))
    }

    fn export_call(_: Wit, func: ExportFunction, cx: &mut MyCall<'_>) {
        Self::export_call_(func, cx, false);
    }

    fn export_async_start(_: Wit, func: ExportFunction, mut cx: Box<MyCall<'_>>) -> u32 {
        Self::export_call_(func, &mut cx, true)
    }

    fn export_async_callback(event0: u32, event1: u32, event2: u32) -> u32 {
        _ = (event0, event1, event2);
        todo!()
    }

    fn resource_dtor(ty: wit::Resource, handle: usize) {
        // We don't currently include a `drop` function as part of the abstract
        // base class we generate for an exported resource, so there's nothing
        // to do here.  If/when that changes, we'll want to call `drop` here.
        _ = (ty, handle);
    }
}

#[expect(dead_code)]
struct MyCall<'a> {
    _phantom: PhantomData<&'a ()>,
    iter_stack: Vec<usize>,
    deferred_deallocations: Vec<(*mut u8, Layout)>,
    strings: Vec<String>,
    borrows: Vec<Borrow>,
    stack: Vec<Value>,
    resources: Option<Vec<EmptyResource>>,
}

impl MyCall<'_> {
    fn new(stack: Vec<Value>) -> Self {
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
        self.stack.pop().unwrap().to_int32() as u32
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
        self.stack.push(UInt32Value(val));
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

// As of this writing, recent Rust `nightly` builds include a version of the
// `libc` crate that expects `wasi-libc` to define the following global
// variables, but `wasi-libc` defines them as preprocessor constants which
// aren't visible at link time, so we need to define them somewhere.  Ideally,
// we should fix this upstream, but for now we work around it:

#[unsafe(no_mangle)]
static _CLOCK_PROCESS_CPUTIME_ID: u8 = 2;
#[unsafe(no_mangle)]
static _CLOCK_THREAD_CPUTIME_ID: u8 = 3;
