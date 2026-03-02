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
        }
    },
}
