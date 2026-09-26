// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Everything `xtask` does to the one crate that links an import: build it,
//! test it on the host, compile it for AArch64, and link it into a component
//! image and measure what comes out. RFC 0082, RFC 0141.
//!
//! # Why a module of its own
//!
//! `user/shaper` is outside the workspace (`IMPORT_LINKERS`, the root
//! `Cargo.toml`'s `exclude`), so none of the workspace verbs reach it: `cargo
//! test --workspace` does not test it, `cargo clippy --workspace` does not lint
//! it, and [`crate::COMPONENTS`] does not build it. Each of those verbs calls
//! into this module instead, so the crate is held to the same checks as every
//! other one without joining the workspace it is kept out of — and the one
//! place a source replacement pointed into `third_party/` is spelled is
//! [`cargo`], under `TOOLING`.
//!
//! # Where HarfRust comes from
//!
//! From the import's `vendor/` directory and nowhere else: [`cargo`] passes
//! cargo's source replacement for `crates-io` on the command line, with
//! `--offline --locked`, so a crate that is not vendored is a resolution
//! failure rather than a download, a lockfile that does not match is a refusal
//! rather than a rewrite, and cargo verifies every vendored file against the
//! `.cargo-checksum.json` `cargo vendor` wrote. `lint-licensing` verifies the
//! same files on every lint without a build (`imported::source_findings`).
//!
//! # Why the image is linked the way it is
//!
//! Every other component is built by [`crate::flat_image_with`] with
//! `-Zbuild-std` and `panic=immediate-abort`. This one cannot be: `-Zbuild-std`
//! resolves the standard library's own lockfile, and with `crates-io` replaced
//! by the import's `vendor/` the standard library's dependencies are not there
//! to find. So the shim is built against the prebuilt `core` and `alloc` that
//! `rust-toolchain.toml` installs for `x86_64-unknown-none`, as an `rlib` (which
//! carries `component::start`, the entry `user/init/link.ld` places first) and
//! a `staticlib` (which carries the import, `core`, `alloc` and the allocator
//! shim rustc only writes for a final artefact), and the two are linked by the
//! same script and held to the same checks ([`crate::flat_checks`]). The panic
//! strategy is `abort` rather than `immediate-abort`, so the image carries the
//! formatting a panic message needs — a few kilobytes the measurement below is
//! an upper bound by, and nothing it depends on.
//!
//! *What would reverse this:* `-Zbuild-std` resolving the standard library
//! without consulting the replaced source, or the import's `vendor/` carrying
//! the standard library's dependencies too — the second of which would be
//! importing something because it was next to the first, RFC 0082's fourth
//! reversal condition, and is refused on that ground.

use std::path::PathBuf;
use std::process::Command;

use crate::{IMPORT_LINKERS, KERNEL_TARGET, llvm_tool, relative, root, target_dir};

/// The crate that links the import: `IMPORT_LINKERS`' one row's directory.
pub fn dir() -> PathBuf {
    let (manifest, ..) = IMPORT_LINKERS[0];
    root().join(manifest.strip_suffix("/Cargo.toml").unwrap_or(manifest))
}

/// Where its builds land: under the workspace's own target directory, which is
/// the container's volume rather than the bind mount, and outside every tree a
/// lint walks as source.
pub fn build_dir() -> PathBuf {
    target_dir().join("shaper")
}

/// The source replacement, as cargo's `--config` takes it: `crates-io` is the
/// import's `vendor/` directory, named by `IMPORT_LINKERS`' row.
fn vendored() -> [String; 4] {
    let (_, import, ..) = IMPORT_LINKERS[0];
    let vendor = root().join(import).join("vendor");
    [
        "--config".to_string(),
        "source.crates-io.replace-with=\"vendored-import\"".to_string(),
        "--config".to_string(),
        format!(
            "source.vendored-import.directory=\"{}\"",
            vendor.to_string_lossy().replace('\\', "/")
        ),
    ]
}

