// `ComponentError` is used to represent `err` `result` values.  Shamelessly
// stolen from
// https://github.com/bytecodealliance/jco/blob/bb56a3e2a30cc107c408a84591ef8788e3abbdf5/crates/js-component-bindgen/src/intrinsics/mod.rs#L281-L287
var ComponentError = class extends Error {
    constructor(value) {
        const enumerable = typeof value !== 'string';
        super(enumerable ? `${String(value)} (see error.payload)` : value);
        Object.defineProperty(this, 'payload', { value, enumerable });
    }
}

var TextEncoder = class {
    constructor() {}
    encode(value) { return _componentizeJsEncodeUtf8(value) }
}

var TextDecoder = class {
    constructor() {}
    decode(value) { return _componentizeJsDecodeUtf8(value) }
}

var _componentizeJsWriteAll = async function(buffer) {
    let total = 0
    while (buffer.length > 0 && !this.readerDropped) {
        count = await this.write(buffer)
        buffer = buffer.slice(count)
        total += count
    }
    return total
}

var _componentizeJsMaybeWriteDefault = function() {
    if (this._componentizeJsHandle) {
        this.write(this.default())
    }
}

var _componentizeJsFinalizationRegistry = new FinalizationRegistry((v) => v())

var _componentizeJsRegisterFinalizer = function(value) {
    const clone = {
        _componentizeJsHandle: value._componentizeJsHandle,
        _componentizeJsType: value._componentizeJsType,
        default: value.default
    }
    const dispose = value[Symbol.dispose]
    _componentizeJsFinalizationRegistry.register(value, () => dispose.call(clone), value)
}

var _componentizeJsUnregisterFinalizer = function(value) {
    _componentizeJsFinalizationRegistry.unregister(value)
}
