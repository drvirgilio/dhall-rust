use std::env;
use std::ffi::OsString;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use walkdir::WalkDir;

fn dhall_files_in_dir<'a>(
    dir: &'a Path,
    take_a_suffix: bool,
) -> impl Iterator<Item = (String, String)> + 'a {
    WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(move |path| {
            let path = path.path();
            let path = path.strip_prefix(dir).unwrap();
            if path.extension() != Some(&OsString::from("dhall")) {
                return None;
            }
            let path = path.to_string_lossy();
            let path = &path[..path.len() - 6];
            let path = if take_a_suffix {
                if &path[path.len() - 1..] != "A" {
                    return None;
                } else {
                    path[..path.len() - 1].to_owned()
                }
            } else {
                path.to_owned()
            };
            let name = path.replace("/", "_").replace("-", "_");
            Some((name, path))
        })
}

fn make_test_module(
    w: &mut impl Write,
    mod_name: &str,
    dir: &Path,
    feature: &str,
    mut exclude: impl FnMut(&str) -> bool,
) -> std::io::Result<()> {
    writeln!(w, "mod {} {{", mod_name)?;
    for (name, path) in dhall_files_in_dir(&dir.join("success/"), true) {
        if exclude(&("success/".to_owned() + &path)) {
            continue;
        }
        writeln!(
            w,
            r#"make_spec_test!({}, Success, success_{}, "success/{}");"#,
            feature, name, path
        )?;
    }
    for (name, path) in dhall_files_in_dir(&dir.join("failure/"), false) {
        if exclude(&("failure/".to_owned() + &path)) {
            continue;
        }
        writeln!(
            w,
            r#"make_spec_test!({}, Failure, failure_{}, "failure/{}");"#,
            feature, name, path
        )?;
    }
    writeln!(w, "}}")?;
    Ok(())
}

fn main() -> std::io::Result<()> {
    println!("cargo:rerun-if-changed=../dhall-lang/.git");
    println!(
        "cargo:rerun-if-changed=../.git/modules/dhall-lang/refs/heads/master"
    );
    let out_dir = env::var("OUT_DIR").unwrap();
    let tests_dir = Path::new("../dhall-lang/tests/");

    let parser_tests_path = Path::new(&out_dir).join("spec_tests.rs");
    let mut file = File::create(parser_tests_path)?;

    make_test_module(
        &mut file,
        "parse",
        &tests_dir.join("parser/"),
        "Parser",
        |path| {
            // Too slow in debug mode
            path == "success/largeExpression"
        },
    )?;

    make_test_module(
        &mut file,
        "printer",
        &tests_dir.join("parser/"),
        "Printer",
        |path| {
            // Failure tests are only for the parser
            path.starts_with("failure/")
            // Too slow in debug mode
            || path == "success/largeExpression"
        },
    )?;

    make_test_module(
        &mut file,
        "binary_encoding",
        &tests_dir.join("parser/"),
        "BinaryEncoding",
        |path| {
            // Failure tests are only for the parser
            path.starts_with("failure/")
            // Too slow in debug mode
            || path == "success/largeExpression"
            // Too much of a pain to implement; shouldn't make a difference
            // since lets disappear on normalization.
            || path == "success/multilet"
            // See https://github.com/pyfisch/cbor/issues/109
            || path == "success/double"
        },
    )?;

    make_test_module(
        &mut file,
        "beta_normalize",
        &tests_dir.join("normalization/"),
        "Normalization",
        |path| {
            // We don't support bignums
            path == "success/simple/integerToDouble"
            // Too slow
            || path == "success/remoteSystems"
        },
    )?;

    make_test_module(
        &mut file,
        "alpha_normalize",
        &tests_dir.join("alpha-normalization/"),
        "AlphaNormalization",
        |_| false,
    )?;

    Ok(())
}