/// Run cargo in the shim's workspace: `--offline --locked`, the source
/// replacement, its own target directory, and `rustflags` if given.
///
/// # Errors
///
/// The command could not be run, or it failed.
pub fn cargo(args: &[&str], rustflags: Option<&str>) -> Result<(), String> {
    let build = build_dir();
    // The build flags go straight after the subcommand, so that a `--` in
    // `args` — clippy's, a test binary's — still has everything before it.
    let (subcommand, rest) = args.split_first().ok_or("no cargo subcommand")?;
    let mut command = Command::new("cargo");
    command
        .arg(subcommand)
        .args(["--offline", "--locked", "--target-dir"])
        .arg(&build)
        .args(vendored())
        .args(rest)
        .current_dir(dir());
    if let Some(flags) = rustflags {
        command.env("RUSTFLAGS", flags);
    }
    let status = command.status().map_err(|e| format!("could not run cargo: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("cargo {} failed in {}", args.join(" "), relative(&dir())))
    }
}

/// The shim's host tests, which are `E3-B03b0`'s run through the `shape`
/// protocol. Called by `cargo xtask test-host`, so they run on x86-64 and on the
/// arm runner.
///
/// # Errors
///
/// A test failed or did not build.
pub fn test_host() -> Result<(), String> {
    println!(
        "\nhost tests for {} on {} — outside the workspace, so run here",
        relative(&dir()),
        std::env::consts::ARCH
    );
    cargo(&["test"], None)
}

/// The shim compiled for AArch64: the library, with the image half the
/// architecture gate leaves in — the allocator — and without the entry, which
/// is x86-64's for the door's reason.
///
/// # Errors
///
/// It does not compile.
pub fn check_aarch64() -> Result<(), String> {
    println!("\ncompiling {} for {}", relative(&dir()), crate::AARCH64_TARGET);
    cargo(&["check", "--lib", "--target", crate::AARCH64_TARGET], None)
}

/// `rustfmt` and `clippy` over the shim, as `lint_style` holds the workspace:
/// `-D warnings`, the root's `rustfmt.toml`. The import is a replaced registry
/// source, so its own warnings are capped by cargo and are not this tree's.
///
/// # Errors
///
/// A formatting difference or a warning.
pub fn style() -> Result<(), String> {
    cargo_plain(&["fmt", "--", "--check"])?;
    cargo(&["clippy", "--all-targets", "--", "-D", "warnings"], None)
}

/// `cargo` in the shim's directory with none of [`cargo`]'s build flags, for
/// the one subcommand that takes none of them.
fn cargo_plain(args: &[&str]) -> Result<(), String> {
    let status = Command::new("cargo")
        .args(args)
        .current_dir(dir())
        .status()
        .map_err(|e| format!("could not run cargo: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("cargo {} failed in {}", args.join(" "), relative(&dir())))
    }
}

/// The most text any spawn shape maps, which is what the shaper would have to
/// fit: `kernel::process::TEXT_PAGES` pages, the largest bound
/// [`crate::IMAGE_MAX`] carries.
/// Unit: bytes.
pub fn spawn_text_max() -> u64 {
    crate::IMAGE_MAX.iter().map(|(_, bytes)| *bytes).max().unwrap_or(crate::INIT_MAX)
}

/// Build the shaper's component image and measure it.
///
/// Returns the flat binary and its length. The length is then held against
/// [`spawn_text_max`] by [`unspawnable`], which is what keeps `UNSPAWNABLE`'s
/// row true rather than remembered.
///
/// # Errors
///
/// The build or the link failed, or [`crate::flat_checks`] refused the image.
pub fn image() -> Result<(PathBuf, u64), String> {
    cargo(
        &[
            "rustc",
            "--lib",
            "--crate-type",
            "rlib",
            "--crate-type",
            "staticlib",
            "--target",
            KERNEL_TARGET,
            "--profile",
            "init",
        ],
        // `relocation-model=static` and the remap, for `flat_image_with`'s
        // reasons; not `immediate-abort`, for this module's.
        Some("-Zremap-cwd-prefix=. -C relocation-model=static"),
    )?;
    let out = build_dir().join(KERNEL_TARGET).join("init");
    let rlib = out.join("libf_shaper.rlib");
    let staticlib = out.join("libf_shaper.a");
    for archive in [&rlib, &staticlib] {
        if !archive.is_file() {
            return Err(format!("the shaper build produced no {}", relative(archive)));
        }
    }
    let elf = out.join("shaper.elf");
    let status = Command::new(llvm_tool("rust-lld")?)
        .args(["-flavor", "gnu", "-T", "user/init/link.ld", "--gc-sections", "-o"])
        .arg(&elf)
        .arg("--whole-archive")
        .arg(&rlib)
        .arg("--no-whole-archive")
        .arg(&staticlib)
        .current_dir(root())
        .status()
        .map_err(|e| format!("could not run rust-lld: {e}"))?;
    if !status.success() {
        return Err("linking the shaper against user/init/link.ld failed".into());
    }
    crate::flat_checks("f-shaper", "shaper", &out, &elf)
}

/// The shaper's image, and the check that it still cannot be spawned.
///
/// `UNSPAWNABLE`'s row says the shaper is declared and not built into a boot
/// because no spawn shape holds its image. This is what makes that sentence
/// able to become false loudly: an image that fits is red here, with the
/// instruction to spawn it.
///
/// # Errors
///
/// The image could not be built, or it fits.
pub fn unspawnable() -> Result<u64, String> {
    let (bin, bytes) = image()?;
    let most = spawn_text_max();
    if bytes <= most {
        return Err(format!(
            "the shaper's image is {bytes} bytes, and a spawn shape maps {most}: it fits.\n\n\
             `UNSPAWNABLE` in xtask/src/main.rs says the shaper is declared and not booted\n\
             because no spawn shape holds its image, and that is no longer true. Give it a\n\
             row in `IMAGE_MAX`, add it to `COMPONENTS`, delete its `UNSPAWNABLE` row, and\n\
             serve the ring `user/shaper/src/component.rs` does not yet serve. RFC 0141."
        ));
    }
    let tenths = ratio_tenths(bytes, most);
    println!(
        "\nshaper image  {bytes} bytes, {}.{} times the {most} a spawn shape maps — linked, \
         checked, and not spawnable (UNSPAWNABLE)  {}",
        tenths / 10,
        tenths % 10,
        relative(&bin)
    );
    Ok(bytes)
}

/// `bytes` over `most`, in tenths, as an integer. RFC 0004: no float here
/// either, and a ratio to one decimal place is all the line needs.
fn ratio_tenths(bytes: u64, most: u64) -> u64 {
    bytes.saturating_mul(10) / most.max(1)
}
