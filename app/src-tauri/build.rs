fn main() {
    // 把 git 提交号烧进二进制：托盘 tooltip 显示它，用来分辨「现在跑的是哪个版本」。
    // 单实例机制下旧实例会吞掉新启动的进程，肉眼分不清新旧。
    let hash = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=VK_BUILD={hash}");
    // 提交改的是分支引用文件（.git/refs/heads/xxx），只盯 HEAD 不会触发重编
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    if let Ok(head) = std::fs::read_to_string("../../.git/HEAD") {
        if let Some(r) = head.strip_prefix("ref: ") {
            println!("cargo:rerun-if-changed=../../.git/{}", r.trim());
        }
    }
    tauri_build::build()
}
