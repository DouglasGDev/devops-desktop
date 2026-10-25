//! Spawns de linha de comando sem janela de console.
//!
//! O executável de release é compilado com `windows_subsystem = "windows"`
//! (nenhum console próprio). Nesse modo, cada processo filho de subsistema
//! console (`ssh`, `sftp`, `git`, `ssh-add`, `cmd`) recebe do Windows uma
//! janela própria, que aparece some a cada operação. A flag `CREATE_NO_WINDOW`
//! declara o filho sem console visível; a saída continua vindo pelos pipes.
//!
//! As sessões de terminal interactive não passam por aqui: elas usam ConPTY
//! (`terminal.rs`), que já renderiza dentro da janela do aplicativo.

use std::process::Command;

/// `Command::new` para todo programa de linha de comando executado pelo app.
pub fn command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    hide_console(&mut cmd);
    cmd
}

fn hide_console(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_preserva_o_argv() {
        let mut cmd = command("git");
        cmd.arg("-C").arg(".").arg("status");
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert_eq!(args, vec!["-C", ".", "status"]);
    }
}
