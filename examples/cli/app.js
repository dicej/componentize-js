import * as stdout from "wasi:cli/stdout@0.3.0-rc-2026-01-06"
import * as witWorld from "wit-world"

export const wasiCliRun030Rc20260106 = {
    run: async function() {
        // As of this writing, the `componentize-js` runtime doesn't have
        // `console.log`, so we must use the raw WASI bindings directly, which
        // is... verbose.
        
        let [tx, rx] = witWorld.u8Stream()
        let write = stdout.writeViaStream(rx)
        await tx.writeAll(new TextEncoder().encode("Hello, world!"))
        
        // Once the SpiderMonkey dep of the `mozjs` dep of `componentize-js` has
        // been updated to support explicit resource management, we'll be able
        // to use `using` declarations to dispose of streams.  For now, we must
        // do it manually:
        tx[_componentizeJsSymbolDispose]()
        
        await write
    }
}
