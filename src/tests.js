var exports = {
    componentize_js_tests_simple_export: {
        foo: function(v) {
            return v + 3
        }
    },
    componentize_js_tests_simple_import_and_export: {
        foo: function(v) {
            return imports.componentize_js_tests_simple_import_and_export.foo(v + 3)
        }
    }
}
