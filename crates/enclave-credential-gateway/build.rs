use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let routes = Path::new("../../profiles/routes");
    println!("cargo:rerun-if-changed={}", routes.display());

    let route_files = collect_yaml_files(routes);
    let generated = format!(
        "pub const ROUTE_FILES: &[(&str, &str)] = &{};\n",
        render_entries(&route_files)
    );

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR must be set"));
    fs::write(out_dir.join("embedded_profiles.rs"), generated)
        .expect("failed to write embedded profile manifest");
}

fn collect_yaml_files(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut files: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", dir.display()))
        .map(|entry| {
            entry
                .expect("failed to read profile directory entry")
                .path()
        })
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "yaml")
        })
        .map(|path| {
            let key = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .expect("route filenames must be valid UTF-8")
                .to_string();
            (
                key,
                path.canonicalize()
                    .expect("failed to canonicalize route profile"),
            )
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

fn render_entries(files: &[(String, PathBuf)]) -> String {
    let mut rendered = String::from("[\n");
    for (key, path) in files {
        rendered.push_str(&format!(
            "    ({:?}, include_str!({:?})),\n",
            key,
            path.display().to_string()
        ));
    }
    rendered.push(']');
    rendered
}
