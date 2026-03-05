async function pipe_bytes(rx, tx) {
    while (!(rx.writer_dropped || tx.reader_dropped)) {
        await tx.write_all(await rx.read(1024))
    }
    tx.drop()
}

async function pipe_strings(rx, tx) {
    await tx.write(await rx.read())
}

async function pipe_things(rx, tx) {
    // Read the things one at a time, forcing the host to re-take ownership of
    // any unwritten items between writes.
    let things = []
    while (!rx.writer_dropped) {
        things.push(...await rx.read(1))
    }

    // Write the things all at once.  The host will read them only one at a time,
    // forcing us to re-take ownership of any unwritten items between writes.
    await tx.write_all(things)
    tx.drop()
}

async function write_thing(thing, tx1, tx2) {
    // The host will drop the first reader without reading, which should give us
    // back ownership of `thing`.
    let wrote = await tx1.write(thing)
    if (wrote) {
        throw Error()
    }
    // The host will read from the second reader, though.
    wrote = await tx2.write(thing)
    if (!wrote) {
        throw Error()
    }
}

var exports = {
    componentize_js_tests_simple_export: {
        foo: function(v) {
            return v + 3
        }
    },
    componentize_js_tests_simple_async_export: {
        foo: function(v) {
            return Promise.resolve(v + 3)
        }
    },
    componentize_js_tests_simple_import_and_export: {
        foo: function(v) {
            return imports.componentize_js_tests_simple_import_and_export.foo(v + 3)
        }
    },
    componentize_js_tests_simple_async_import_and_export: {
        foo: function(v) {
            return imports.componentize_js_tests_simple_async_import_and_export.foo(v + 3)
        }
    },
    componentize_js_tests_echoes: {
        echo_nothing: function() {
            return imports.componentize_js_tests_echoes.echo_nothing()
        },
        echo_bool: function(v) {
            return imports.componentize_js_tests_echoes.echo_bool(v)
        },
        echo_u8: function(v) {
            return imports.componentize_js_tests_echoes.echo_u8(v)
        },
        echo_s8: function(v) {
            return imports.componentize_js_tests_echoes.echo_s8(v)
        },
        echo_u16: function(v) {
            return imports.componentize_js_tests_echoes.echo_u16(v)
        },
        echo_s16: function(v) {
            return imports.componentize_js_tests_echoes.echo_s16(v)
        },
        echo_u32: function(v) {
            return imports.componentize_js_tests_echoes.echo_u32(v)
        },
        echo_s32: function(v) {
            return imports.componentize_js_tests_echoes.echo_s32(v)
        },
        echo_u64: function(v) {
            return imports.componentize_js_tests_echoes.echo_u64(v)
        },
        echo_s64: function(v) {
            return imports.componentize_js_tests_echoes.echo_s64(v)
        },
        echo_char: function(v) {
            return imports.componentize_js_tests_echoes.echo_char(v)
        },
        echo_f32: function(v) {
            return imports.componentize_js_tests_echoes.echo_f32(v)
        },
        echo_f64: function(v) {
            return imports.componentize_js_tests_echoes.echo_f64(v)
        },
        echo_string: function(v) {
            return imports.componentize_js_tests_echoes.echo_string(v)
        },
        echo_list_bool: function(v) {
            return imports.componentize_js_tests_echoes.echo_list_bool(v)
        },
        echo_list_u8: function(v) {
            return imports.componentize_js_tests_echoes.echo_list_u8(v)
        },
        echo_list_list_u8: function(v) {
            return imports.componentize_js_tests_echoes.echo_list_list_u8(v)
        },
        echo_list_list_list_u8: function(v) {
            return imports.componentize_js_tests_echoes.echo_list_list_list_u8(v)
        },
        echo_option_u8: function(v) {
            return imports.componentize_js_tests_echoes.echo_option_u8(v)
        },
        echo_option_option_u8: function(v) {
            return imports.componentize_js_tests_echoes.echo_option_option_u8(v)
        },
        echo_list_s8: function(v) {
            return imports.componentize_js_tests_echoes.echo_list_s8(v)
        },
        echo_list_u16: function(v) {
            return imports.componentize_js_tests_echoes.echo_list_u16(v)
        },
        echo_list_s16: function(v) {
            return imports.componentize_js_tests_echoes.echo_list_s16(v)
        },
        echo_list_u32: function(v) {
            return imports.componentize_js_tests_echoes.echo_list_u32(v)
        },
        echo_list_s32: function(v) {
            return imports.componentize_js_tests_echoes.echo_list_s32(v)
        },
        echo_list_u64: function(v) {
            return imports.componentize_js_tests_echoes.echo_list_u64(v)
        },
        echo_list_s64: function(v) {
            return imports.componentize_js_tests_echoes.echo_list_s64(v)
        },
        echo_list_char: function(v) {
            return imports.componentize_js_tests_echoes.echo_list_char(v)
        },
        echo_list_f32: function(v) {
            return imports.componentize_js_tests_echoes.echo_list_f32(v)
        },
        echo_list_f64: function(v) {
            return imports.componentize_js_tests_echoes.echo_list_f64(v)
        },
        echo_many: function(v1, v2, v3, v4, v5, v6, v7, v8, v9, v10, v11, v12, v13, v14, v15, v16) {
            return imports.componentize_js_tests_echoes.echo_many(
                v1, v2, v3, v4, v5, v6, v7, v8, v9, v10, v11, v12, v13, v14, v15, v16
            )
        },
        echo_resource: function(v) {
            return imports.componentize_js_tests_echoes.echo_resource(v)
        },
        accept_borrow: function(v) {
            return imports.componentize_js_tests_echoes.accept_borrow(v)
        },
        echo_record: function(v) {
            return imports.componentize_js_tests_echoes.echo_record(v)
        },
        echo_enum: function(v) {
            return imports.componentize_js_tests_echoes.echo_enum(v)
        },
        echo_flags: function(v) {
            return imports.componentize_js_tests_echoes.echo_flags(v)
        },
        echo_variant: function(v) {
            return imports.componentize_js_tests_echoes.echo_variant(v)
        },
        echo_stream: function(v) {
            return imports.componentize_js_tests_echoes.echo_stream(v)
        },
        echo_future: function(v) {
            return imports.componentize_js_tests_echoes.echo_future(v)
        }
    },
    componentize_js_tests_streams_and_futures: {
        echo_stream_u8: function(stream) {
            let [tx, rx] = types.u8_stream()
            pipe_bytes(stream, tx)
            return Promise.resolve(rx)
        },
        echo_future_string: function(future) {
            let [tx, rx] = types.string_future()
            pipe_strings(future, tx)
            return Promise.resolve(rx)
        },
        short_reads: function(stream) {
            let [tx, rx] = types.componentize_js_tests_streams_and_futures_thing_stream()
            pipe_things(stream, tx)
            return Promise.resolve(rx)
        },
        short_reads_host: function(stream) {
            let [tx, rx] = types.componentize_js_tests_host_thing_interface_host_thing_stream()
            pipe_things(stream, tx)
            return Promise.resolve(rx)            
        },
        dropped_future_reader: function(value) {
            let [tx1, rx1] = types.componentize_js_tests_streams_and_futures_thing_future()
            let [tx2, rx2] = types.componentize_js_tests_streams_and_futures_thing_future()
            write_thing({ value }, tx1, tx2)
            return Promise.resolve([rx1, rx2])
        },
        dropped_future_reader_host: function(value) {
            let [tx1, rx1] = types.componentize_js_tests_host_thing_interface_host_thing_future()
            let [tx2, rx2] = types.componentize_js_tests_host_thing_interface_host_thing_future()
            write_thing(
                imports.componentize_js_tests_host_thing_interface._constructor_host_thing(value),
                tx1,
                tx2
            )
            return Promise.resolve([rx1, rx2])
        },
        _constructor_thing: function(value) {
            return { value }
        },
        _method_thing_get: async function(thing, delay) {
            if (delay) {
                await imports.delay()
            }
            return thing.value
        }
    }
}
