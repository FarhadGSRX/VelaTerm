//! Windows: use the bundled ConPTY (`conpty.dll` + `OpenConsole.exe` from Microsoft's
//! Microsoft.Windows.Console.ConPTY package) instead of the console host shipped with the OS.
//!
//! The inbox conhost drops the mouse-mode requests (`?1000h` / `?1002h` / `?1006h`) and `?1049h` that TUIs
//! such as vim or Node CLIs write, so xterm.js never enters mouse reporting and clicks never reach the app
//! (issue #143). The bundled host forwards them and turns the terminal's SGR mouse reports back into input.
//!
//! portable-pty loads `conpty.dll` by bare name on the first `openpty` and falls back to kernel32. Windows
//! resolves a bare module name to an already-loaded module with the same name, so loading the bundled DLL
//! by full path first is enough. `conpty.dll` then starts `OpenConsole.exe` from its own directory.

use std::path::Path;
use windows::core::HSTRING;
use windows::Win32::System::LibraryLoader::LoadLibraryW;

/// Load `<dir>/conpty.dll` for the rest of the process. Called once by GUI setup before any PTY exists;
/// when the files are missing (dev builds) or fail to load, sessions keep using the system ConPTY.
pub fn preload(dir: &Path) {
    let dll = dir.join("conpty.dll");
    if !dll.is_file() || !dir.join("OpenConsole.exe").is_file() {
        crate::diagnostic_warn!("bundled ConPTY not found; terminal sessions use the system console host");
        return;
    }
    // The module is never freed: portable-pty keeps its function pointers for the process lifetime.
    if let Err(e) = unsafe { LoadLibraryW(&HSTRING::from(dll.as_path())) } {
        crate::diagnostic_warn!("failed to load bundled ConPTY (falling back to the system console host): {e}");
    }
}
