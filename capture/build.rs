fn main() {
    let target = std::env::var("TARGET").unwrap();

    if target.contains("espidf") {
        embuild::espidf::sysenv::output();
    }
}
