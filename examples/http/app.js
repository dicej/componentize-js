import { Request, Response, Fields } from "wasi:http/types@0.3.0-rc-2026-01-06"
import * as client from "wasi:http/client@0.3.0-rc-2026-01-06"
import * as witWorld from "wit-world"
import { IncrementalSHA256 as Sha256 } from "./sha256.js"

const decoder = new TextDecoder()
const encoder = new TextEncoder()

export const wasiHttpHandler030Rc20260106 = {
    handle: async function(request) {
        const method = request.getMethod().tag
        const path = request.getPathWithQuery()
        const headers = request.getHeaders().copyAll()

        if (method === "get" && path === "/hash-all") {
            const urls = headers
                  .filter(([k, _]) => k === "url")
                  .map(([_, v]) => decoder.decode(v))

            const [tx, rx] = witWorld.u8Stream()
            hashAll(urls, tx).catch((error) => _componentizeJsLog(error.toString()))

            return Response.new(
                Fields.fromList([["content-type", encoder.encode("text/plain")]]),
                rx,
                trailersFuture()
            )[0]
        } else if (method === "post" && path === "/echo") {
            const [rx, trailers] = Request.consumeBody(request, unitFuture())

            return Response.new(
                Fields.fromList(headers.filter(([k, _]) => k === "content-type")),
                rx,
                trailers
            )[0]
        } else {
            const response = Response.new(new Fields(), undefined, trailersFuture())[0]
            response.setStatusCode(400)
            return response
        }
    }
}

async function hashAll(urls, tx) {
    let promises = urls.map((url) => [url, sha256(url)])
    while (promises.length > 0) {
        const [url, hash] = await Promise.race(promises.map(([_, v]) => v))
        promises = promises.filter(([k, _]) => k !== url)
        await tx.writeAll(encoder.encode(`${url}: ${hash}\n`))
    }
    tx[_componentizeJsSymbolDispose]()
}

async function sha256(url) {
    // TODO: use a proper URL parser
    const schemeDelimiter = url.indexOf("://")
    if (schemeDelimiter === -1) {
        return [url, "unable to parse URL"]
    }
    const schemeString = url.substring(0, schemeDelimiter)
    const remaining = url.substring(schemeDelimiter + 3)
    const authorityDelimiter = remaining.indexOf("/")
    const authority = authorityDelimiter === -1 ? remaining : remaining.substring(0, authorityDelimiter)
    const path = authorityDelimiter === -1 ? "/" : remaining.substring(authorityDelimiter)

    let scheme
    switch (schemeString) {
    case "http":
        scheme = { tag: "http" }
        break
    case "http":
        scheme = { tag: "https" }
        break
    default:
        scheme = { tag: "other", val: schemeString }
        break
    }

    const request = Request.new(new Fields(), undefined, trailersFuture(), undefined)[0]
    request.setScheme(scheme)
    request.setAuthority(authority)
    request.setPathWithQuery(path)

    const response = await client.send(request)
    const status = response.getStatusCode()
    if (status < 200 || status > 299) {
        return [url, `unexpected status: ${status}`]
    }

    const rx = Response.consumeBody(response, unitFuture())[0]

    const hasher = new Sha256()
    while (!rx.writerDropped) {
        const chunk = await rx.read(16 * 1024)
        hasher.update(chunk)
    }
    return [url, hasher.digest()]
}

function trailersFuture() {
    const [tx, rx] = witWorld.resultOptionWasiHttpTypes030Rc20260106FieldsWasiHttpTypes030Rc20260106ErrorCodeFuture()
    tx.write({ tag: 'ok' })
    return rx
}

function unitFuture() {
    const [tx, rx] = witWorld.resultUnitWasiHttpTypes030Rc20260106ErrorCodeFuture()
    tx.write({ tag: 'ok' })
    return rx
}
