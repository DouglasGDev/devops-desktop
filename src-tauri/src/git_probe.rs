use std::{
    path::{Path, PathBuf},
};

/// Detecta se a pasta contém um repositório Git e coleta branch/remote com o
/// Git nativo (argumentos separados, sem shell). Somente leitura.
pub struct GitProbe {
    pub is_git_repo: bool,
    pub branch: Option<String>,
    pub remote: Option<String>,
}

pub fn probe(path: &Path) -> GitProbe {
    let is_git_repo = path.join(".git").exists();
    if !is_git_repo {
        return GitProbe { is_git_repo: false, branch: None, remote: None };
    }
    GitProbe {
        is_git_repo: true,
        branch: git_line(path, &["rev-parse", "--abbrev-ref", "HEAD"]),
        remote: git_line(path, &["remote", "get-url", "origin"]),
    }
}

fn git_line(path: &Path, args: &[&str]) -> Option<String> {
    let output = crate::proc::command("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

pub fn canonical_dir(raw: &str) -> Result<PathBuf, String> {
    let path = crate::ssh_config::expand_tilde(raw);
    let meta = std::fs::metadata(&path)
        .map_err(|e| format!("a pasta {} não pôde ser acessada: {e}", path.display()))?;
    if !meta.is_dir() {
        return Err(format!("o caminho {} não é uma pasta", path.display()));
    }
    let canonico = path
        .canonicalize()
        .map_err(|e| format!("falha ao normalizar {}: {e}", path.display()))?;
    Ok(simple(&canonico))
}

/// No Windows, `canonicalize()` devolve caminhos verbatim (`\\?\C:\...` e
/// `\\?\UNC\servidor\share`). Eles vão para o banco e para a tela, e nenhum
/// outro programa os reconhece, então o prefixo é desfeito aqui.
pub fn simple(path: &Path) -> PathBuf {
    let texto = path.to_string_lossy();
    if let Some(resto) = texto.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{resto}"));
    }
    if let Some(resto) = texto.strip_prefix(r"\\?\") {
        return PathBuf::from(resto);
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desaz_o_caminho_verbatim_do_windows() {
        assert_eq!(simple(Path::new(r"\\?\C:\repos\app")), PathBuf::from(r"C:\repos\app"));
        assert_eq!(
            simple(Path::new(r"\\?\UNC\servidor\share\app")),
            PathBuf::from(r"\\servidor\share\app")
        );
        assert_eq!(simple(Path::new("/home/deploy/app")), PathBuf::from("/home/deploy/app"));
    }
}
