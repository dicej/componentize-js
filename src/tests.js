import * as echoes from "componentize-js:tests/echoes"
import * as simpleImportAndExport from "componentize-js:tests/simple-import-and-export"
import * as simpleAsyncImportAndExport from "componentize-js:tests/simple-async-import-and-export"
import * as hostThingInterface from "componentize-js:tests/host-thing-interface"
import * as witWorld from "wit-world"

// TODO: As of this writing, the version of SpiderMonkey `mozjs` uses (140.x) is
// just barely too old to support `using` declarations.  Once we've upgraded, we
// should update the following code to use them instead of calling
// `_componentizeJsSymbolDispose` functions directly.

async function pipeBytes(rx, tx) {
    while (!(rx.writerDropped || tx.readerDropped)) {
        await tx.writeAll(await rx.read(1024))
    }

    tx[_componentizeJsSymbolDispose]()
    rx[_componentizeJsSymbolDispose]()
}

async function pipeStrings(rx, tx) {
    // TODO: The version of SpiderMonkey we're using doesn't appear to support
    // the `using` syntax, otherwise we would use it here.
    await tx.write(await rx.read())

    tx[_componentizeJsSymbolDispose]()
    rx[_componentizeJsSymbolDispose]()
}

async function pipeThings(rx, tx, class_) {
    // TODO: The version of SpiderMonkey we're using doesn't appear to support
    // the `using` syntax, otherwise we would use it here.

    // Read the things one at a time, forcing the host to re-take ownership of
    // any unwritten items between writes.
    let things = []
    while (!rx.writerDropped) {
        things.push(...await rx.read(1))
    }

    let strings = ["a", "b", "c", "d", "e"];
    if (things.length !== strings.length) {
        throw `expected ${strings.length} things; got ${things.length}`
    }
    
    for (let i = 0; i < things.length; ++i) {
        if (!(things[i] instanceof class_)) {
            throw `expected ${class_.name}; got ${things[i].constructor.name}`
        }
        
        let s = await things[i].get()
        if (s !== strings[i]) {
            throw `expected ${strings[i]}; got ${s}`
        }

        s = await class_.getStatic(things[i])
        if (s !== strings[i]) {
            throw `expected ${strings[i]}; got ${s}`
        }
    }

    // Write the things all at once.  The host will read them only one at a time,
    // forcing us to re-take ownership of any unwritten items between writes.
    await tx.writeAll(things)

    tx[_componentizeJsSymbolDispose]()
    rx[_componentizeJsSymbolDispose]()
}

async function writeThing(thing, tx1, tx2) {
    // TODO: The version of SpiderMonkey we're using doesn't appear to support
    // the `using` syntax, otherwise we would use it here.

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

    thing[_componentizeJsSymbolDispose]()
    tx1[_componentizeJsSymbolDispose]()
    tx2[_componentizeJsSymbolDispose]()
}

export const componentizeJsTestsSimpleExport = {
    foo: function(v) {
        return v + 3
    }
}

export const componentizeJsTestsSimpleAsyncExport = {
    foo: function(v) {
        return Promise.resolve(v + 3)
    }
}

export const componentizeJsTestsSimpleImportAndExport = {
    foo: function(v) {
        return simpleImportAndExport.foo(v + 3)
    }
}

export const componentizeJsTestsSimpleAsyncImportAndExport = {
    foo: function(v) {
        return simpleAsyncImportAndExport.foo(v + 3)
    }
}

