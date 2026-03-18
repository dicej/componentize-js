import * as stdout from "wasi:cli/stdout@0.3.0-rc-2026-01-06"
import * as witWorld from "wit-world"

export const wasiCliRun030Rc20260106 = {
    run: async function() {
        // As of this writing, the `componentize-js` runtime doesn't have
        // `console.log`, so we must use the raw WASI bindings directly, which
        // is... verbose.
        
        const [tx, rx] = witWorld.u8Stream()
        using _tx = tx, _rx = rx
        const write = stdout.writeViaStream(rx)
        await tx.writeAll(new TextEncoder().encode("Hello, world!"))
        tx[Symbol.dispose]()
        await write
    }
}
