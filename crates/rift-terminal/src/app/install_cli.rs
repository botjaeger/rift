//! The palette's *Install 'rift' command* (`rift_core::cli_install`): a
//! `rift` link to this binary, so a shell or an editor can run `rift .`.

use super::*;
use rift_core::cli_install;

impl App {
    /// Install the `rift` command; the toast says where, and when that
    /// folder is not on the shell's `PATH`.
    pub(super) fn install_cli(&mut self) {
        let Some(dir) = cli_install::default_dir() else {
            self.toast_error("Could not install 'rift': no home folder".into());
            return;
        };
        let binary = match std::env::current_exe() {
            Ok(b) => b,
            Err(e) => {
                self.toast_error(format!("Could not install 'rift': {e}"));
                return;
            }
        };
        let done = match cli_install::install(&binary, &dir) {
            Ok(done) => done,
            Err(e) => {
                log::warn!("install cli: {e}");
                self.toast_error(format!("Could not install 'rift': {e}"));
                return;
            }
        };
        log::info!(
            "install cli: {} -> {}",
            done.link.display(),
            binary.display()
        );
        // the interactive shell's PATH (rc files extend it); Rift's own
        // when the shell has not reported one yet
        let path = self
            .shell_env
            .as_ref()
            .and_then(|e| e.path().map(|p| p.to_os_string()))
            .or_else(|| std::env::var_os("PATH"));
        let shown = editor::shorten_home(&dir.to_string_lossy());
        if cli_install::on_path(&dir, path.as_deref()) {
            self.toast_done(format!(
                "Installed 'rift' in {shown}: run rift . to open a folder in Rift"
            ));
        } else {
            self.toast(format!(
                "Installed 'rift' in {shown}, which is not on your PATH: add it to use 'rift'"
            ));
        }
    }
}
