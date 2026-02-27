#![deny(warnings)]

use {
    anyhow::{Context as _, anyhow, bail},
    std::{
        env,
        fs::{self, File},
        io, iter, mem,
        path::{Path, PathBuf},
        process::Command,
    },
    wasm_encoder::{ComponentSectionId, Encode, RawSection, Section},
    wasmparser::{Parser, Payload},
    zstd::Encoder,
};

const DEBUG_RUNTIME: bool = true;
const STRIP_RUNTIME: bool = !DEBUG_RUNTIME;
const ZSTD_COMPRESSION_LEVEL: i32 = if DEBUG_RUNTIME { 0 } else { 19 };

#[cfg(target_os = "windows")]
const CLANG_EXECUTABLE: &str = "clang.exe";
#[cfg(not(target_os = "windows"))]
const CLANG_EXECUTABLE: &str = "clang";

fn main() -> anyhow::Result<()> {
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());

    if matches!(env::var("CARGO_CFG_FEATURE").as_deref(), Ok("cargo-clippy"))
        || env::var("CLIPPY_ARGS").is_ok()
        || env::var("CARGO_EXPAND_NO_RUN_NIGHTLY").is_ok()
    {
        stubs_for_clippy(&out_dir)
    } else {
        package_all_the_things(&out_dir)
    }
}

fn stubs_for_clippy(out_dir: &Path) -> anyhow::Result<()> {
    println!(
        "cargo:warning=using stubbed runtime, core library, and adapter for static analysis purposes..."
    );

    let files = [
        "libcomponentize_js_runtime.so.zst",
        "libc.so.zst",
        "libwasi-emulated-getpid.so.zst",
        "wasi_snapshot_preview1.reactor.wasm.zst",
    ];

    for file in files {
        let path = out_dir.join(file);

        if !path.exists() {
            Encoder::new(File::create(path)?, ZSTD_COMPRESSION_LEVEL)?.do_finish()?;
        }
    }

    Ok(())
}

fn package_all_the_things(out_dir: &Path) -> anyhow::Result<()> {
    let repo_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());

    let wasi_sdk =
        PathBuf::from(env::var_os("WASI_SDK_PATH").unwrap_or_else(|| "/opt/wasi-sdk".into()));

    make_runtime(out_dir, &wasi_sdk, "libcomponentize_js_runtime.so")?;

    let libraries = ["libc.so", "libwasi-emulated-getpid.so"];

    for library in libraries {
        compress(
            &wasi_sdk.join("share/wasi-sysroot/lib/wasm32-wasip2"),
            library,
            out_dir,
            true,
        )?;
    }

    compress(
        &repo_dir.join("adapters/3dda9169"),
        "wasi_snapshot_preview1.reactor.wasm",
        out_dir,
        false,
    )?;

    Ok(())
}

fn compress(
    src_dir: &Path,
    name: &str,
    dst_dir: &Path,
    rerun_if_changed: bool,
) -> anyhow::Result<()> {
    let path = src_dir.join(name);

    if rerun_if_changed {
        println!("cargo:rerun-if-changed={}", path.to_str().unwrap());
    }

    if path.exists() {
        let mut encoder = Encoder::new(
            File::create(dst_dir.join(format!("{name}.zst")))?,
            ZSTD_COMPRESSION_LEVEL,
        )?;
        io::copy(&mut File::open(path)?, &mut encoder)?;
        encoder.do_finish()?;
        Ok(())
    } else {
        Err(anyhow!("no such file: {}", path.display()))
    }
}

