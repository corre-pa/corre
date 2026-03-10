fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    let output = std::process::Command::new("date").args(["-u", "+%Y-%m-%d"]).output().expect("failed to run `date`");
    let date = String::from_utf8(output.stdout).expect("invalid UTF-8 from `date`").trim().to_string();
    println!("cargo::rustc-env=BUILD_DATE={date}");
}
