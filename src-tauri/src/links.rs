//! Links de pasta (junction no Windows, symlink no resto) usados para
//! compartilhar pastas entre perfis e para as skills globais.

use std::fs;
use std::path::{Path, PathBuf};

/// Remove um link sem seguir para o destino.
pub fn remove_link(p: &Path) -> std::io::Result<()> {
    fs::remove_dir(p).or_else(|_| fs::remove_file(p))
}

pub fn is_link(p: &Path) -> bool {
    fs::symlink_metadata(p).map(|m| m.file_type().is_symlink()).unwrap_or(false) || junction_target(p).is_some()
}

/// Destino do link (junction ou symlink), se `p` for um.
pub fn link_target(p: &Path) -> Option<PathBuf> {
    junction_target(p).or_else(|| fs::read_link(p).ok())
}

#[cfg(windows)]
fn junction_target(p: &Path) -> Option<PathBuf> {
    junction::get_target(p).ok()
}
#[cfg(not(windows))]
fn junction_target(_p: &Path) -> Option<PathBuf> {
    None
}

#[cfg(windows)]
pub fn make_dir_link(target: &Path, link: &Path) -> std::io::Result<()> {
    junction::create(target, link)
}
#[cfg(not(windows))]
pub fn make_dir_link(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// Compara caminhos ignorando caixa, barras e prefixo `\\?\`.
pub fn same_path(a: &Path, b: &Path) -> bool {
    let n = |p: &Path| {
        p.to_string_lossy()
            .trim_start_matches(r"\\?\")
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    };
    n(a) == n(b)
}
