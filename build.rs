fn main() {
    #[cfg(target_os = "macos")]
    {
        cc::Build::new()
            .file("src/mac_face.m")
            .compile("mac_face");

        println!("cargo:rustc-link-lib=framework=Vision");
        println!("cargo:rustc-link-lib=framework=Foundation");
    }

    #[cfg(target_os = "windows")]
    {
        println!("cargo:rerun-if-changed=app.rc");
        println!("cargo:rerun-if-changed=app.ico");

        let out_dir = std::env::var("OUT_DIR").unwrap();
        let res_path = format!("{}/app.res", out_dir);

        let mut compiled = false;
        if let Ok(s) = std::process::Command::new("windres")
            .args(&["app.rc", "-O", "coff", "-o", &res_path])
            .status()
        {
            if s.success() {
                compiled = true;
            }
        }

        if !compiled {
            if let Ok(s) = std::process::Command::new("llvm-windres")
                .args(&["app.rc", "-O", "coff", "-o", &res_path])
                .status()
            {
                if s.success() {
                    compiled = true;
                }
            }
        }

        if compiled {
            println!("cargo:rustc-link-arg={}", res_path);
        }

        println!("cargo:rustc-link-arg=-Wl,--delayload=WebView2Loader.dll");
        println!("cargo:rustc-link-arg=-Wl,--delayload=libunwind.dll");
    }
}