export const componentizeJsTestsEchoes = {
    echoNothing: function() {
        return echoes.echoNothing()
    },
    echoBool: function(v) {
        return echoes.echoBool(v)
    },
    echoU8: function(v) {
        return echoes.echoU8(v)
    },
    echoS8: function(v) {
        return echoes.echoS8(v)
    },
    echoU16: function(v) {
        return echoes.echoU16(v)
    },
    echoS16: function(v) {
        return echoes.echoS16(v)
    },
    echoU32: function(v) {
        return echoes.echoU32(v)
    },
    echoS32: function(v) {
        return echoes.echoS32(v)
    },
    echoU64: function(v) {
        return echoes.echoU64(v)
    },
    echoS64: function(v) {
        return echoes.echoS64(v)
    },
    echoChar: function(v) {
        return echoes.echoChar(v)
    },
    echoF32: function(v) {
        return echoes.echoF32(v)
    },
    echoF64: function(v) {
        return echoes.echoF64(v)
    },
    echoString: function(v) {
        return echoes.echoString(v)
    },
    echoListBool: function(v) {
        return echoes.echoListBool(v)
    },
    echoListU8: function(v) {
        return echoes.echoListU8(v)
    },
    echoListListU8: function(v) {
        return echoes.echoListListU8(v)
    },
    echoListListListU8: function(v) {
        return echoes.echoListListListU8(v)
    },
    echoOptionU8: function(v) {
        return echoes.echoOptionU8(v)
    },
    echoOptionOptionU8: function(v) {
        return echoes.echoOptionOptionU8(v)
    },
    echoResultU8U8: function(v) {
        return echoes.echoResultU8U8(v)
    },
    echoResultResultU8U8U8: function(v) {
        return echoes.echoResultResultU8U8U8(v)
    },
    echoListS8: function(v) {
        return echoes.echoListS8(v)
    },
    echoListU16: function(v) {
        return echoes.echoListU16(v)
    },
    echoListS16: function(v) {
        return echoes.echoListS16(v)
    },
    echoListU32: function(v) {
        return echoes.echoListU32(v)
    },
    echoListS32: function(v) {
        return echoes.echoListS32(v)
    },
    echoListU64: function(v) {
        return echoes.echoListU64(v)
    },
    echoListS64: function(v) {
        return echoes.echoListS64(v)
    },
    echoListChar: function(v) {
        return echoes.echoListChar(v)
    },
    echoListF32: function(v) {
        return echoes.echoListF32(v)
    },
    echoListF64: function(v) {
        return echoes.echoListF64(v)
    },
    echoMany: function(v1, v2, v3, v4, v5, v6, v7, v8, v9, v10, v11, v12, v13, v14, v15, v16) {
        return echoes.echoMany(
            v1, v2, v3, v4, v5, v6, v7, v8, v9, v10, v11, v12, v13, v14, v15, v16
        )
    },
    echoResource: function(v) {
        return echoes.echoResource(v)
    },
    acceptBorrow: function(v) {
        return echoes.acceptBorrow(v)
    },
    echoRecord: function(v) {
        return echoes.echoRecord(v)
    },
    echoEnum: function(v) {
        return echoes.echoEnum(v)
    },
    echoFlags: function(v) {
        return echoes.echoFlags(v)
    },
    echoVariant: function(v) {
        return echoes.echoVariant(v)
    },
    echoStream: function(v) {
        return echoes.echoStream(v)
    },
    echoFuture: function(v) {
        return echoes.echoFuture(v)
    }
}

class Thing {
    constructor(value) {
        this.value = value
    }
    async get(delay) {
        if (delay) {
            await witWorld.delay()
        }
        return this.value
    }
    static async getStatic(v, delay) {
        if (delay) {
            await witWorld.delay()
        }
        return v.value
    }
    [_componentizeJsSymbolDispose]() {}
}

export const componentizeJsTestsStreamsAndFutures = {
    echoStreamU8: function(stream) {
        let [tx, rx] = witWorld.u8Stream()
        pipeBytes(stream, tx)
            .catch((error) => _componentizeJsLog(error.toString()))
        return Promise.resolve(rx)
    },
    echoFutureString: function(future) {
        let [tx, rx] = witWorld.stringFuture()
        pipeStrings(future, tx)
            .catch((error) => _componentizeJsLog(error.toString()))
        return Promise.resolve(rx)
    },
    shortReads: function(stream) {
        let [tx, rx] = witWorld.componentizeJsTestsStreamsAndFuturesThingStream()
        pipeThings(stream, tx, Thing)
            .catch((error) => _componentizeJsLog(error.toString()))
        return Promise.resolve(rx)
    },
    shortReadsHost: function(stream) {
        let [tx, rx] = witWorld.componentizeJsTestsHostThingInterfaceHostThingStream()
        pipeThings(stream, tx, hostThingInterface.HostThing)
            .catch((error) => _componentizeJsLog(error.toString()))
        return Promise.resolve(rx)            
    },
    droppedFutureReader: function(value) {
        let [tx1, rx1] = witWorld.componentizeJsTestsStreamsAndFuturesThingFuture()
        let [tx2, rx2] = witWorld.componentizeJsTestsStreamsAndFuturesThingFuture()
        writeThing(new Thing(value), tx1, tx2).catch((error) => _componentizeJsLog(error.toString()))
        return Promise.resolve([rx1, rx2])
    },
    droppedFutureReaderHost: function(value) {
        let [tx1, rx1] = witWorld.componentizeJsTestsHostThingInterfaceHostThingFuture()
        let [tx2, rx2] = witWorld.componentizeJsTestsHostThingInterfaceHostThingFuture()
        writeThing(new hostThingInterface.HostThing(value), tx1, tx2)
            .catch((error) => _componentizeJsLog(error.toString()))
        return Promise.resolve([rx1, rx2])
    },
    Thing
}
