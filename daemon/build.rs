//! Embeds the owner app's static build (app/dist, `npm run build` in app/)
//! into the daemon, which serves it on this machine (src/web.rs). Without a
//! build there the daemon is built without it: `toolnet up` says so.
use std::{env, fmt::Write, fs, path::Path};

fn main() {
    let dist = env::var("TOOLNET_APP_DIST")
        .unwrap_or_else(|_| format!("{}/../app/dist", env::var("CARGO_MANIFEST_DIR").unwrap()));
    println!("cargo:rerun-if-env-changed=TOOLNET_APP_DIST");
    println!("cargo:rerun-if-changed={dist}");
    let mut files = Vec::new();
    let root = Path::new(&dist);
    if root.join("index.html").is_file() {
        walk(root, root, &mut files);
    }
    files.sort();
    let mut out = String::from("pub static FILES: &[(&str, &[u8])] = &[\n");
    for (rel, abs) in &files {
        writeln!(out, "    ({rel:?}, include_bytes!({abs:?})),").unwrap();
    }
    out.push_str("];\n");
    fs::write(Path::new(&env::var("OUT_DIR").unwrap()).join("owner_app.rs"), out).unwrap();
}

fn walk(root: &Path, dir: &Path, files: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            // download/: the APK the Pages deploy adds (tens of MB), not ours to serve
            if path != root.join("download") {
                walk(root, &path, files);
            }
        } else {
            let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            files.push((rel, fs::canonicalize(&path).unwrap().to_string_lossy().into_owned()));
        }
    }
}
