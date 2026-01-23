#![deny(warnings)]
#![expect(
    clippy::not_unsafe_ptr_arg_deref,
    reason = "wit_dylib_ffi::export produces code that does this"
)]

use {
    std::{
        alloc::{self, Layout},
        marker::PhantomData,
        sync::OnceLock,
    },
    wit_dylib_ffi::{
        self as wit, Call, ExportFunction, Interpreter, List, Wit, WitOption, WitResult,
    },
};

static WIT: OnceLock<Wit> = OnceLock::new();

struct Borrow;
struct Object;
struct EmptyResource;

struct MyInterpreter;

impl MyInterpreter {
    fn export_call_(func: ExportFunction, cx: &mut MyCall<'_>, async_: bool) -> u32 {
        _ = (func, cx, async_);
        todo!()
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
    stack: Vec<Object>,
    resources: Option<Vec<EmptyResource>>,
}

impl MyCall<'_> {
    fn new(stack: Vec<Object>) -> Self {
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
        todo!()
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
        _ = val;
        todo!()
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
