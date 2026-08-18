fn main() {
    // sqlx::migrate! 在编译期嵌进二进制。只改 migrations/*.sql 时
    // cargo 默认不会重编 gw-model，活库就会一直停在旧版本。
    println!("cargo:rerun-if-changed=../../migrations");
}
