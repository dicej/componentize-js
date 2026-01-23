#![deny(warnings)]

use {
    anyhow::{Context as _, Result, anyhow, bail},
    std::{
        env,
        fs::File,
        io, iter,
        path::{Path, PathBuf},
        process::Command,
    },
    zstd::Encoder,
};

const ZSTD_COMPRESSION_LEVEL: i32 = 19;

#[cfg(target_os = "windows")]
const CLANG_EXECUTABLE: &str = "clang.exe";
#[cfg(not(target_os = "windows"))]
const CLANG_EXECUTABLE: &str = "clang";

fn main() -> Result<()> {
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

fn stubs_for_clippy(out_dir: &Path) -> Result<()> {
    println!(
        "cargo:warning=using stubbed runtime, core library, and adapter for static analysis purposes..."
    );

    let files = [
        "libcomponentize_js_runtime.so.zst",
        "libc.so.zst",
        "libwasi-emulated-mman.so.zst",
        "libwasi-emulated-process-clocks.so.zst",
        "libwasi-emulated-getpid.so.zst",
        "libwasi-emulated-signal.so.zst",
        "libc++.so.zst",
        "libc++abi.so.zst",
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

fn package_all_the_things(out_dir: &Path) -> Result<()> {
    let repo_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());

    let wasi_sdk =
        PathBuf::from(env::var_os("WASI_SDK_PATH").unwrap_or_else(|| "/opt/wasi-sdk".into()));

    make_runtime(out_dir, &wasi_sdk, "libcomponentize_js_runtime.so")?;

    let libraries = [
        "libc.so",
        "libwasi-emulated-mman.so",
        "libwasi-emulated-process-clocks.so",
        "libwasi-emulated-getpid.so",
        "libwasi-emulated-signal.so",
        "libc++.so",
        "libc++abi.so",
    ];

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

fn compress(src_dir: &Path, name: &str, dst_dir: &Path, rerun_if_changed: bool) -> Result<()> {
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

fn make_runtime(out_dir: &Path, wasi_sdk: &Path, name: &str) -> Result<()> {
    let mut cmd = Command::new("rustup");
    cmd.current_dir("runtime")
        .arg("run")
        .arg("nightly")
        .arg("cargo")
        .arg("build")
        .arg("-Z")
        .arg("build-std=panic_abort,std")
        .arg("--release")
        .arg("--target=wasm32-wasip1");

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

    let path = out_dir.join("wasm32-wasip1/release/libcomponentize_js_runtime.a");

    if path.exists() {
        let clang = wasi_sdk.join(format!("bin/{CLANG_EXECUTABLE}"));
        if clang.exists() {
            run(Command::new(clang)
                .arg("-shared")
                .arg("-o")
                .arg(out_dir.join(name))
                .arg("-Wl,--whole-archive")
                .arg(&path)
                .arg("-Wl,--no-whole-archive"))?;

            compress(out_dir, name, out_dir, false)?;
        } else {
            bail!("no such file: {}", clang.display())
        }
    } else {
        bail!("no such file: {}", path.display())
    }

    Ok(())
}

fn run(command: &mut Command) -> Result<Vec<u8>> {
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
