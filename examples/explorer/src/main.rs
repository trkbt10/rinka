//! Rinka file-explorer consumer.

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
mod windows_contract;

use rinka_explorer::{surface, view};
use std::process::ExitCode;

fn requested_scene(arguments: &[String]) -> Result<view::Scene, String> {
    let Some(index) = arguments.iter().position(|argument| argument == "--scene") else {
        return Ok(view::Scene::Ready);
    };
    let value = arguments
        .get(index + 1)
        .ok_or_else(|| "--scene requires ready, empty, busy, or error".to_owned())?;
    view::Scene::parse(value)
        .ok_or_else(|| format!("unknown scene '{value}'; expected ready, empty, busy, or error"))
}

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments
        .iter()
        .any(|argument| argument == "--extract-surface")
    {
        print!("{}", surface::extract_all_scenes());
        return ExitCode::SUCCESS;
    }

    #[cfg(target_os = "windows")]
    let application = if arguments
        .iter()
        .any(|argument| argument == "--windows-contract-probe")
    {
        windows_contract::application()
    } else {
        let scene = match requested_scene(&arguments) {
            Ok(scene) => scene,
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
        };
        view::application(scene)
    };

    #[cfg(not(target_os = "windows"))]
    let application = {
        let scene = match requested_scene(&arguments) {
            Ok(scene) => scene,
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
        };
        view::application(scene)
    };

    #[cfg(target_os = "macos")]
    {
        rinka_macos::run(application);
        ExitCode::SUCCESS
    }
    #[cfg(target_os = "linux")]
    {
        let code = rinka_gtk::run(application);
        if code == 0 {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(u8::try_from(code).unwrap_or(1))
        }
    }
    #[cfg(target_os = "windows")]
    {
        let is_contract_probe = arguments
            .iter()
            .any(|argument| argument == "--windows-contract-probe");
        let result = if is_contract_probe {
            rinka_windows::run(application).map_err(|error| error.to_string())
        } else {
            rinka_winui::run(application).map_err(|error| error.to_string())
        };
        match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        eprintln!("Rinka explorer supports macOS, Linux, and Windows hosts");
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::requested_scene;
    use crate::view::Scene;

    #[test]
    fn scene_argument_rejects_missing_and_unknown_values() {
        assert_eq!(requested_scene(&[]).unwrap(), Scene::Ready);
        assert_eq!(
            requested_scene(&["--scene".to_owned(), "busy".to_owned()]).unwrap(),
            Scene::Busy
        );
        assert!(requested_scene(&["--scene".to_owned()]).is_err());
        assert!(requested_scene(&["--scene".to_owned(), "unknown".to_owned()]).is_err());
    }
}
