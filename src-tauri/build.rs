fn main() {
    // Etterna's MinaCalc (MSD, see minacalc/README.md), compiled in
    cc::Build::new()
        .cpp(true)
        .std("c++20")
        .warnings(false)
        .define("STANDALONE_CALC", None)
        .define("NDEBUG", None)
        .opt_level(2)
        .include("minacalc")
        .include("minacalc/MinaCalc")
        .file("minacalc/API.cpp")
        .file("minacalc/MinaCalc/MinaCalc.cpp")
        .compile("minacalc");
    println!("cargo:rerun-if-changed=minacalc");
    tauri_build::build()
}
