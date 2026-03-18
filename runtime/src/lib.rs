#![deny(warnings)]
#![expect(
    clippy::not_unsafe_ptr_arg_deref,
    reason = "wit_dylib_ffi::export produces code that does this"
)]

use {
    anyhow::{Context as _, anyhow, bail},
    heck::{ToLowerCamelCase as _, ToUpperCamelCase as _},
    mozjs::{
        context::JSContext,
        conversions::{Utf8Chars, jsstr_to_string},
        gc::Handle,
        glue::{
            CallObjectTracer, CallValueTracer, CreateRustJSPrincipals, DestroyRustJSPrincipals,
            GetBigInt64ArrayLengthAndData, GetBigUint64ArrayLengthAndData, JSPrincipalsCallbacks,
            PrintAndClearException, RUST_SYMBOL_TO_JSID,
        },
        jsapi::{
            ExceptionStackBehavior, GCTraceKindToAscii, Handle as RawHandle, HandleValueArray,
            Heap, JS_CallArgsFromVp, JS_GetFunctionObject, JS_HoldPrincipals, JSAutoRealm,
            JSCLASS_GLOBAL_FLAGS, JSClass, JSClassOps, JSContext as RawJSContext, JSObject,
            JSTracer, ModuleErrorBehaviour, OnNewGlobalHookOption, PromiseState, PropertyKey,
            SetModuleResolveHook, SymbolCode, TraceKind, Value,
        },
        jsval::{
            BigIntValue, BooleanValue, DoubleValue, Int32Value, ObjectValue, StringValue,
            UInt32Value, UndefinedValue,
        },
        rooted,
        rust::{
            self, CompileOptionsWrapper, JSEngine, RealmOptions, Runtime, ToString,
            wrappers2::{
                BigIntFromInt64, BigIntFromUint64, BigIntToString, CompileModule1, Construct1,
                CurrentGlobalOrNull, Evaluate2, GetModuleRequestSpecifier, GetPromiseState,
                GetWellKnownSymbol, InitRealmStandardClasses, IsPromiseObject,
                JS_AddExtraGCRootsTracer, JS_CallFunctionValue, JS_ClearPendingException,
                JS_DeleteProperty1, JS_GetElement, JS_GetPendingException, JS_GetProperty,
                JS_InitDestroyPrincipalsCallback, JS_IsExceptionPending, JS_NewBigInt64Array,
                JS_NewBigUint64Array, JS_NewFunction, JS_NewGlobalObject, JS_NewObject,
                JS_NewObjectWithGivenProto, JS_NewStringCopyUTF8N, JS_SetElement,
                JS_SetPendingException, JS_SetProperty, JS_SetPropertyById, ModuleEvaluate,
                ModuleLink, NewArrayObject, NewArrayObject1, NewPromiseObject, ResolvePromise,
                RunJobs, ThrowOnModuleEvaluationFailure,
            },
        },
        typedarray::{
            CreateWith, Float32, Float32Array, Float64, Float64Array, Int8, Int8Array, Int16,
            Int16Array, Int32, Int32Array, TypedArrayElement as _, Uint8, Uint8Array, Uint16,
            Uint16Array, Uint32, Uint32Array,
        },
    },
    std::{
        alloc::{self, Layout},
        collections::{HashMap, HashSet},
        ffi::{CStr, CString, c_char, c_void},
        fs,
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
    fn subtask_drop(task: u32);
}
#[link(wasm_import_module = "$root")]
unsafe extern "C" {
    #[link_name = "[waitable-set-new]"]
    fn waitable_set_new() -> u32;
}
#[link(wasm_import_module = "$root")]
unsafe extern "C" {
    #[link_name = "[waitable-join]"]
    fn waitable_join(waitable: u32, set: u32);
}
#[link(wasm_import_module = "$root")]
unsafe extern "C" {
    #[link_name = "[waitable-set-drop]"]
    fn waitable_set_drop(set: u32);
}
#[link(wasm_import_module = "$root")]
unsafe extern "C" {
    #[link_name = "[context-get-0]"]
    fn context_get() -> u32;
}
#[link(wasm_import_module = "$root")]
unsafe extern "C" {
    #[link_name = "[context-set-0]"]
    fn context_set(value: u32);
}

const EVENT_NONE: u32 = 0;
const EVENT_SUBTASK: u32 = 1;
const EVENT_STREAM_READ: u32 = 2;
const EVENT_STREAM_WRITE: u32 = 3;
const EVENT_FUTURE_READ: u32 = 4;
const EVENT_FUTURE_WRITE: u32 = 5;
const EVENT_CANCELLED: u32 = 6;

const STATUS_STARTING: u32 = 0;
const STATUS_STARTED: u32 = 1;
const STATUS_RETURNED: u32 = 2;

const CALLBACK_CODE_EXIT: u32 = 0;
const CALLBACK_CODE_WAIT: u32 = 2;

const RETURN_CODE_BLOCKED: u32 = 0xFFFF_FFFF;
const RETURN_CODE_COMPLETED: u32 = 0x0;
const RETURN_CODE_DROPPED: u32 = 0x1;
const RETURN_CODE_CANCELLED: u32 = 0x2;

const HANDLE_FIELD_NAME: &CStr = c"_componentizeJsHandle";
const TYPE_FIELD_NAME: &CStr = c"_componentizeJsType";

struct Borrow {
    value: Box<Heap<*mut JSObject>>,
    handle: u32,
    drop: unsafe extern "C" fn(u32),
}

struct EmptyResource {
    value: Box<Heap<*mut JSObject>>,
    handle: u32,
}

struct TransmitTraced {
    wrapper: Box<Heap<*mut JSObject>>,
    promise: Box<Heap<*mut JSObject>>,
    resources: Option<Vec<Vec<EmptyResource>>>,
}

impl TransmitTraced {
    #[expect(clippy::arc_with_non_send_sync)]
    fn new(
        wrapper: *mut JSObject,
        promise: *mut JSObject,
        resources: Option<Vec<Vec<EmptyResource>>>,
    ) -> Arc<Mutex<Self>> {
        let traced = Arc::new(Mutex::new(Self {
            wrapper: Heap::boxed(wrapper),
            promise: Heap::boxed(promise),
            resources,
        }));
        assert!(
            TRANSMIT_TRACED
                .try_lock()
                .unwrap()
                .0
                .insert(ArcHash(traced.clone()))
        );
        traced
    }
}

enum Pending {
    ImportCall {
        index: usize,
        call: MyCall<'static>,
        buffer: *mut u8,
    },
    StreamWrite {
        _call: MyCall<'static>,
        traced: Arc<Mutex<TransmitTraced>>,
    },
    StreamRead {
        call: MyCall<'static>,
        buffer: *mut u8,
        traced: Arc<Mutex<TransmitTraced>>,
        index: usize,
    },
    FutureWrite {
        _call: MyCall<'static>,
        traced: Arc<Mutex<TransmitTraced>>,
    },
    FutureRead {
        call: MyCall<'static>,
        buffer: *mut u8,
        traced: Arc<Mutex<TransmitTraced>>,
        index: usize,
    },
}

impl Drop for Pending {
    fn drop(&mut self) {
        match self {
            Self::ImportCall { .. } => {}
            Self::StreamWrite { traced, .. }
            | Self::StreamRead { traced, .. }
            | Self::FutureWrite { traced, .. }
            | Self::FutureRead { traced, .. } => {
                assert!(
                    TRANSMIT_TRACED
                        .try_lock()
                        .unwrap()
                        .0
                        .remove(&ArcHash(traced.clone()))
                );
            }
        }
    }
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

type MyCallTracedSet = HashSet<ArcHash<Mutex<MyCallTraced>>, BuildHasherDefault<DefaultHasher>>;
type TransmitTracedSet = HashSet<ArcHash<Mutex<TransmitTraced>>, BuildHasherDefault<DefaultHasher>>;
type ModuleMap = HashMap<String, Box<Heap<*mut JSObject>>, BuildHasherDefault<DefaultHasher>>;

static WIT: OnceLock<Wit> = OnceLock::new();
static CONTEXT: OnceLock<SyncSend<NonNull<RawJSContext>>> = OnceLock::new();
static MY_CALL_TRACED: Mutex<SyncSend<MyCallTracedSet>> =
    Mutex::new(SyncSend(HashSet::with_hasher(BuildHasherDefault::new())));
static TRANSMIT_TRACED: Mutex<SyncSend<TransmitTracedSet>> =
    Mutex::new(SyncSend(HashSet::with_hasher(BuildHasherDefault::new())));
static CURRENT_TASK_STATE: Mutex<Option<SyncSend<TaskState>>> = Mutex::new(None);
static EXPORTED_RESOURCES: Mutex<SyncSend<Table<Box<Heap<*mut JSObject>>>>> =
    Mutex::new(SyncSend(Table::new()));
static MODULES: Mutex<SyncSend<ModuleMap>> =
    Mutex::new(SyncSend(HashMap::with_hasher(BuildHasherDefault::new())));
static MAIN_MODULE: Mutex<Option<SyncSend<Box<Heap<*mut JSObject>>>>> = Mutex::new(None);

fn init_runtime() -> anyhow::Result<()> {
    let engine = JSEngine::init()
        .map_err(|e| anyhow!("{e:?}"))
        .context("JSEngine::init failed")?;

    let mut runtime = Runtime::new(engine.handle());

    mem::forget(engine);

    // TODO: Also call `SetModuleDynamicImportHook`
    unsafe {
        SetModuleResolveHook(runtime.rt(), Some(resolve_import));
    }

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

    mem::forget(JSAutoRealm::new(
        unsafe { cx.raw_cx_no_gc() },
        global_object,
    ));

    if !unsafe { InitRealmStandardClasses(cx) } {
        bail!("InitRealmStandardClasses failed")
    }

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

fn get(cx: &mut JSContext, object: Handle<'_, *mut JSObject>, name: &CStr) -> Value {
    rooted!(&in(cx) let mut value = UndefinedValue());
    // TODO: Is there a quicker way to get the array length, e.g. using
    // `JS_GetPropertyById`?
    if !unsafe {
        JS_GetProperty(
            cx,
            object,
            name.as_ptr() as *const c_char,
            value.handle_mut(),
        )
    } {
        unsafe { PrintAndClearException(cx.raw_cx()) }
        panic!("JS_GetProperty failed for `{}`", name.to_str().unwrap())
    }
    value.get()
}

fn set(
    cx: &mut JSContext,
    object: Handle<'_, *mut JSObject>,
    name: &CStr,
    value: Handle<'_, Value>,
) {
    if !unsafe { JS_SetProperty(cx, object, name.as_ptr() as *const c_char, value) } {
        unsafe { PrintAndClearException(cx.raw_cx()) }
        panic!("JS_SetProperty failed for `{}`", name.to_str().unwrap())
    }
}

fn set_with_symbol(
    cx: &mut JSContext,
    object: Handle<'_, *mut JSObject>,
    code: SymbolCode,
    value: Handle<'_, Value>,
) {
    rooted!(&in(cx) let symbol = unsafe { GetWellKnownSymbol(cx, code) });
    rooted!(&in(cx) let mut key = PropertyKey::default());
    unsafe { RUST_SYMBOL_TO_JSID(symbol.get(), key.handle_mut().into()) }
    if !unsafe { JS_SetPropertyById(cx, object, key.handle(), value) } {
        unsafe { PrintAndClearException(cx.raw_cx()) }
        panic!("JS_SetPropertyById failed")
    }
}

fn get_element(cx: &mut JSContext, object: Handle<'_, *mut JSObject>, index: u32) -> Value {
    rooted!(&in(cx) let mut value = UndefinedValue());
    if !unsafe { JS_GetElement(cx, object, index, value.handle_mut()) } {
        unsafe { PrintAndClearException(cx.raw_cx()) }
        panic!("JS_GetElement failed for `{index}`")
    }
    value.get()
}

fn set_element(
    cx: &mut JSContext,
    object: Handle<'_, *mut JSObject>,
    index: u32,
    value: Handle<'_, Value>,
) {
    if !unsafe { JS_SetElement(cx, object, index, value) } {
        unsafe { PrintAndClearException(cx.raw_cx()) }
        panic!("JS_SetElement failed for `{index}`")
    }
}

fn delete(cx: &mut JSContext, object: Handle<'_, *mut JSObject>, name: &CStr) {
    if !unsafe { JS_DeleteProperty1(cx, object, name.as_ptr() as *const c_char) } {
        unsafe { PrintAndClearException(cx.raw_cx()) }
        panic!("JS_DeleteProperty failed for `{}`", name.to_str().unwrap())
    }
}

fn call(
    cx: &mut JSContext,
    object: Handle<'_, *mut JSObject>,
    fun: Handle<'_, Value>,
    args: &HandleValueArray,
) -> Value {
    rooted!(&in(cx) let mut result = UndefinedValue());
    if !unsafe { JS_CallFunctionValue(cx, object, fun, args, result.handle_mut()) } {
        unsafe { PrintAndClearException(cx.raw_cx()) }
        panic!("JS_CallFunctionValue failed")
    }
    result.get()
}

fn wrap(cx: &mut JSContext, fun: JsFunction) -> Value {
    ObjectValue(unsafe {
        JS_GetFunctionObject(JS_NewFunction(
            cx,
            Some(fun),
            // TODO: how is this argument used, if at all?
            0,
            0,
            ptr::null(),
        ))
    })
}

fn resolve(cx: &mut JSContext, promise: Handle<'_, *mut JSObject>, value: Handle<'_, Value>) {
    if !unsafe { ResolvePromise(cx, promise, value) } {
        unsafe { PrintAndClearException(cx.raw_cx()) }
        panic!("ResolvePromise failed")
    }
}

fn register_resource(cx: &mut JSContext, value: Handle<'_, *mut JSObject>, handle: u32) {
    rooted!(&in(cx) let handle = UInt32Value(handle));
    set(cx, value, HANDLE_FIELD_NAME, handle.handle());

    rooted!(&in(cx) let global_object = unsafe { CurrentGlobalOrNull(cx) });
    rooted!(&in(cx) let register = get(cx, global_object.handle(), c"_componentizeJsRegisterFinalizer"));
    rooted!(&in(cx) let params = vec![ObjectValue(value.get())]);
    call(
        cx,
        global_object.handle(),
        register.handle(),
        &HandleValueArray::from(&params),
    );
}

fn unregister_resource(cx: &mut JSContext, value: Handle<'_, *mut JSObject>) {
    rooted!(&in(cx) let global_object = unsafe { CurrentGlobalOrNull(cx) });
    rooted!(&in(cx) let unregister = get(cx, global_object.handle(), c"_componentizeJsUnregisterFinalizer"));
    rooted!(&in(cx) let params = vec![ObjectValue(value.get())]);
    call(
        cx,
        global_object.handle(),
        unregister.handle(),
        &HandleValueArray::from(&params),
    );

    delete(cx, value, HANDLE_FIELD_NAME);
}

fn release_borrows(cx: &mut JSContext, traced: &Mutex<MyCallTraced>) {
    // Note that we're careful here to leave all but the current borrow in
    // `traced` (and immediately root the `Borrow::value` before doing anything
    // else with the current borrow) to ensure the others remain visible and
    // update-able to the GC.

    while let Some(Borrow {
        value,
        handle,
        drop,
    }) = traced.try_lock().unwrap().borrows.pop()
    {
        rooted!(&in(cx) let value = value.get());
        unregister_resource(cx, value.handle());

        unsafe {
            drop(handle);
        }
    }
}

fn restore_resources(cx: &mut JSContext, traced: &Mutex<TransmitTraced>, count: u32) {
    // Note that we're careful here to leave all but the current
    // resource in `traced` (and immediately root the GC-able fields
    // before doing anything else with the current resource) to
    // ensure the others remain visible and update-able to the GC.

    let pop_resource = || {
        if let Some(resources) = traced.try_lock().unwrap().resources.as_mut() {
            let count = usize::try_from(count).unwrap();
            while resources.len() > count {
                let last = resources.last_mut().unwrap();
                let last_last = last.pop();
                if last_last.is_some() {
                    return last_last;
                } else {
                    resources.pop();
                }
            }
        }
        None
    };

    while let Some(resource) = pop_resource() {
        rooted!(&in(cx) let wrapper = resource.value.get());
        register_resource(cx, wrapper.handle(), resource.handle);
    }
}

unsafe fn create_typed_array(
    cx: &mut JSContext,
    ty: Type,
    buffer: *const u8,
    count: usize,
) -> Value {
    rooted!(&in(cx) let mut array = ptr::null_mut::<JSObject>());
    unsafe {
        match ty {
            Type::U8 => Uint8Array::create(
                cx.raw_cx(),
                CreateWith::Slice(slice::from_raw_parts(buffer, count)),
                array.handle_mut(),
            ),
            Type::S8 => Int8Array::create(
                cx.raw_cx(),
                CreateWith::Slice(slice::from_raw_parts(buffer.cast(), count)),
                array.handle_mut(),
            ),
            Type::U16 => Uint16Array::create(
                cx.raw_cx(),
                CreateWith::Slice(slice::from_raw_parts(buffer.cast(), count)),
                array.handle_mut(),
            ),
            Type::S16 => Int16Array::create(
                cx.raw_cx(),
                CreateWith::Slice(slice::from_raw_parts(buffer.cast(), count)),
                array.handle_mut(),
            ),
            Type::U32 => Uint32Array::create(
                cx.raw_cx(),
                CreateWith::Slice(slice::from_raw_parts(buffer.cast(), count)),
                array.handle_mut(),
            ),
            Type::S32 => Int32Array::create(
                cx.raw_cx(),
                CreateWith::Slice(slice::from_raw_parts(buffer.cast(), count)),
                array.handle_mut(),
            ),
            Type::U64 => {
                // As of this writing, `mozjs::typedarray::BigUint64Array` does
                // not yet exist, so we have use lower-level APIs.
                array.set(JS_NewBigUint64Array(cx, count));
                let mut length = 0;
                let mut is_shared_memory = false;
                let mut data = ptr::null_mut();
                GetBigUint64ArrayLengthAndData(
                    array.get(),
                    &mut length,
                    &mut is_shared_memory,
                    &mut data,
                );
                assert_eq!(length, count);
                ptr::copy_nonoverlapping(buffer.cast(), data, count);
                Ok(())
            }
            Type::S64 => {
                // As of this writing, `mozjs::typedarray::BigInt64Array` does
                // not yet exist, so we have use lower-level APIs.
                array.set(JS_NewBigInt64Array(cx, count));
                let mut length = 0;
                let mut is_shared_memory = false;
                let mut data = ptr::null_mut();
                GetBigInt64ArrayLengthAndData(
                    array.get(),
                    &mut length,
                    &mut is_shared_memory,
                    &mut data,
                );
                assert_eq!(length, count);
                ptr::copy_nonoverlapping(buffer.cast(), data, count);
                Ok(())
            }
            Type::F32 => Float32Array::create(
                cx.raw_cx(),
                CreateWith::Slice(slice::from_raw_parts(buffer.cast(), count)),
                array.handle_mut(),
            ),
            Type::F64 => Float64Array::create(
                cx.raw_cx(),
                CreateWith::Slice(slice::from_raw_parts(buffer.cast(), count)),
                array.handle_mut(),
            ),
            _ => unreachable!(),
        }
        .unwrap()
    }
    ObjectValue(array.get())
}

unsafe fn typed_array_data(ty: Type, array: *mut JSObject) -> (*mut u8, usize, Layout) {
    unsafe {
        match ty {
            Type::U8 => {
                let (data, length) = Uint8::length_and_data(array);
                (
                    data.cast(),
                    length,
                    Layout::from_size_align(length, 1).unwrap(),
                )
            }
            Type::S8 => {
                let (data, length) = Int8::length_and_data(array);
                (
                    data.cast(),
                    length,
                    Layout::from_size_align(length, 1).unwrap(),
                )
            }
            Type::U16 => {
                let (data, length) = Uint16::length_and_data(array);
                (
                    data.cast(),
                    length,
                    Layout::from_size_align(length * 2, 2).unwrap(),
                )
            }
            Type::S16 => {
                let (data, length) = Int16::length_and_data(array);
                (
                    data.cast(),
                    length,
                    Layout::from_size_align(length * 2, 2).unwrap(),
                )
            }
            Type::U32 => {
                let (data, length) = Uint32::length_and_data(array);
                (
                    data.cast(),
                    length,
                    Layout::from_size_align(length * 4, 4).unwrap(),
                )
            }
            Type::S32 => {
                let (data, length) = Int32::length_and_data(array);
                (
                    data.cast(),
                    length,
                    Layout::from_size_align(length * 4, 4).unwrap(),
                )
            }
            Type::U64 => {
                // As of this writing, `mozjs::typedarray::BigUint64` does not
                // yet exist, so we have use a lower-level API.
                let mut length = 0;
                let mut is_shared_memory = false;
                let mut data = ptr::null_mut();
                GetBigUint64ArrayLengthAndData(
                    array,
                    &mut length,
                    &mut is_shared_memory,
                    &mut data,
                );
                (
                    data.cast(),
                    length,
                    Layout::from_size_align(length * 8, 8).unwrap(),
                )
            }
            Type::S64 => {
                // As of this writing, `mozjs::typedarray::BigInt64` does not
                // yet exist, so we have use a lower-level API.
                let mut length = 0;
                let mut is_shared_memory = false;
                let mut data = ptr::null_mut();
                GetBigInt64ArrayLengthAndData(array, &mut length, &mut is_shared_memory, &mut data);
                (
                    data.cast(),
                    length,
                    Layout::from_size_align(length * 8, 8).unwrap(),
                )
            }
            Type::F32 => {
                let (data, length) = Float32::length_and_data(array);
                (
                    data.cast(),
                    length,
                    Layout::from_size_align(length * 4, 4).unwrap(),
                )
            }
            Type::F64 => {
                let (data, length) = Float64::length_and_data(array);
                (
                    data.cast(),
                    length,
                    Layout::from_size_align(length * 8, 8).unwrap(),
                )
            }
            _ => unreachable!(),
        }
    }
}

fn handle_import_result(
    cx: &mut JSContext,
    call: &mut MyCall<'_>,
    ty: Option<Type>,
) -> Result<Option<Value>, Value> {
    match ty {
        Some(Type::Result(ty)) => {
            rooted!(&in(cx) let wrapper = call.pop().to_object());
            let tag = unsafe {
                jsstr_to_string(
                    cx.raw_cx(),
                    NonNull::new(get(cx, wrapper.handle(), c"tag").to_string()).unwrap(),
                )
            };

            match tag.as_str() {
                "ok" => Ok(if ty.ok().is_some() {
                    Some(get(cx, wrapper.handle(), c"val"))
                } else {
                    None
                }),
                "err" => {
                    rooted!(&in(cx) let global_object = unsafe { CurrentGlobalOrNull(cx) });
                    rooted!(&in(cx) let class = get(cx, global_object.handle(), c"ComponentError"));
                    let param = if ty.err().is_some() {
                        get(cx, wrapper.handle(), c"val")
                    } else {
                        UndefinedValue()
                    };
                    rooted!(&in(cx) let mut result = ptr::null_mut::<JSObject>());
                    rooted!(&in(cx) let params = vec![param]);
                    if !unsafe {
                        Construct1(
                            cx,
                            class.handle(),
                            &HandleValueArray::from(&params),
                            result.handle_mut(),
                        )
                    } {
                        unsafe { PrintAndClearException(cx.raw_cx()) }
                        panic!("Construct1 failed")
                    }
                    Err(ObjectValue(result.get()))
                }
                _ => unreachable!(),
            }
        }
        Some(_) => Ok(Some(call.pop())),
        None => Ok(None),
    }
}

unsafe extern "C" fn call_import(cx: *mut RawJSContext, argc: u32, vp: *mut Value) -> bool {
    assert!(argc >= 2);

    let args = unsafe { JS_CallArgsFromVp(argc, vp) };
    let cx = &mut unsafe { JSContext::from_ptr(NonNull::new(cx).unwrap()) };
    let index = args.index(0);
    let params = args.index(1);
    rooted!(&in(cx) let params = params.to_object());
    // TODO: Is there a quicker way to get the array length, e.g. using
    // `JS_GetPropertyById`?
    let length = u32::try_from(get(cx, params.handle(), c"length").to_int32()).unwrap();
    let func = WIT
        .get()
        .unwrap()
        .import_func(usize::try_from(index.to_int32()).unwrap());
    assert_eq!(func.params().len(), usize::try_from(length).unwrap());

    let mut call = MyCall::new();
    for index in 0..length {
        call.push(get_element(cx, params.handle(), length - index - 1));
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
            self::call(
                cx,
                Handle::<*mut JSObject>::null(),
                unsafe { Handle::from_raw(resolve) },
                &HandleValueArray::from(&params),
            );
        }

        args.rval().set(UndefinedValue())
    } else {
        assert_eq!(argc, 2);

        func.call_import_sync(&mut call);

        match handle_import_result(cx, &mut call, func.result()) {
            Ok(value) => args.rval().set(value.unwrap_or_else(UndefinedValue)),
            Err(value) => {
                rooted!(&in(cx) let exception = value);
                unsafe {
                    JS_SetPendingException(
                        cx,
                        exception.handle(),
                        ExceptionStackBehavior::DoNotCapture,
                    )
                };
            }
        }
    }

    true
}

fn handle_export_result(
    cx: &mut JSContext,
    call: &mut MyCall<'_>,
    ty: Option<Type>,
    value: Handle<'_, Value>,
    fulfilled: bool,
) {
    match ty {
        Some(Type::Result(ty)) => {
            rooted!(&in(cx) let mut value = value.get());
            if !fulfilled {
                if !value.is_object() {
                    panic!("caught unexpected exception of non-object type");
                }
                rooted!(&in(cx) let object = value.to_object());
                rooted!(&in(cx) let constructor = get(cx, object.handle(), c"constructor").to_object());
                let name = &unsafe {
                    jsstr_to_string(
                        cx.raw_cx(),
                        NonNull::new(get(cx, constructor.handle(), c"name").to_string()).unwrap(),
                    )
                };
                if "ComponentError" != name {
                    let string = unsafe {
                        jsstr_to_string(
                            cx.raw_cx(),
                            NonNull::new(ToString(cx.raw_cx(), value.handle())).unwrap(),
                        )
                    };
                    panic!(
                        "caught unexpected exception; expected `ComponentError`, got `{string}`"
                    );
                }
                if ty.err().is_some() {
                    value.set(get(cx, object.handle(), c"payload"));
                }
            }

            if (fulfilled && ty.ok().is_some()) || (!fulfilled && ty.err().is_some()) {
                call.push(value.get());
            }
            call.push_result(ty, !fulfilled)
        }
        Some(_) => {
            if !fulfilled {
                panic!("caught unexpected exception for infallible exported function type");
            }
            call.push(value.get());
        }
        None => {}
    }
}

unsafe extern "C" fn call_task_return(cx: *mut RawJSContext, argc: u32, vp: *mut Value) -> bool {
    assert_eq!(argc, 4);

    let args = unsafe { JS_CallArgsFromVp(argc, vp) };
    let cx = &mut unsafe { JSContext::from_ptr(NonNull::new(cx).unwrap()) };
    let index = args.index(0);
    let value = args.index(1);
    let borrows = args.index(2).to_int32();
    let fulfilled = args.index(3).to_boolean();
    let func = WIT
        .get()
        .unwrap()
        .export_func(usize::try_from(index.to_int32()).unwrap());
    let mut call = MyCall::new();

    rooted!(&in(cx) let mut value = value.get());
    handle_export_result(cx, &mut call, func.result(), value.handle(), fulfilled);

    func.call_task_return(&mut call);

    if borrows != 0 {
        release_borrows(
            cx,
            unsafe { Arc::from_raw(borrows as *const Mutex<MyCallTraced>) }.as_ref(),
        );
    }

    args.rval().set(UndefinedValue());
    true
}

unsafe extern "C" fn drop_resource(cx: *mut RawJSContext, argc: u32, vp: *mut Value) -> bool {
    assert_eq!(argc, 0);

    let args = unsafe { JS_CallArgsFromVp(argc, vp) };
    let cx = &mut unsafe { JSContext::from_ptr(NonNull::new(cx).unwrap()) };
    rooted!(&in(cx) let this = args.thisv().to_object());
    let index = get(cx, this.handle(), TYPE_FIELD_NAME);
    let handle = get(cx, this.handle(), HANDLE_FIELD_NAME);

    if index.is_int32() && handle.is_int32() {
        let index = index.to_int32() as u32;
        let handle = handle.to_int32() as u32;
        let ty = WIT.get().unwrap().resource(usize::try_from(index).unwrap());

        unregister_resource(cx, this.handle());

        unsafe { ty.drop()(handle) };
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

unsafe extern "C" fn stream_write(cx: *mut RawJSContext, argc: u32, vp: *mut Value) -> bool {
    assert_eq!(argc, 1);

    let args = unsafe { JS_CallArgsFromVp(argc, vp) };
    let cx = &mut unsafe { JSContext::from_ptr(NonNull::new(cx).unwrap()) };
    rooted!(&in(cx) let this = args.thisv().to_object());
    let index = get(cx, this.handle(), TYPE_FIELD_NAME).to_int32() as u32;
    let handle = get(cx, this.handle(), HANDLE_FIELD_NAME).to_int32() as u32;
    rooted!(&in(cx) let values = args.index(0).to_object());
    let ty = WIT.get().unwrap().stream(usize::try_from(index).unwrap());

    // TODO: unregister resource and then reregister upon (possibly async)
    // completion to prevent concurrent reads.

    let write_count = if let Some(ty) = ty.ty().filter(|&v| use_typed_array(v)) {
        unsafe { typed_array_data(ty, args.index(0).to_object()) }.1
    } else {
        // TODO: Is there a quicker way to get the array length, e.g. using
        // `JS_GetPropertyById`?
        usize::try_from(get(cx, values.handle(), c"length").to_int32()).unwrap()
    };

    let layout =
        Layout::from_size_align(ty.abi_payload_size() * write_count, ty.abi_payload_align())
            .unwrap();
    let buffer = unsafe { std::alloc::alloc(layout) };
    if buffer.is_null() {
        panic!("unable to allocate buffer for stream write");
    }

    let mut call = MyCall::new();
    unsafe { call.defer_deallocate(buffer, layout) };

    rooted!(&in(cx) let promise = unsafe { NewPromiseObject(cx, Handle::<*mut JSObject>::null()) });

    let traced = TransmitTraced::new(this.get(), promise.get(), None);

    if let Some(payload_type) = ty.ty().filter(|&v| use_typed_array(v)) {
        let (data, length, _) =
            unsafe { typed_array_data(payload_type, args.index(0).to_object()) };
        assert_eq!(length, write_count);
        // TODO: Can we avoid the copy here by telling SpiderMonkey to pin the
        // typed array buffer so it can't be moved, collected, or resized until
        // we've unpinned it?
        unsafe { ptr::copy_nonoverlapping(data, buffer, write_count * ty.abi_payload_size()) };
    } else {
        let mut need_restore_resources = false;
        traced.try_lock().unwrap().resources = Some(Vec::with_capacity(write_count));
        for offset in 0..write_count {
            call.push(get_element(
                cx,
                values.handle(),
                u32::try_from(offset).unwrap(),
            ));
            call.traced.try_lock().unwrap().resources = Some(Vec::new());
            unsafe { ty.lower(&mut call, buffer.add(ty.abi_payload_size() * offset)) };
            let res = call.traced.try_lock().unwrap().resources.take().unwrap();
            if !res.is_empty() {
                need_restore_resources = true;
            }
            traced
                .try_lock()
                .unwrap()
                .resources
                .as_mut()
                .unwrap()
                .push(res);
        }

        if !need_restore_resources {
            traced.try_lock().unwrap().resources = None;
        }
    }

    let code = unsafe { ty.write()(handle, buffer.cast(), write_count) };

    if code == RETURN_CODE_BLOCKED {
        let mut state = CURRENT_TASK_STATE.try_lock().unwrap();
        let state = &mut state.as_mut().unwrap().0;
        if state.waitable_set.is_none() {
            state.waitable_set = Some(unsafe { waitable_set_new() });
        }
        unsafe { waitable_join(handle, state.waitable_set.unwrap()) }

        state.pending.insert(
            handle,
            Pending::StreamWrite {
                _call: call,
                traced,
            },
        );
    } else {
        let count = code >> 4;
        let code = code & 0xF;

        restore_resources(cx, &traced, count);

        if code == RETURN_CODE_DROPPED {
            rooted!(&in(cx) let value = BooleanValue(true));
            set(cx, this.handle(), c"readerDropped", value.handle());
        }
        rooted!(&in(cx) let value = UInt32Value(count));
        resolve(cx, promise.handle(), value.handle());
    }

    args.rval().set(ObjectValue(promise.get()));
    true
}

unsafe extern "C" fn stream_drop_writable(
    cx: *mut RawJSContext,
    argc: u32,
    vp: *mut Value,
) -> bool {
    assert_eq!(argc, 0);

    let args = unsafe { JS_CallArgsFromVp(argc, vp) };
    let cx = &mut unsafe { JSContext::from_ptr(NonNull::new(cx).unwrap()) };
    rooted!(&in(cx) let this = args.thisv().to_object());
    let index = get(cx, this.handle(), TYPE_FIELD_NAME);
    let handle = get(cx, this.handle(), HANDLE_FIELD_NAME);

    if index.is_int32() && handle.is_int32() {
        let index = index.to_int32() as u32;
        let handle = handle.to_int32() as u32;
        let ty = WIT.get().unwrap().stream(usize::try_from(index).unwrap());

        unregister_resource(cx, this.handle());

        unsafe { ty.drop_writable()(handle) };
    }

    args.rval().set(UndefinedValue());
    true
}

unsafe extern "C" fn stream_read(cx: *mut RawJSContext, argc: u32, vp: *mut Value) -> bool {
    assert_eq!(argc, 1);

    let args = unsafe { JS_CallArgsFromVp(argc, vp) };
    let cx = &mut unsafe { JSContext::from_ptr(NonNull::new(cx).unwrap()) };
    rooted!(&in(cx) let this = args.thisv().to_object());
    let index = usize::try_from(get(cx, this.handle(), TYPE_FIELD_NAME).to_int32() as u32).unwrap();
    let handle = get(cx, this.handle(), HANDLE_FIELD_NAME).to_int32() as u32;
    let ty = WIT.get().unwrap().stream(index);

    // TODO: unregister resource and then reregister upon (possibly async)
    // completion to prevent concurrent reads.

    let max_count = usize::try_from(args.index(0).to_int32() as u32).unwrap();
    let layout =
        Layout::from_size_align(ty.abi_payload_size() * max_count, ty.abi_payload_align()).unwrap();
    let buffer = unsafe { std::alloc::alloc(layout) };
    if buffer.is_null() {
        panic!("unable to allocate buffer for stream read");
    }

    let mut call = MyCall::new();
    unsafe { call.defer_deallocate(buffer, layout) };

    let code = unsafe { ty.read()(handle, buffer.cast(), max_count) };

    rooted!(&in(cx) let promise = unsafe { NewPromiseObject(cx, Handle::<*mut JSObject>::null()) });

    if code == RETURN_CODE_BLOCKED {
        let mut state = CURRENT_TASK_STATE.try_lock().unwrap();
        let state = &mut state.as_mut().unwrap().0;
        if state.waitable_set.is_none() {
            state.waitable_set = Some(unsafe { waitable_set_new() });
        }
        unsafe { waitable_join(handle, state.waitable_set.unwrap()) }

        state.pending.insert(
            handle,
            Pending::StreamRead {
                call,
                buffer,
                traced: TransmitTraced::new(this.get(), promise.get(), None),
                index,
            },
        );
    } else {
        let count = usize::try_from(code >> 4).unwrap();
        let code = code & 0xF;
        if code == RETURN_CODE_DROPPED {
            rooted!(&in(cx) let value = BooleanValue(true));
            set(cx, this.handle(), c"writerDropped", value.handle());
        }

        if let Some(ty) = ty.ty().filter(|&v| use_typed_array(v)) {
            rooted!(&in(cx) let value = unsafe { create_typed_array(cx, ty, buffer, count) });
            resolve(cx, promise.handle(), value.handle());
        } else {
            rooted!(&in(cx) let array = unsafe { NewArrayObject1(cx, count) });
            for offset in 0..count {
                unsafe { ty.lift(&mut call, buffer.add(ty.abi_payload_size() * offset)) };
                rooted!(&in(cx) let value = call.pop());
                set_element(
                    cx,
                    array.handle(),
                    offset.try_into().unwrap(),
                    value.handle(),
                );
            }
            rooted!(&in(cx) let value = ObjectValue(array.get()));
            resolve(cx, promise.handle(), value.handle());
        }
    }

    args.rval().set(ObjectValue(promise.get()));
    true
}

unsafe extern "C" fn stream_drop_readable(
    cx: *mut RawJSContext,
    argc: u32,
    vp: *mut Value,
) -> bool {
    assert_eq!(argc, 0);

    let args = unsafe { JS_CallArgsFromVp(argc, vp) };
    let cx = &mut unsafe { JSContext::from_ptr(NonNull::new(cx).unwrap()) };
    rooted!(&in(cx) let this = args.thisv().to_object());
    let index = get(cx, this.handle(), TYPE_FIELD_NAME);
    let handle = get(cx, this.handle(), HANDLE_FIELD_NAME);

    if index.is_int32() && handle.is_int32() {
        let index = index.to_int32() as u32;
        let handle = handle.to_int32() as u32;
        let ty = WIT.get().unwrap().stream(usize::try_from(index).unwrap());

        unregister_resource(cx, this.handle());

        unsafe { ty.drop_readable()(handle) };
    }

    args.rval().set(UndefinedValue());
    true
}

unsafe extern "C" fn make_stream(cx: *mut RawJSContext, argc: u32, vp: *mut Value) -> bool {
    assert_eq!(argc, 1);

    let args = unsafe { JS_CallArgsFromVp(argc, vp) };
    let cx = &mut unsafe { JSContext::from_ptr(NonNull::new(cx).unwrap()) };
    let index = args.index(0);
    let ty = WIT
        .get()
        .unwrap()
        .stream(usize::try_from(index.to_int32()).unwrap());
    let handles = unsafe { ty.new()() };
    let tx_handle = u32::try_from(handles >> 32).unwrap();
    let rx_handle = u32::try_from(handles & 0xFFFF_FFFF).unwrap();

    rooted!(&in(cx) let tx = unsafe { JS_NewObject(cx, ptr::null_mut()) });
    set(cx, tx.handle(), TYPE_FIELD_NAME, unsafe {
        Handle::from_raw(index)
    });

    rooted!(&in(cx) let mut write = wrap(cx, stream_write));
    set(cx, tx.handle(), c"write", write.handle());

    rooted!(&in(cx) let mut dispose = wrap(cx, stream_drop_writable));
    set_with_symbol(cx, tx.handle(), SymbolCode::dispose, dispose.handle());

    register_resource(cx, tx.handle(), tx_handle);

    rooted!(&in(cx) let global_object = unsafe { CurrentGlobalOrNull(cx) });
    rooted!(&in(cx) let write_all = get(cx, global_object.handle(), c"_componentizeJsWriteAll"));
    set(cx, tx.handle(), c"writeAll", write_all.handle());

    rooted!(&in(cx) let rx = unsafe { JS_NewObject(cx, ptr::null_mut()) });
    set(cx, rx.handle(), TYPE_FIELD_NAME, unsafe {
        Handle::from_raw(index)
    });

    rooted!(&in(cx) let mut read = wrap(cx, stream_read));
    set(cx, rx.handle(), c"read", read.handle());

    rooted!(&in(cx) let mut dispose = wrap(cx, stream_drop_readable));
    set_with_symbol(cx, rx.handle(), SymbolCode::dispose, dispose.handle());

    register_resource(cx, rx.handle(), rx_handle);

    rooted!(&in(cx) let elements = vec![ObjectValue(tx.get()), ObjectValue(rx.get())]);
    args.rval().set(ObjectValue(unsafe {
        NewArrayObject(cx, &HandleValueArray::from(&elements))
    }));
    true
}

unsafe extern "C" fn future_write(cx: *mut RawJSContext, argc: u32, vp: *mut Value) -> bool {
    assert_eq!(argc, 1);

    // TODO: Detect and raise exception if future has already been written or
    // dropped.

    let args = unsafe { JS_CallArgsFromVp(argc, vp) };
    let cx = &mut unsafe { JSContext::from_ptr(NonNull::new(cx).unwrap()) };
    rooted!(&in(cx) let this = args.thisv().to_object());
    let index = get(cx, this.handle(), TYPE_FIELD_NAME).to_int32() as u32;
    let handle = get(cx, this.handle(), HANDLE_FIELD_NAME).to_int32() as u32;

    // TODO: will need to re-register on cancellation once that's supported:
    unregister_resource(cx, this.handle());

    let ty = WIT.get().unwrap().future(usize::try_from(index).unwrap());

    let layout = Layout::from_size_align(ty.abi_payload_size(), ty.abi_payload_align()).unwrap();
    let buffer = unsafe { std::alloc::alloc(layout) };
    if buffer.is_null() {
        panic!("unable to allocate buffer for stream read");
    }

    let mut call = MyCall::new();
    call.push(args.index(0).get());
    unsafe { call.defer_deallocate(buffer, layout) };

    call.traced.try_lock().unwrap().resources = Some(Vec::new());
    let code = unsafe {
        ty.lower(&mut call, buffer);
        ty.write()(handle, buffer.cast())
    };

    rooted!(&in(cx) let promise = unsafe { NewPromiseObject(cx, Handle::<*mut JSObject>::null()) });
    let traced = TransmitTraced::new(
        this.get(),
        promise.get(),
        call.traced
            .try_lock()
            .unwrap()
            .resources
            .take()
            .and_then(|v| if v.is_empty() { None } else { Some(vec![v]) }),
    );

    if code == RETURN_CODE_BLOCKED {
        let mut state = CURRENT_TASK_STATE.try_lock().unwrap();
        let state = &mut state.as_mut().unwrap().0;
        if state.waitable_set.is_none() {
            state.waitable_set = Some(unsafe { waitable_set_new() });
        }
        unsafe { waitable_join(handle, state.waitable_set.unwrap()) }

        state.pending.insert(
            handle,
            Pending::FutureWrite {
                _call: call,
                traced,
            },
        );
    } else {
        let code = code & 0xF;

        let result = match code {
            self::RETURN_CODE_COMPLETED => true,
            self::RETURN_CODE_DROPPED => {
                restore_resources(cx, &traced, 0);
                false
            }
            self::RETURN_CODE_CANCELLED => todo!(),
            _ => unreachable!(),
        };
        rooted!(&in(cx) let value = BooleanValue(result));
        resolve(cx, promise.handle(), value.handle());
    }

    args.rval().set(ObjectValue(promise.get()));
    true
}

unsafe extern "C" fn future_read(cx: *mut RawJSContext, argc: u32, vp: *mut Value) -> bool {
    assert_eq!(argc, 0);

    // TODO: Detect and raise exception if future has already been read or
    // dropped.

    let args = unsafe { JS_CallArgsFromVp(argc, vp) };
    let cx = &mut unsafe { JSContext::from_ptr(NonNull::new(cx).unwrap()) };
    rooted!(&in(cx) let this = args.thisv().to_object());
    let index = usize::try_from(get(cx, this.handle(), TYPE_FIELD_NAME).to_int32() as u32).unwrap();
    let handle = get(cx, this.handle(), HANDLE_FIELD_NAME).to_int32() as u32;

    // TODO: will need to re-register on cancellation once that's supported:
    unregister_resource(cx, this.handle());

    let ty = WIT.get().unwrap().future(index);

    let layout = Layout::from_size_align(ty.abi_payload_size(), ty.abi_payload_align()).unwrap();
    let buffer = unsafe { std::alloc::alloc(layout) };
    if buffer.is_null() {
        panic!("unable to allocate buffer for future read");
    }

    let mut call = MyCall::new();
    unsafe { call.defer_deallocate(buffer, layout) };

    let code = unsafe { ty.read()(handle, buffer.cast()) };

    rooted!(&in(cx) let promise = unsafe { NewPromiseObject(cx, Handle::<*mut JSObject>::null()) });

    if code == RETURN_CODE_BLOCKED {
        let mut state = CURRENT_TASK_STATE.try_lock().unwrap();
        let state = &mut state.as_mut().unwrap().0;
        if state.waitable_set.is_none() {
            state.waitable_set = Some(unsafe { waitable_set_new() });
        }
        unsafe { waitable_join(handle, state.waitable_set.unwrap()) }

        state.pending.insert(
            handle,
            Pending::FutureRead {
                call,
                buffer,
                traced: TransmitTraced::new(this.get(), promise.get(), None),
                index,
            },
        );
    } else {
        let code = code & 0xF;
        match code {
            self::RETURN_CODE_COMPLETED => unsafe { ty.lift(&mut call, buffer) },
            self::RETURN_CODE_DROPPED => unreachable!(),
            self::RETURN_CODE_CANCELLED => todo!(),
            _ => unreachable!(),
        }
        rooted!(&in(cx) let value = call.pop());
        resolve(cx, promise.handle(), value.handle());
    }

    args.rval().set(ObjectValue(promise.get()));
    true
}

unsafe extern "C" fn future_drop_readable(
    cx: *mut RawJSContext,
    argc: u32,
    vp: *mut Value,
) -> bool {
    assert_eq!(argc, 0);

    let args = unsafe { JS_CallArgsFromVp(argc, vp) };
    let cx = &mut unsafe { JSContext::from_ptr(NonNull::new(cx).unwrap()) };
    rooted!(&in(cx) let this = args.thisv().to_object());
    let index = get(cx, this.handle(), TYPE_FIELD_NAME);
    let handle = get(cx, this.handle(), HANDLE_FIELD_NAME);

    if index.is_int32() && handle.is_int32() {
        let index = index.to_int32() as u32;
        let handle = handle.to_int32() as u32;
        let ty = WIT.get().unwrap().future(usize::try_from(index).unwrap());

        unregister_resource(cx, this.handle());

        unsafe { ty.drop_readable()(handle) };
    }

    args.rval().set(UndefinedValue());
    true
}

unsafe extern "C" fn make_future(cx: *mut RawJSContext, argc: u32, vp: *mut Value) -> bool {
    assert_eq!(argc, 2);

    let args = unsafe { JS_CallArgsFromVp(argc, vp) };
    let cx = &mut unsafe { JSContext::from_ptr(NonNull::new(cx).unwrap()) };
    let index = args.index(0);
    let default = args.index(1);
    let ty = WIT
        .get()
        .unwrap()
        .future(usize::try_from(index.to_int32()).unwrap());
    let handles = unsafe { ty.new()() };
    let tx_handle = u32::try_from(handles >> 32).unwrap();
    let rx_handle = u32::try_from(handles & 0xFFFF_FFFF).unwrap();

    rooted!(&in(cx) let tx = unsafe { JS_NewObject(cx, ptr::null_mut()) });
    set(cx, tx.handle(), TYPE_FIELD_NAME, unsafe {
        Handle::from_raw(index)
    });

    rooted!(&in(cx) let mut write = wrap(cx, future_write));
    set(cx, tx.handle(), c"write", write.handle());
    set(cx, tx.handle(), c"default", unsafe {
        Handle::from_raw(default)
    });

    rooted!(&in(cx) let global_object = unsafe { CurrentGlobalOrNull(cx) });
    rooted!(&in(cx) let dispose = get(cx, global_object.handle(), c"_componentizeJsMaybeWriteDefault"));
    set_with_symbol(cx, tx.handle(), SymbolCode::dispose, dispose.handle());

    register_resource(cx, tx.handle(), tx_handle);

    rooted!(&in(cx) let rx = unsafe { JS_NewObject(cx, ptr::null_mut()) });
    set(cx, rx.handle(), TYPE_FIELD_NAME, unsafe {
        Handle::from_raw(index)
    });

    rooted!(&in(cx) let mut read = wrap(cx, future_read));
    set(cx, rx.handle(), c"read", read.handle());

    rooted!(&in(cx) let mut dispose = wrap(cx, future_drop_readable));
    set_with_symbol(cx, rx.handle(), SymbolCode::dispose, dispose.handle());

    register_resource(cx, rx.handle(), rx_handle);

    rooted!(&in(cx) let elements = vec![ObjectValue(tx.get()), ObjectValue(rx.get())]);
    args.rval().set(ObjectValue(unsafe {
        NewArrayObject(cx, &HandleValueArray::from(&elements))
    }));

    true
}

unsafe extern "C" fn encode_utf8(cx: *mut RawJSContext, argc: u32, vp: *mut Value) -> bool {
    assert_eq!(argc, 1);

    let args = unsafe { JS_CallArgsFromVp(argc, vp) };
    let cx = &mut unsafe { JSContext::from_ptr(NonNull::new(cx).unwrap()) };

    let string = unsafe {
        jsstr_to_string(
            cx.raw_cx(),
            NonNull::new(args.index(0).to_string()).unwrap(),
        )
    };

    rooted!(&in(cx) let mut array = ptr::null_mut::<JSObject>());
    unsafe {
        Uint8Array::create(
            cx.raw_cx(),
            CreateWith::Slice(string.as_bytes()),
            array.handle_mut(),
        )
        .unwrap()
    }
    args.rval().set(ObjectValue(array.get()));

    true
}

unsafe extern "C" fn decode_utf8(cx: *mut RawJSContext, argc: u32, vp: *mut Value) -> bool {
    assert_eq!(argc, 1);

    let args = unsafe { JS_CallArgsFromVp(argc, vp) };
    let cx = &mut unsafe { JSContext::from_ptr(NonNull::new(cx).unwrap()) };
    let (data, length) = unsafe { Uint8::length_and_data(args.index(0).to_object()) };

    let string = str::from_utf8(unsafe { slice::from_raw_parts(data, length) })
        .unwrap()
        .to_string();

    args.rval().set(StringValue(unsafe {
        &*JS_NewStringCopyUTF8N(cx, &*Utf8Chars::from(string.as_str()))
    }));

    true
}

unsafe extern "C" fn resolve_import(
    cx: *mut RawJSContext,
    _: RawHandle<Value>,
    specifier: RawHandle<*mut JSObject>,
) -> *mut JSObject {
    let cx = &mut unsafe { JSContext::from_ptr(NonNull::new(cx).unwrap()) };

    let specifier = unsafe {
        jsstr_to_string(
            cx.raw_cx(),
            NonNull::new(GetModuleRequestSpecifier(cx, Handle::from_raw(specifier))).unwrap(),
        )
    };

    let mut module = MODULES
        .try_lock()
        .unwrap()
        .0
        .get(&specifier)
        .map(|v| v.get())
        .unwrap_or_else(ptr::null_mut);

    if module.is_null() {
        // Try loading it from the filesystem
        if let Ok(script) = fs::read_to_string(&specifier) {
            let compile_options =
                CompileOptionsWrapper::new(cx, CString::new(specifier.as_str()).unwrap(), 1);
            module = unsafe {
                CompileModule1(
                    cx,
                    compile_options.ptr,
                    &mut rust::transform_str_to_source_text(&script),
                )
            };
            if module.is_null() {
                unsafe { PrintAndClearException(cx.raw_cx()) }
                panic!("CompileModule1 failed")
            }
            MODULES
                .try_lock()
                .unwrap()
                .0
                .insert(specifier, Heap::boxed(module));
        } else {
            panic!("unable to resolve import `{specifier}`");
        }
    }

    module
}

fn evaluate(cx: &mut JSContext, name: &str, script: &str) -> anyhow::Result<*mut JSObject> {
    let compile_options = CompileOptionsWrapper::new(cx, CString::new(name)?, 1);
    let module = unsafe {
        CompileModule1(
            cx,
            compile_options.ptr,
            &mut rust::transform_str_to_source_text(script),
        )
    };
    if module.is_null() {
        unsafe { PrintAndClearException(cx.raw_cx()) }
        bail!("CompileModule1 failed")
    }

    rooted!(&in(cx) let module = module);
    if !unsafe { ModuleLink(cx, module.handle()) } {
        unsafe { PrintAndClearException(cx.raw_cx()) }
        bail!("ModuleLink failed")
    }

    rooted!(&in(cx) let mut result = UndefinedValue());
    if !unsafe { ModuleEvaluate(cx, module.handle(), result.handle_mut()) } {
        unsafe { PrintAndClearException(cx.raw_cx()) }
        bail!("ModuleEvaluate failed")
    }

    unsafe { RunJobs(cx) }

    rooted!(&in(cx) let result = result.to_object());
    if !unsafe {
        ThrowOnModuleEvaluationFailure(
            cx,
            result.handle(),
            ModuleErrorBehaviour::ThrowModuleErrorsSync,
        )
    } {
        unsafe { PrintAndClearException(cx.raw_cx()) }
        bail!("ThrowOnModuleEvaluationFailure failed")
    }

    assert!(unsafe { IsPromiseObject(result.handle()) });
    assert_eq!(PromiseState::Fulfilled, unsafe {
        GetPromiseState(result.handle())
    });

    Ok(module.get())
}

fn init(globals: &str, modules: &[(&str, &str)], script: &str) -> anyhow::Result<()> {
    init_runtime()?;

    let cx = &mut context();

    for (name, func) in [
        (c"_componentizeJsCallImport", call_import as JsFunction),
        (
            c"_componentizeJsCallTaskReturn",
            call_task_return as JsFunction,
        ),
        (c"_componentizeJsDropResource", drop_resource as JsFunction),
        (c"_componentizeJsLog", log as JsFunction),
        (c"_componentizeJsMakeStream", make_stream as JsFunction),
        (c"_componentizeJsMakeFuture", make_future as JsFunction),
        (c"_componentizeJsEncodeUtf8", encode_utf8 as JsFunction),
        (c"_componentizeJsDecodeUtf8", decode_utf8 as JsFunction),
    ] {
        rooted!(&in(cx) let mut func = wrap(cx, func));
        rooted!(&in(cx) let global_object = unsafe { CurrentGlobalOrNull(cx) });
        set(cx, global_object.handle(), name, func.handle());
    }

    let compile_options = CompileOptionsWrapper::new(cx, c"script".into(), 1);
    rooted!(&in(cx) let mut result = UndefinedValue());
    if !unsafe {
        Evaluate2(
            cx,
            compile_options.ptr,
            &mut rust::transform_str_to_source_text(globals),
            result.handle_mut(),
        )
    } {
        unsafe { PrintAndClearException(cx.raw_cx()) }
        bail!("Evaluate2 failed")
    }

    for &(name, script) in modules {
        let module = evaluate(cx, name, script)?;
        MODULES
            .try_lock()
            .unwrap()
            .0
            .insert(name.into(), Heap::boxed(module));
    }

    let module = evaluate(cx, "script", script)?;
    *MAIN_MODULE.try_lock().unwrap() = Some(SyncSend(Heap::boxed(module)));

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
    fn init(globals: String, modules: Vec<(String, String)>, script: String) -> Result<(), String> {
        let result = init(
            &globals,
            &modules
                .iter()
                .map(|(a, b)| (a.as_str(), b.as_str()))
                .collect::<Vec<_>>(),
            &script,
        )
        .map_err(|e| format!("{e:?}"));

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
            *CURRENT_TASK_STATE.try_lock().unwrap() = Some(SyncSend(TaskState::default()));
        }

        let cx = &mut context();
        rooted!(&in(cx) let mut module = MAIN_MODULE.try_lock().unwrap().as_ref().unwrap().0.get());
        rooted!(&in(cx) let mut object = unsafe {
            mozjs::rust::wrappers2::GetModuleNamespace(cx, module.handle())
        });

        if async_ {
            object.set(get(cx, object.handle(), c"_componentizeJsAsyncExports").to_object());
        }

        if let Some(interface) = func.interface() {
            object.set(
                get(
                    cx,
                    object.handle(),
                    &CString::new(mangle_name(interface)).unwrap(),
                )
                .to_object(),
            );
        }

        let params = |call: &mut MyCall, offset| {
            if async_ {
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
                call.traced
                    .try_lock()
                    .unwrap()
                    .stack
                    .drain(offset..)
                    .map(|v| v.get()),
            )
            .collect::<Vec<_>>()
        };

        let result = if let Some(ty) = func.name().strip_prefix("[constructor]") {
            assert!(!async_);

            let class = get(
                cx,
                object.handle(),
                &CString::new(ty.to_upper_camel_case()).unwrap(),
            );
            rooted!(&in(cx) let class = class);
            if class.is_undefined() {
                panic!("export `{}` not defined", ty.to_upper_camel_case());
            }
            rooted!(&in(cx) let mut result = ptr::null_mut::<JSObject>());
            rooted!(&in(cx) let params = params(call, 0));
            if !unsafe {
                Construct1(
                    cx,
                    class.handle(),
                    &HandleValueArray::from(&params),
                    result.handle_mut(),
                )
            } {
                unsafe { PrintAndClearException(cx.raw_cx()) }
                panic!("Construct1 failed")
            }
            ObjectValue(result.get())
        } else if let Some(name) = func.name().strip_prefix("[method]") {
            let (ty, name) = name.split_once('.').unwrap();
            let class = get(
                cx,
                object.handle(),
                &CString::new(ty.to_upper_camel_case()).unwrap(),
            );
            rooted!(&in(cx) let class = class);
            if class.is_undefined() {
                panic!("export `{}` not defined", ty.to_upper_camel_case());
            }
            rooted!(&in(cx) let object = class.to_object());
            let function = get(
                cx,
                object.handle(),
                &CString::new(name.to_lower_camel_case()).unwrap(),
            );
            rooted!(&in(cx) let function = function);
            if function.is_undefined() {
                panic!("export `{}` not defined", mangle_name(func.name()));
            }
            rooted!(&in(cx) let params = params(call, 1));
            rooted!(&in(cx) let this = call.pop().to_object());
            self::call(
                cx,
                this.handle(),
                function.handle(),
                &HandleValueArray::from(&params),
            )
        } else if let Some(name) = func.name().strip_prefix("[static]") {
            let (ty, name) = name.split_once('.').unwrap();
            let class = get(
                cx,
                object.handle(),
                &CString::new(ty.to_upper_camel_case()).unwrap(),
            );
            rooted!(&in(cx) let class = class);
            if class.is_undefined() {
                panic!("export `{}` not defined", ty.to_upper_camel_case());
            }
            rooted!(&in(cx) let object = class.to_object());
            let function = get(
                cx,
                object.handle(),
                &CString::new(name.to_lower_camel_case()).unwrap(),
            );
            rooted!(&in(cx) let function = function);
            if function.is_undefined() {
                panic!("export `{}` not defined", mangle_name(func.name()));
            }
            rooted!(&in(cx) let params = params(call, 0));
            self::call(
                cx,
                object.handle(),
                function.handle(),
                &HandleValueArray::from(&params),
            )
        } else {
            let function = get(
                cx,
                object.handle(),
                &CString::new(mangle_name(func.name())).unwrap(),
            );
            rooted!(&in(cx) let function = function);
            if function.is_undefined() {
                panic!("export `{}` not defined", mangle_name(func.name()));
            }
            rooted!(&in(cx) let params = params(call, 0));
            self::call(
                cx,
                object.handle(),
                function.handle(),
                &HandleValueArray::from(&params),
            )
        };

        if async_ {
            poll(cx)
        } else {
            rooted!(&in(cx) let mut result = result);
            let fulfilled = !unsafe { JS_IsExceptionPending(cx) };
            if !fulfilled {
                rooted!(&in(cx) let mut exception = UndefinedValue());
                if !unsafe { JS_GetPendingException(cx, exception.handle_mut()) } {
                    unsafe { PrintAndClearException(cx.raw_cx()) }
                    panic!("JS_GetPendingException failed")
                }
                unsafe { JS_ClearPendingException(cx) };
                result.set(exception.get())
            }

            handle_export_result(cx, call, func.result(), result.handle(), fulfilled);

            release_borrows(cx, &call.traced);

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
            self::EVENT_NONE => {}
            self::EVENT_SUBTASK => match event2 {
                self::STATUS_STARTING => unreachable!(),
                self::STATUS_STARTED => {}
                self::STATUS_RETURNED => {
                    unsafe {
                        waitable_join(event1, 0);
                        subtask_drop(event1);
                    }

                    let Pending::ImportCall {
                        index,
                        buffer,
                        ref mut call,
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

                    unsafe { func.lift_import_async_result(call, buffer) };
                    assert!(call.len() < 4);

                    let result = handle_import_result(cx, call, func.result());

                    rooted!(&in(cx) let reject = call.pop());
                    rooted!(&in(cx) let resolve = call.pop());

                    let (result, resolve_or_reject) = match result {
                        Ok(value) => (value.unwrap_or_else(UndefinedValue), resolve),
                        Err(value) => (value, reject),
                    };
                    rooted!(&in(cx) let params = vec![result]);

                    self::call(
                        cx,
                        Handle::<*mut JSObject>::null(),
                        resolve_or_reject.handle(),
                        &HandleValueArray::from(&params),
                    );
                }
                _ => todo!(),
            },
            self::EVENT_STREAM_WRITE => {
                unsafe { waitable_join(event1, 0) };

                let pending = &CURRENT_TASK_STATE
                    .try_lock()
                    .unwrap()
                    .as_mut()
                    .unwrap()
                    .0
                    .pending
                    .remove(&event1)
                    .unwrap();

                let Pending::StreamWrite { traced, .. } = pending else {
                    unreachable!()
                };

                rooted!(&in(cx) let stream = traced.try_lock().unwrap().wrapper.get());
                rooted!(&in(cx) let promise = traced.try_lock().unwrap().promise.get());

                let count = event2 >> 4;
                let code = event2 & 0xF;

                if code == RETURN_CODE_DROPPED {
                    rooted!(&in(cx) let value = BooleanValue(true));
                    set(cx, stream.handle(), c"readerDropped", value.handle());
                }

                restore_resources(cx, traced, count);

                rooted!(&in(cx) let value = UInt32Value(count));
                resolve(cx, promise.handle(), value.handle());
            }
            self::EVENT_STREAM_READ => {
                unsafe { waitable_join(event1, 0) };

                let pending = &mut CURRENT_TASK_STATE
                    .try_lock()
                    .unwrap()
                    .as_mut()
                    .unwrap()
                    .0
                    .pending
                    .remove(&event1)
                    .unwrap();

                let &mut Pending::StreamRead {
                    ref traced,
                    buffer,
                    ref mut call,
                    index,
                    ..
                } = pending
                else {
                    unreachable!()
                };

                rooted!(&in(cx) let stream = traced.try_lock().unwrap().wrapper.get());
                rooted!(&in(cx) let promise = traced.try_lock().unwrap().promise.get());

                let ty = WIT.get().unwrap().stream(index);

                let count = usize::try_from(event2 >> 4).unwrap();
                let code = event2 & 0xF;

                if code == RETURN_CODE_DROPPED {
                    rooted!(&in(cx) let value = BooleanValue(true));
                    set(cx, stream.handle(), c"writerDropped", value.handle());
                }

                assert!(traced.try_lock().unwrap().resources.is_none());

                if let Some(ty) = ty.ty().filter(|&v| use_typed_array(v)) {
                    rooted!(&in(cx) let value = unsafe { create_typed_array(cx, ty, buffer, count) });
                    resolve(cx, promise.handle(), value.handle());
                } else {
                    rooted!(&in(cx) let array = unsafe { NewArrayObject1(cx, count) });
                    for offset in 0..count {
                        unsafe { ty.lift(call, buffer.add(ty.abi_payload_size() * offset)) };
                        rooted!(&in(cx) let value = call.pop());
                        set_element(
                            cx,
                            array.handle(),
                            offset.try_into().unwrap(),
                            value.handle(),
                        );
                    }
                    rooted!(&in(cx) let value = ObjectValue(array.get()));
                    resolve(cx, promise.handle(), value.handle());
                }
            }
            self::EVENT_FUTURE_WRITE => {
                unsafe { waitable_join(event1, 0) };

                let pending = &CURRENT_TASK_STATE
                    .try_lock()
                    .unwrap()
                    .as_mut()
                    .unwrap()
                    .0
                    .pending
                    .remove(&event1)
                    .unwrap();

                let Pending::FutureWrite { traced, .. } = pending else {
                    unreachable!()
                };

                rooted!(&in(cx) let promise = traced.try_lock().unwrap().promise.get());

                let code = event2 & 0xF;

                if code == RETURN_CODE_DROPPED {
                    restore_resources(cx, traced, 0);
                }

                let result = match code {
                    self::RETURN_CODE_COMPLETED => true,
                    self::RETURN_CODE_DROPPED => false,
                    self::RETURN_CODE_CANCELLED => todo!(),
                    _ => unreachable!(),
                };
                rooted!(&in(cx) let value = BooleanValue(result));
                resolve(cx, promise.handle(), value.handle());
            }
            self::EVENT_FUTURE_READ => {
                unsafe { waitable_join(event1, 0) };

                let pending = &mut CURRENT_TASK_STATE
                    .try_lock()
                    .unwrap()
                    .as_mut()
                    .unwrap()
                    .0
                    .pending
                    .remove(&event1)
                    .unwrap();

                let &mut Pending::FutureRead {
                    ref traced,
                    buffer,
                    ref mut call,
                    index,
                    ..
                } = pending
                else {
                    unreachable!()
                };

                assert!(traced.try_lock().unwrap().resources.is_none());

                rooted!(&in(cx) let future = traced.try_lock().unwrap().wrapper.get());
                rooted!(&in(cx) let promise = traced.try_lock().unwrap().promise.get());

                let ty = WIT.get().unwrap().future(index);

                let code = event2 & 0xF;
                match code {
                    self::RETURN_CODE_COMPLETED => unsafe { ty.lift(call, buffer) },
                    self::RETURN_CODE_DROPPED => unreachable!(),
                    self::RETURN_CODE_CANCELLED => todo!(),
                    _ => unreachable!(),
                }
                rooted!(&in(cx) let value = call.pop());
                resolve(cx, promise.handle(), value.handle());
            }
            self::EVENT_CANCELLED => todo!(),
            _ => unreachable!(),
        }

        poll(cx)
    }

    fn resource_dtor(ty: wit::Resource, handle: usize) {
        let cx = &mut context();
        let wrapper = EXPORTED_RESOURCES.try_lock().unwrap().0.remove(handle);

        rooted!(&in(cx) let wrapper = wrapper.get());
        assert_eq!(
            ty.index(),
            usize::try_from(get(cx, wrapper.handle(), TYPE_FIELD_NAME).to_int32() as u32).unwrap()
        );
    }
}

struct MyCallTraced {
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
    traced: Arc<Mutex<MyCallTraced>>,
}

impl MyCall<'_> {
    #[expect(clippy::arc_with_non_send_sync)]
    fn new() -> Self {
        let traced = Arc::new(Mutex::new(MyCallTraced {
            stack: Vec::new(),
            resources: None,
            borrows: Vec::new(),
        }));
        assert!(
            MY_CALL_TRACED
                .try_lock()
                .unwrap()
                .0
                .insert(ArcHash(traced.clone()))
        );
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

    fn imported_resource_to_canon(&mut self, cx: &mut JSContext, value: Value, owned: bool) -> u32 {
        rooted!(&in(cx) let value = value.to_object());
        let handle = get(cx, value.handle(), HANDLE_FIELD_NAME).to_int32() as u32;

        if owned {
            unregister_resource(cx, value.handle());

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
            MY_CALL_TRACED
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
        let cx = &mut context();
        let value = self.pop();
        if let Some(new) = ty.new() {
            // exported resource type
            exported_resource_to_canon(cx, ty, new, value)
        } else {
            // imported resource type
            self.imported_resource_to_canon(cx, value, false)
        }
    }

    fn pop_own(&mut self, ty: wit::Resource) -> u32 {
        let cx = &mut context();
        let value = self.pop();
        if let Some(new) = ty.new() {
            // exported resource type
            exported_resource_to_canon(cx, ty, new, value)
        } else {
            // imported resource type
            self.imported_resource_to_canon(cx, value, true)
        }
    }

    fn pop_enum(&mut self, ty: wit::Enum) -> u32 {
        let cx = &mut context();
        let tag =
            unsafe { jsstr_to_string(cx.raw_cx(), NonNull::new(self.pop().to_string()).unwrap()) };
        // TODO: use e.g. a HashMap to make this more efficient:
        ty.names()
            .position(|v| v == tag.as_str())
            .unwrap()
            .try_into()
            .unwrap()
    }

    fn pop_flags(&mut self, _ty: wit::Flags) -> u32 {
        let cx = &mut context();
        rooted!(&in(cx) let wrapper = self.pop().to_object());
        get(cx, wrapper.handle(), c"val").to_int32() as u32
    }

    fn pop_future(&mut self, _ty: wit::Future) -> u32 {
        let cx = &mut context();
        let value = self.pop();
        self.imported_resource_to_canon(cx, value, true)
    }

    fn pop_stream(&mut self, _ty: wit::Stream) -> u32 {
        let cx = &mut context();
        let value = self.pop();
        self.imported_resource_to_canon(cx, value, true)
    }

    fn pop_option(&mut self, ty: WitOption) -> u32 {
        if self.last().is_undefined() {
            self.pop();
            0
        } else {
            if let Type::Option(_) = ty.ty() {
                let cx = &mut context();
                rooted!(&in(cx) let wrapper = self.pop().to_object());
                self.push(get(cx, wrapper.handle(), c"val"));
            } else {
                // Leave value on the stack as-is.
            }
            1
        }
    }

    fn pop_result(&mut self, ty: WitResult) -> u32 {
        let cx = &mut context();
        rooted!(&in(cx) let wrapper = self.pop().to_object());
        let tag = unsafe {
            jsstr_to_string(
                cx.raw_cx(),
                NonNull::new(get(cx, wrapper.handle(), c"tag").to_string()).unwrap(),
            )
        };

        let (discriminant, has_payload) = match tag.as_str() {
            "ok" => (0, ty.ok().is_some()),
            "err" => (1, ty.err().is_some()),
            _ => unreachable!(),
        };

        if has_payload {
            self.push(get(cx, wrapper.handle(), c"val"));
        }

        discriminant
    }

    fn pop_variant(&mut self, ty: wit::Variant) -> u32 {
        let cx = &mut context();
        rooted!(&in(cx) let wrapper = self.pop().to_object());
        let tag = unsafe {
            jsstr_to_string(
                cx.raw_cx(),
                NonNull::new(get(cx, wrapper.handle(), c"tag").to_string()).unwrap(),
            )
        };

        // TODO: use e.g. a HashMap to make this more efficient:
        let (discriminant, (_, payload_type)) = ty
            .cases()
            .enumerate()
            .find(|(_, (v, _))| *v == tag.as_str())
            .unwrap();

        if payload_type.is_some() {
            self.push(get(cx, wrapper.handle(), c"val"));
        }

        discriminant.try_into().unwrap()
    }

    fn pop_record(&mut self, ty: wit::Record) {
        let cx = &mut context();
        rooted!(&in(cx) let record = self.pop().to_object());
        for (name, _) in ty.fields() {
            self.push(get(
                cx,
                record.handle(),
                &CString::new(mangle_name(name)).unwrap(),
            ));
        }
    }

    fn pop_tuple(&mut self, ty: wit::Tuple) {
        let count = ty.types().len();
        let cx = &mut context();
        rooted!(&in(cx) let tuple = self.pop().to_object());
        for index in 0..count {
            self.push(get_element(
                cx,
                tuple.handle(),
                u32::try_from(count - index - 1).unwrap(),
            ));
        }
    }

    unsafe fn maybe_pop_list(&mut self, ty: List) -> Option<(*const u8, usize)> {
        if use_typed_array(ty.ty()) {
            let (data, length, layout) =
                unsafe { typed_array_data(ty.ty(), self.pop().to_object()) };
            let dst = unsafe { alloc::alloc(layout) };
            unsafe { ptr::copy_nonoverlapping(data, dst, layout.size()) };
            Some((dst as _, length))
        } else {
            None
        }
    }

    fn pop_list(&mut self, _ty: List) -> usize {
        self.iter_stack.push(0);
        let cx = &mut context();
        rooted!(&in(cx) let list = self.last().to_object());
        // TODO: Is there a quicker way to get the array length, e.g. using
        // `JS_GetPropertyById`?
        get(cx, list.handle(), c"length")
            .to_int32()
            .try_into()
            .unwrap()
    }

    fn pop_iter_next(&mut self, _ty: List) {
        let index = *self.iter_stack.last().unwrap();
        let cx = &mut context();
        rooted!(&in(cx) let list = self.last().to_object());
        let value = get_element(cx, list.handle(), u32::try_from(index).unwrap());
        *self.iter_stack.last_mut().unwrap() = index + 1;
        self.push(value);
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
            set(
                cx,
                value.handle(),
                &CString::new(mangle_name(name)).unwrap(),
                field.handle(),
            );
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
        set(cx, wrapper.handle(), c"val", value.handle());
        self.push(ObjectValue(wrapper.get()));
    }

    fn push_enum(&mut self, ty: wit::Enum, discriminant: u32) {
        let cx = &mut context();
        self.push(StringValue(unsafe {
            &*JS_NewStringCopyUTF8N(
                cx,
                &*Utf8Chars::from(ty.names().nth(discriminant.try_into().unwrap()).unwrap()),
            )
        }));
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
            let value =
                imported_resource_from_canon(&mut context(), ty.index(), handle, None, Some(ty));

            self.traced.try_lock().unwrap().borrows.push(Borrow {
                value: Heap::boxed(value),
                handle,
                drop: ty.drop(),
            });

            value
        }));
    }

    fn push_own(&mut self, ty: wit::Resource, handle: u32) {
        let cx = &mut context();
        self.push(ObjectValue(if let Some(rep) = ty.rep() {
            // exported resource type
            let rep = unsafe { rep(handle) };
            rooted!(&in(cx) let value = EXPORTED_RESOURCES.try_lock().unwrap().0.remove(rep).get());

            unregister_resource(cx, value.handle());

            value.get()
        } else {
            // imported resource type
            imported_resource_from_canon(cx, ty.index(), handle, None, Some(ty))
        }));
    }

    fn push_future(&mut self, ty: wit::Future, handle: u32) {
        let cx = &mut context();
        let stream =
            imported_resource_from_canon(cx, ty.index(), handle, Some(future_drop_readable), None);
        rooted!(&in(cx) let rx = stream);

        rooted!(&in(cx) let mut func = wrap(cx, future_read));
        set(cx, rx.handle(), c"read", func.handle());

        self.push(ObjectValue(rx.get()))
    }

    fn push_stream(&mut self, ty: wit::Stream, handle: u32) {
        let cx = &mut context();
        let stream =
            imported_resource_from_canon(cx, ty.index(), handle, Some(stream_drop_readable), None);
        rooted!(&in(cx) let rx = stream);

        rooted!(&in(cx) let mut func = wrap(cx, stream_read));
        set(cx, rx.handle(), c"read", func.handle());

        self.push(ObjectValue(rx.get()))
    }

    fn push_variant(&mut self, ty: wit::Variant, discriminant: u32) {
        let cx = &mut context();
        rooted!(&in(cx) let wrapper = unsafe { JS_NewObject(cx, ptr::null_mut()) });

        let (tag, payload_type) = ty.cases().nth(discriminant.try_into().unwrap()).unwrap();

        rooted!(&in(cx) let tag = StringValue(unsafe {
            &*JS_NewStringCopyUTF8N(cx, &*Utf8Chars::from(tag))
        }));
        set(cx, wrapper.handle(), c"tag", tag.handle());

        if payload_type.is_some() {
            rooted!(&in(cx) let value = self.pop());
            set(cx, wrapper.handle(), c"val", value.handle());
        }

        self.push(ObjectValue(wrapper.get()));
    }

    fn push_option(&mut self, ty: WitOption, is_some: bool) {
        if is_some {
            if let Type::Option(_) = ty.ty() {
                let cx = &mut context();
                rooted!(&in(cx) let wrapper = unsafe { JS_NewObject(cx, ptr::null_mut()) });
                rooted!(&in(cx) let value = self.pop());
                set(cx, wrapper.handle(), c"val", value.handle());
                self.push(ObjectValue(wrapper.get()));
            } else {
                // Leave payload on the stack as-is.
            }
        } else {
            self.push(UndefinedValue());
        }
    }

    fn push_result(&mut self, ty: WitResult, is_err: bool) {
        let cx = &mut context();
        rooted!(&in(cx) let wrapper = unsafe { JS_NewObject(cx, ptr::null_mut()) });

        let tag = if is_err { "err" } else { "ok" };
        rooted!(&in(cx) let tag = StringValue(unsafe {
            &*JS_NewStringCopyUTF8N(cx, &*Utf8Chars::from(tag))
        }));
        set(cx, wrapper.handle(), c"tag", tag.handle());

        if (is_err && ty.err().is_some()) || (!is_err && ty.ok().is_some()) {
            rooted!(&in(cx) let value = self.pop());
            set(cx, wrapper.handle(), c"val", value.handle());
        }

        self.push(ObjectValue(wrapper.get()));
    }

    unsafe fn push_raw_list(&mut self, ty: List, src: *mut u8, len: usize) -> bool {
        if use_typed_array(ty.ty()) {
            self.push(unsafe { create_typed_array(&mut context(), ty.ty(), src, len) });
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
        rooted!(&in(cx) let mut push = get(cx, list.handle(), c"push"));
        rooted!(&in(cx) let params = vec![element.get()]);
        call(
            cx,
            list.handle(),
            push.handle(),
            &HandleValueArray::from(&params),
        );
    }
}

wit_dylib_ffi::export!(MyInterpreter);

fn imported_resource_from_canon(
    cx: &mut JSContext,
    index: usize,
    handle: u32,
    dispose: Option<JsFunction>,
    ty: Option<wit::Resource>,
) -> *mut JSObject {
    let wrapper = if let Some(ty) = ty {
        let module = MODULES
            .try_lock()
            .unwrap()
            .0
            .get(ty.interface().unwrap_or("witWorld"))
            .unwrap()
            .get();
        rooted!(&in(cx) let mut module = module);
        rooted!(&in(cx) let mut object = unsafe {
            mozjs::rust::wrappers2::GetModuleNamespace(cx, module.handle())
        });
        let class = get(
            cx,
            object.handle(),
            &CString::new(ty.name().to_upper_camel_case()).unwrap(),
        )
        .to_object();
        rooted!(&in(cx) let class = class);
        let proto = get(cx, class.handle(), c"prototype").to_object();
        rooted!(&in(cx) let proto = proto);
        unsafe { JS_NewObjectWithGivenProto(cx, ptr::null_mut(), proto.handle()) }
    } else {
        unsafe { JS_NewObject(cx, ptr::null_mut()) }
    };
    rooted!(&in(cx) let wrapper = wrapper);

    rooted!(&in(cx) let index = UInt32Value(index.try_into().unwrap()));
    set(cx, wrapper.handle(), TYPE_FIELD_NAME, index.handle());

    if let Some(dispose) = dispose {
        rooted!(&in(cx) let mut dispose = wrap(cx, dispose));
        set_with_symbol(cx, wrapper.handle(), SymbolCode::dispose, dispose.handle());
    }

    register_resource(cx, wrapper.handle(), handle);

    wrapper.get()
}

fn exported_resource_to_canon(
    cx: &mut JSContext,
    ty: wit::Resource,
    new: unsafe extern "C" fn(usize) -> u32,
    value: Value,
) -> u32 {
    rooted!(&in(cx) let value = value.to_object());
    rooted!(&in(cx) let mut handle = get(cx, value.handle(), HANDLE_FIELD_NAME));

    if handle.is_int32() {
        handle.to_int32() as u32
    } else {
        let rep = EXPORTED_RESOURCES
            .try_lock()
            .unwrap()
            .0
            .insert(Heap::boxed(value.get()));

        let handle = unsafe { new(rep) };

        rooted!(&in(cx) let index = UInt32Value(ty.index().try_into().unwrap()));
        set(cx, value.handle(), TYPE_FIELD_NAME, index.handle());

        register_resource(cx, value.handle(), handle);

        handle
    }
}

unsafe extern "C" fn trace_roots(tracer: *mut JSTracer, _: *mut c_void) {
    for traced in MY_CALL_TRACED.try_lock().unwrap().0.iter() {
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

    for traced in TRANSMIT_TRACED.try_lock().unwrap().0.iter() {
        let mut traced = traced.0.try_lock().unwrap();

        unsafe {
            CallObjectTracer(
                tracer,
                traced.wrapper.ptr.get() as *mut _,
                GCTraceKindToAscii(TraceKind::Object),
            )
        }

        unsafe {
            CallObjectTracer(
                tracer,
                traced.promise.ptr.get() as *mut _,
                GCTraceKindToAscii(TraceKind::Object),
            )
        }

        if let Some(resources) = traced.resources.as_mut() {
            for resources in resources.iter_mut() {
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

    for value in MODULES.try_lock().unwrap().0.values() {
        unsafe {
            CallObjectTracer(
                tracer,
                value.ptr.get() as *mut _,
                GCTraceKindToAscii(TraceKind::Object),
            )
        }
    }

    if let Some(value) = MAIN_MODULE.try_lock().unwrap().as_ref() {
        unsafe {
            CallObjectTracer(
                tracer,
                value.0.ptr.get() as *mut _,
                GCTraceKindToAscii(TraceKind::Object),
            )
        }
    }
}

fn mangle_name(name: &str) -> String {
    name.replace(['@', ':', '/', '-', '[', ']', '.'], "_")
        .to_lower_camel_case()
}

fn use_typed_array(ty: Type) -> bool {
    matches!(
        ty,
        Type::U8
            | Type::S8
            | Type::U16
            | Type::S16
            | Type::U32
            | Type::S32
            | Type::U64
            | Type::S64
            | Type::F32
            | Type::F64
    )
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
