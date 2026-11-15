// Windows host launcher for the Linux GTK4/VTE application under WSLg.
// This is not a native Windows port of edex-native.
use std::{env, io::{self, Write}, path::PathBuf, process::{self, Command}};

const ARCHIVE: &str = "eDEX-Remake-linux-x86_64-debian13.tar.gz";
const SCRIPT: &str = "wsl-launch.sh";
const DISTRO: &str = "Debian";

fn fail(message: &str) -> ! {
    eprintln!("\n{message}");
    eprintln!("Windows setup instructions: README-Windows.md");
    eprint!("\nPress Enter to close...");
    let _ = io::stderr().flush();
    let mut response = String::new();
    let _ = io::stdin().read_line(&mut response);
    process::exit(1);
}

fn main() {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if arguments.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("eDEX-Remake Windows launcher (WSL2 + WSLg, not a native Windows build)");
        println!("Usage: eDEX-Remake.exe [--setup | --help]");
        println!("Requires a WSL distribution named Debian running Debian 13.");
        println!("Install the distro using: wsl --install -d Debian");
        println!("Then run: eDEX-Remake.exe --setup");
        return;
    }
    if arguments.len() > 1 || arguments.first().is_some_and(|a| a != "--setup") {
        fail("Unsupported option. Use --setup to install or no options to launch.");
    }

    let executable = env::current_exe().unwrap_or_else(|error| {
        fail(&format!("Could not locate the Windows launcher: {error}"));
    });
    let folder: PathBuf = executable.parent().unwrap_or_else(|| {
        fail("Could not locate the extracted Windows package directory.");
    }).to_owned();

    if !folder.join(ARCHIVE).is_file() || !folder.join(SCRIPT).is_file() {
        fail("Missing Linux archive or wsl-launch.sh. Extract the *entire* Windows ZIP first.");
    }

    // WSLENV's /p flag converts the host's Windows path into a Linux mount path
    // without shell interpolation. It also handles spaces and non-ASCII paths.
    let old_wslenv = env::var("WSLENV").unwrap_or_default();
    let wslenv = if old_wslenv.is_empty() {
        String::from("EDEX_PACKAGE_ROOT/p")
    } else {
        format!("{old_wslenv}:EDEX_PACKAGE_ROOT/p")
    };

    let mut command = Command::new("wsl.exe");
    command
        .args([
            "--distribution", DISTRO,
            "--exec", "/bin/bash", "-c",
            r#"exec /bin/bash "$EDEX_PACKAGE_ROOT/wsl-launch.sh" "$@""#,
            "edex-windows",
        ])
        .args(&arguments)
        .env("EDEX_PACKAGE_ROOT", folder)
        .env("WSLENV", wslenv);

    let status = command.status().unwrap_or_else(|error| {
        fail(&format!(
            "Failed to run wsl.exe: {error}\n\
             Install or update WSL2 and its Debian distribution:\n\
             wsl --install -d Debian\nwsl --update"
        ));
    });

    if !status.success() {
        fail(&format!(
            "The WSL command exited with status: {status}.\n\
             If this is your first run, use: eDEX-Remake.exe --setup"
        ));
    }
}
