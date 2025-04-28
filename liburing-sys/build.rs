use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");

    let liburing = pkg_config::Config::new()
        .probe("liburing")
        .expect("Didn't find liburing!");

    let header_path = liburing
        .include_paths
        .iter()
        .map(|path| {
            let mut header_path = path.clone();
            header_path.push("liburing.h");
            header_path
        })
        .find(|path| {
            path.try_exists()
                .expect("Unable to test include header existence")
        })
        .expect("Did not find include header")
        .into_os_string()
        .into_string()
        .expect("Found header path is not a valid UTF-8 string!");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("Could not find OUT_DIR in env"));

    let bindings = bindgen::Builder::default()
        .rust_edition(bindgen::RustEdition::Edition2021)
        .header(header_path)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .wrap_static_fns(true)
        .wrap_static_fns_path(out_dir.join("wrap_static_fns"))
        .generate()
        .expect("Unable to generate bindings");

    bindings
        .write_to_file(out_dir.join("bindings.rs"))
        .expect("Couldn't write bindings!");

    cc::Build::new()
        .file(out_dir.join("wrap_static_fns.c"))
        .flag_if_supported("-flto=thin")
        .compile("wrap_static_fns");
}
