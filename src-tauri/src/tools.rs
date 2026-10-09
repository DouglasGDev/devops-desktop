use crate::models::ToolVersions;

fn run_capture(program: &str, args: &[&str]) -> Option<(String, String, Option<i32>)> {
    let output = crate::proc::command(program).args(args).output().ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    Some((stdout, stderr, output.status.code()))
}

pub fn detect() -> ToolVersions {
    let ssh = run_capture("ssh", &["-V"]).map(|(out, err, _)| {
        // `ssh -V` escreve a versão na stderr, ex.: "OpenSSH_10.2p1, ...".
        let first_line = if err.trim().is_empty() { out } else { err };
        first_line
            .lines()
            .next()
            .unwrap_or("")
            .split_whitespace()
            .next()
            .unwrap_or("desconhecida")
            .to_string()
    });

    let git = run_capture("git", &["--version"]).map(|(out, _, _)| {
        out.trim()
            .strip_prefix("git version ")
            .unwrap_or_else(|| out.trim())
            .to_string()
    });

    let (ssh_add, agent_running) = match run_capture("ssh-add", &["-l"]) {
        None => (None, None),
        Some((_, _, Some(0))) => (Some("disponível".into()), Some(true)),
        Some((_, _, Some(1))) => (Some("disponível".into()), Some(true)),
        Some((_, _, Some(2))) => (Some("disponível".into()), Some(false)),
        Some(_) => (Some("disponível".into()), None),
    };

    ToolVersions {
        ssh,
        ssh_add,
        git,
        agent_running,
    }
}
