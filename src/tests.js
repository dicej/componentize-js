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
        }
    },
}
