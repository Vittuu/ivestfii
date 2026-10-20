//! Backend-only setup; the key is read from stdin, never from arguments or files.
use ivestfii_market_data::MarketSettingsStore;
use std::{io, path::PathBuf};
#[cfg(windows)]
struct EchoGuard {
    handle: *mut std::ffi::c_void,
    mode: u32,
}
#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetStdHandle(kind: u32) -> *mut std::ffi::c_void;
    fn GetConsoleMode(handle: *mut std::ffi::c_void, mode: *mut u32) -> i32;
    fn SetConsoleMode(handle: *mut std::ffi::c_void, mode: u32) -> i32;
}
#[cfg(windows)]
impl EchoGuard {
    fn disable() -> Self {
        unsafe {
            let handle = GetStdHandle(-10_i32 as u32);
            let mut mode = 0;
            if GetConsoleMode(handle, &mut mode) != 0 {
                SetConsoleMode(handle, mode & !4);
            }
            Self { handle, mode }
        }
    }
}
#[cfg(windows)]
impl Drop for EchoGuard {
    fn drop(&mut self) {
        unsafe {
            if self.mode != 0 {
                SetConsoleMode(self.handle, self.mode);
            }
        }
    }
}
fn main() -> Result<(), String> {
    let directory = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("Informe o diretório de dados.")?;
    let mut secret = String::new();
    #[cfg(windows)]
    let _echo = EchoGuard::disable();
    io::stdin()
        .read_line(&mut secret)
        .map_err(|_| "Não foi possível receber a chave.")?;
    let result = MarketSettingsStore::new(directory)?.save_brapi_key(&secret);
    secret.clear();
    let settings = result?;
    println!(
        "{}",
        serde_json::json!({"configured":settings.brapi_key_configured,"source":settings.brapi_key_source})
    );
    Ok(())
}