fn make_runtime(out_dir: &Path, wasi_sdk: &Path, name: &str) -> anyhow::Result<()> {
    let mut cmd = Command::new("rustup");
    cmd.current_dir("runtime")
        .arg("run")
        .arg("nightly")
        .arg("cargo")
        .arg("build")
        .arg("-Z")
        .arg("build-std=panic_abort,std")
        .arg("--target=wasm32-wasip1");

    if !DEBUG_RUNTIME {
        cmd.arg("--release");
    }

    for (key, _) in env::vars_os() {
        if key
            .to_str()
            .map(|key| key.starts_with("RUST") || key.starts_with("CARGO"))
            .unwrap_or(false)
        {
            cmd.env_remove(&key);
        }
    }

    cmd.env("RUSTFLAGS", "-C relocation-model=pic")
        .env("CARGO_TARGET_DIR", out_dir)
        .env("MOZJS_FROM_SOURCE", "1");

    let status = cmd.status()?;
    assert!(status.success());
    println!("cargo:rerun-if-changed=runtime");

    let build = if DEBUG_RUNTIME { "debug" } else { "release" };
    let path = out_dir.join(format!(
        "wasm32-wasip1/{build}/libcomponentize_js_runtime.a"
    ));

    if path.exists() {
        let clang = wasi_sdk.join(format!("bin/{CLANG_EXECUTABLE}"));
        if clang.exists() {
            run(Command::new(clang)
                .arg("-shared")
                .arg("-o")
                .arg(out_dir.join(name))
                .arg("-Wl,--whole-archive")
                .arg(&path)
                .arg("-Wl,--no-whole-archive")
                .arg("-lwasi-emulated-getpid"))?;

            if STRIP_RUNTIME {
                fs::write(out_dir.join(name), &strip(&fs::read(out_dir.join(name))?)?)?;
            }

            compress(out_dir, name, out_dir, false)?;
        } else {
            bail!("no such file: {}", clang.display())
        }
    } else {
        bail!("no such file: {}", path.display())
    }

    Ok(())
}

fn strip(input: &[u8]) -> anyhow::Result<Vec<u8>> {
    // Adapted from https://github.com/bytecodealliance/wasm-tools/blob/main/src/bin/wasm-tools/strip.rs
    //
    // TODO: Move that code into e.g. `wasm_encoder` so we can reuse it here
    // instead of duplicating it.

    let mut output = Vec::new();
    let mut stack = Vec::new();

    for payload in Parser::new(0).parse_all(input) {
        let payload = payload?;

        // Track nesting depth, so that we don't mess with inner producer sections:
        match payload {
            Payload::Version { encoding, .. } => {
                output.extend_from_slice(match encoding {
                    wasmparser::Encoding::Component => &wasm_encoder::Component::HEADER,
                    wasmparser::Encoding::Module => &wasm_encoder::Module::HEADER,
                });
            }
            Payload::ModuleSection { .. } | Payload::ComponentSection { .. } => {
                stack.push(mem::take(&mut output));
                continue;
            }
            Payload::End { .. } => {
                let mut parent = match stack.pop() {
                    Some(c) => c,
                    None => break,
                };
                if output.starts_with(&wasm_encoder::Component::HEADER) {
                    parent.push(ComponentSectionId::Component as u8);
                    output.encode(&mut parent);
                } else {
                    parent.push(ComponentSectionId::CoreModule as u8);
                    output.encode(&mut parent);
                }
                output = parent;
            }
            _ => {}
        }

        if let Payload::CustomSection(ref c) = payload {
            let name = c.name();
            if name != "name" && !name.starts_with("component-type:") && name != "dylink.0" {
                continue;
            }
        }

        if let Some((id, range)) = payload.as_section() {
            RawSection {
                id,
                data: &input[range],
            }
            .append_to(&mut output);
        }
    }

    Ok(output)
}

fn run(command: &mut Command) -> anyhow::Result<Vec<u8>> {
    let command_string = iter::once(command.get_program())
        .chain(command.get_args())
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");

    let output = command.output().with_context({
        let command_string = command_string.clone();
        move || command_string
    })?;

    if output.status.success() {
        Ok(output.stdout)
    } else {
        bail!(
            "command `{command_string}` failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
