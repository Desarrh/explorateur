// Icône et nom de l'application dans l'exe : visibles dans l'Explorateur Windows,
// la barre des tâches et le Gestionnaire des tâches (qui affiche « Explorateur »).
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico")
            .set("FileDescription", "Explorateur")
            .set("ProductName", "Explorateur")
            .set("InternalName", "explorateur")
            .set("OriginalFilename", "Explorateur.exe")
            .set("CompanyName", "Explorateur")
            .set("LegalCopyright", "");
        if let Err(e) = res.compile() {
            println!("cargo:warning=icône non intégrée : {e}");
        }
    }
}
