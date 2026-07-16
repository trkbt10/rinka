//! Runs the shared Explorer consumer through the reusable WinUI 3 adapter.

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
fn main() {
    let application = rinka_explorer::view::application(rinka_explorer::view::Scene::Ready);
    if let Err(error) = rinka_winui::run(application) {
        eprintln!("{error}");
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {
    println!("The WinUI 3 Explorer consumer runs on Windows.");
}
